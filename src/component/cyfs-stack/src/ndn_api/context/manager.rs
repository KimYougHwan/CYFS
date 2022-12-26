use super::context::*;
use cyfs_base::*;
use cyfs_bdt::*;
use cyfs_core::*;
use cyfs_lib::*;

use lru_time_cache::LruCache;
use std::sync::{Arc, Mutex};

pub(crate) struct ContextItem {
    pub object_id: ObjectId,
    pub object: TransContext,
    pub source_list: Vec<DownloadSource<DeviceDesc>>,
}

#[derive(Clone)]
pub(crate) struct ContextManager {
    noc: NamedObjectCacheRef,
    device_manager: Arc<Box<dyn DeviceCache>>,
    list: Arc<Mutex<LruCache<ObjectId, Arc<ContextItem>>>>,
}

impl ContextManager {
    pub fn new(noc: NamedObjectCacheRef, device_manager: Box<dyn DeviceCache>) -> Self {
        Self {
            noc,
            device_manager: Arc::new(device_manager),
            list: Arc::new(Mutex::new(LruCache::with_expiry_duration_and_capacity(
                std::time::Duration::from_secs(60 * 10),
                128,
            ))),
        }
    }

    fn decode_context_id_from_string(source_dec: &ObjectId, s: &str) -> TransContextRef {
        if OBJECT_ID_BASE58_RANGE.contains(&s.len()) {
            match ObjectId::from_base58(s) {
                Ok(ret) => TransContextRef::Object(ret),
                Err(_) => TransContextRef::Path((s.to_owned(), source_dec.to_owned())),
            }
        } else if OBJECT_ID_BASE36_RANGE.contains(&s.len()) {
            match ObjectId::from_base36(s) {
                Ok(ret) => TransContextRef::Object(ret),
                Err(_) => TransContextRef::Path((s.to_owned(), source_dec.to_owned())),
            }
        } else {
            TransContextRef::Path((s.to_owned(), source_dec.to_owned()))
        }
    }

    pub async fn create_download_context_from_trans_context(
        &self,
        source_dec: &ObjectId,
        referer: impl Into<String>,
        trans_context: &str,
    ) -> BuckyResult<TransContextHolder> {
        let ref_id = Self::decode_context_id_from_string(source_dec, trans_context);

        let holder = TransContextHolder::new_context(self.clone(), ref_id, referer);
        holder.init().await?;

        Ok(holder)
    }

    pub async fn create_download_context_from_target(
        &self,
        referer: impl Into<String>,
        target: DeviceId,
    ) -> BuckyResult<TransContextHolder> {
        let ret = self.device_manager.get(&target).await;
        if ret.is_none() {
            let msg = format!(
                "load trans context with target but not found! target={}",
                target
            );
            error!("{}", msg);
            return Err(BuckyError::new(BuckyErrorCode::NotFound, msg));
        }

        let device = ret.unwrap();
        let holder =
            TransContextHolder::new_target(target, device.into_desc(), referer);

        Ok(holder)
    }

    pub fn create_download_context_from_target_sync(
        referer: impl Into<String>,
        target: DeviceId,
        target_desc: DeviceDesc,
    ) -> TransContextHolder {
        let holder =
            TransContextHolder::new_target(target, target_desc, referer);

        holder
    }

    async fn new_item(&self, object_id: ObjectId, object: TransContext) -> ContextItem {
        let mut source_list = Vec::with_capacity(object.device_list().len());
        for item in object.device_list() {
            let ret = self.device_manager.get(&item.target).await;
            if ret.is_none() {
                warn!(
                    "load trans context target but not found! id={}, context_path={}, target={}",
                    object_id,
                    object.context_path(),
                    item.target
                );
                continue;
            }

            let device = ret.unwrap();
            let source = DownloadSource {
                target: device.into_desc(),
                codec_desc: item.chunk_codec_desc.clone(),
            };
            source_list.push(source);
        }

        ContextItem {
            object_id,
            object,
            source_list,
        }
    }

    /* path likes /a/b/c */
    pub async fn search_context(&self, dec_id: &ObjectId, path: &str) -> Option<Arc<ContextItem>> {
        assert!(TransContextPath::verify(path));

        let mut current_path = path;
        loop {
            let id = TransContext::gen_context_id(dec_id.to_owned(), current_path);
            if let Some(item) = self.get_context(&id).await {
                debug!(
                    "search trans context by path! path={}, matched={}, context={}",
                    path, current_path, id
                );
                break Some(item);
            }

            if current_path == "/" {
                error!("search trans context by path but not found! path={}", path);
                break None;
            }

            let ret = path.rsplit_once('/').unwrap();
            current_path = match ret.0 {
                "" => "/",
                _ => ret.0,
            };
        }
    }

    pub async fn get_context(&self, id: &ObjectId) -> Option<Arc<ContextItem>> {
        let (ret, gc_list) = {
            let mut cache = self.list.lock().unwrap();
            let (ret, gc_list) = cache.notify_get(id);
            (ret.cloned(), gc_list)
        };

        if let Some(item) = ret {
            return Some(item.clone());
        }

        drop(gc_list);

        // then load from noc
        if let Ok(Some(object)) = self.load_context_from_noc(id).await {
            let item = self.new_item(id.to_owned(), object).await;
            let item = Arc::new(item);
            self.update_context(item.clone());
            Some(item)
        } else {
            None
        }
    }

    pub async fn get_context_by_path(&self, dec_id: &ObjectId, context_path: &str) -> Option<Arc<ContextItem>> {
        let object_id = TransContext::gen_context_id(dec_id.to_owned(), context_path);
        self.get_context(&object_id).await
    }

    async fn load_context_from_noc(&self, id: &ObjectId) -> BuckyResult<Option<TransContext>> {
        let noc_req = NamedObjectCacheGetObjectRequest {
            object_id: id.to_owned(),
            source: RequestSourceInfo::new_local_system(),
            last_access_rpath: None,
        };

        match self.noc.get_object(&noc_req).await {
            Ok(Some(resp)) => {
                let object = TransContext::clone_from_slice(resp.object.object_raw.as_slice())
                    .map_err(|e| {
                        let msg = format!(
                            "load trans context from noc but invalid object! id={}, {}",
                            id, e
                        );
                        error!("{}", msg);
                        BuckyError::new(BuckyErrorCode::InvalidData, msg)
                    })?;

                Ok(Some(object))
            }
            Ok(None) => {
                debug!(
                    "load trans context object from noc but not found: id={}",
                    id
                );
                Ok(None)
            }
            Err(e) => {
                warn!(
                    "load trans context object from noc failed! id={}, {}",
                    id, e
                );
                Err(e)
            }
        }
    }

    pub async fn put_context(
        &self,
        source: RequestSourceInfo,
        object: NONObjectInfo,
    ) -> BuckyResult<()> {
        let trans_context = TransContext::clone_from_slice(&object.object_raw).map_err(|e| {
            let msg = format!(
                "invalid trans context object! id={}, {}",
                object.object_id, e
            );
            error!("{}", msg);
            BuckyError::new(BuckyErrorCode::InvalidData, msg)
        })?;

        // please make sure the id is matched before call this method!!
        // let id = trans_context.desc().calculate_id();
        let id = object.object_id.clone();

        let req = NamedObjectCachePutObjectRequest {
            source,
            object,
            storage_category: NamedObjectStorageCategory::Cache,
            context: None,
            last_access_rpath: None,
            access_string: None,
        };

        self.noc.put_object(&req).await.map_err(|e| {
            error!("save trans context to noc failed! id={}, {}", id, e);
            e
        })?;

        let item = self.new_item(id, trans_context).await;
        let item = Arc::new(item);
        self.update_context(item);

        Ok(())
    }

    fn update_context(&self, trans_context: Arc<ContextItem>) {
        let ret = {
            let mut cache = self.list.lock().unwrap();
            cache.notify_insert(trans_context.object_id.clone(), trans_context)
        };

        match ret.0 {
            Some(v) => {
                info!("replace old trans context! id={}", v.object_id);
            }
            None => {}
        }
    }
}

