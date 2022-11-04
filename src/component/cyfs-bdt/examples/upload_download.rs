use async_std::{future, io::prelude::*, task};
use cyfs_base::*;
use cyfs_util::cache::{NamedDataCache, TrackerCache};
use cyfs_bdt::{
    download::*,
    SingleDownloadContext, 
    ChunkReader,
    MemChunkStore,
    MemTracker,
    // StackConfig,
    Stack,
    StackGuard,
    StackOpenParams, 
    NdnEventHandler,  
    DefaultNdnEventHandler, 
    ndn::{channel::{Channel, DownloadSession, protocol::v0::*}}, 
};
use std::{sync::Arc, time::Duration};
mod utils;

async fn watch_recv_chunk(stack: StackGuard, chunkid: ChunkId) -> BuckyResult<ChunkId> {
    loop {
        let ret = stack.ndn().chunk_manager().store().get(&chunkid).await;
        if let Ok(mut reader) = ret {
            let mut content = vec![0u8; chunkid.len()];
            let _ = reader.read(content.as_mut_slice()).await?;
            let recv_id = ChunkId::calculate(content.as_slice()).await?;
            return Ok(recv_id);
        } else {
            task::sleep(Duration::from_millis(500)).await;
        }
    }
}

#[async_std::main]
async fn main() {
    cyfs_util::process::check_cmd_and_exec("bdt-example-upload-download");
    cyfs_debug::CyfsLoggerBuilder::new_app("bdt-example-upload-download")
        .level("trace")
        .console("debug")
        .build()
        .unwrap()
        .start();

    cyfs_debug::PanicBuilder::new("bdt-example-upload-download", "bdt-example-upload-download")
        .exit_on_panic(true)
        .build()
        .start();
            
    let (down_dev, down_secret) = utils::create_device(
        "5aSixgLuJjfrNKn9D4z66TEM6oxL3uNmWCWHk52cJDKR",
        &["W4udp127.0.0.1:10010"],
    )
    .unwrap();

    let (cache_dev, cache_secret) = utils::create_device(
        "5aSixgLuJjfrNKn9D4z66TEM6oxL3uNmWCWHk52cJDKR",
        &["W4udp127.0.0.1:10011"],
    )
    .unwrap();

    let (src_dev, src_secret) = utils::create_device(
        "5aSixgLuJjfrNKn9D4z66TEM6oxL3uNmWCWHk52cJDKR",
        &["W4udp127.0.0.1:10012"],
    )
    .unwrap();

    let (down_stack, down_store) = {
        let mut params = StackOpenParams::new("bdt-example-upload-from-downloader-down");
        let tracker = MemTracker::new();
        let store = MemChunkStore::new(NamedDataCache::clone(&tracker).as_ref());
        params.chunk_store = Some(store.clone_as_reader());
        params.ndc = Some(NamedDataCache::clone(&tracker));
        params.tracker = Some(TrackerCache::clone(&tracker));
        params.known_device = Some(vec![cache_dev.clone(), src_dev.clone()]);
        (
            Stack::open(down_dev.clone(), down_secret, params)
                .await
                .unwrap(),
            store,
        )
    };

    let (cache_stack, _cache_store) = {
        let mut params = StackOpenParams::new("bdt-example-upload-from-downloader-cache");
        let tracker = MemTracker::new();
        let store = MemChunkStore::new(NamedDataCache::clone(&tracker).as_ref());
        params.chunk_store = Some(store.clone_as_reader());
        params.ndc = Some(NamedDataCache::clone(&tracker));
        params.tracker = Some(TrackerCache::clone(&tracker));
        params.known_device = Some(vec![src_dev.clone()]);

        struct DownloadFromSource {
            src_dev: Device, 
            store: MemChunkStore,
            default: DefaultNdnEventHandler
        }

        #[async_trait::async_trait]
        impl NdnEventHandler for DownloadFromSource {
            async fn on_newly_interest(
                &self, 
                stack: &Stack, 
                interest: &Interest, 
                from: &Channel
            ) -> BuckyResult<()> {
                let task = download_chunk(
                    stack,
                    interest.chunk.clone(), 
                    None, 
                    Some(SingleDownloadContext::desc_streams(None, vec![self.src_dev.desc().clone()])),
                ).await.unwrap();
                {
                    let store = self.store.clone();
                    let chunk = interest.chunk.clone();
                    let reader = task.reader();
                    task::spawn(async move {
                        store.write_chunk(&chunk, reader).await.unwrap();
                    });
                }
                self.default.on_newly_interest(stack, interest, from).await
            }
        
            fn on_unknown_piece_data(
                &self, 
                _stack: &Stack, 
                _piece: &PieceData, 
                _from: &Channel
            ) -> BuckyResult<DownloadSession> {
                unimplemented!()
            }
        }
        params.ndn_event = Some(Box::new(DownloadFromSource {
            default: DefaultNdnEventHandler::new(), 
            src_dev: src_dev.clone(), 
            store: store.clone()
        }));
        (
            Stack::open(cache_dev, cache_secret, params).await.unwrap(),
            store,
        )
    };

    let (_src_stack, src_store) = {
        let mut params = StackOpenParams::new("bdt-example-upload-from-downloader-src");
        params.config.interface.udp.sim_loss_rate = 10;
        let tracker = MemTracker::new();
        let store = MemChunkStore::new(NamedDataCache::clone(&tracker).as_ref());
        params.chunk_store = Some(store.clone_as_reader());
        params.ndc = Some(NamedDataCache::clone(&tracker));
        params.tracker = Some(TrackerCache::clone(&tracker));
        (
            Stack::open(src_dev, src_secret, params).await.unwrap(),
            store,
        )
    };

    let (chunk_len, chunk_data) = utils::random_mem(1024, 1024);
    let chunk_hash = hash_data(&chunk_data[..]);
    let chunkid = ChunkId::new(&chunk_hash, chunk_len as u32);

    let to_put = Arc::new(chunk_data);
    let _ = src_store
        .add(chunkid.clone(), to_put.clone())
        .await
        .unwrap();

    let task = download_chunk(
        &*down_stack,
        chunkid.clone(), 
        None, 
        Some(SingleDownloadContext::desc_streams(None, vec![cache_stack.local_const().clone(),])),
    ).await.unwrap();
    down_store.write_chunk(&chunkid, task.reader()).await.unwrap();
    let recv = future::timeout(
        Duration::from_secs(50),
        watch_recv_chunk(down_stack.clone(), chunkid.clone()),
    )
    .await
    .unwrap();
    let recv_chunk_id = recv.unwrap();
    assert_eq!(recv_chunk_id, chunkid);
}