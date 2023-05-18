use std::{
    io::{Read}, 
    path::Path, 
    str::FromStr, 
    time::Duration, 
    net::Shutdown,
};
use rand::Rng;
use clap::{
    App, 
    SubCommand, 
    Arg
};
use async_std::{
    future,
};

use cyfs_base::*;
use cyfs_bdt::*;
use cyfs_stack_loader::*;
use cyfs_lib::*;
use cyfs_noc::*;
use cyfs_util::*;
use cyfs_stack::{
    name::{NameResolver, NameResult}, 
    meta::{RawMetaCache, MetaCacheRef}
};
use cyfs_meta_lib::*;
use cyfs_base_meta::*;
use log::*;

mod sn_bench;
use crate::sn_bench::*;

fn load_dev_by_path(path: &str) -> Option<Device> {
    let desc_path = Path::new(path);
    if desc_path.exists() {
        let mut file = std::fs::File::open(desc_path).unwrap();
        let mut buf = Vec::<u8>::new();
        let _ = file.read_to_end(&mut buf).unwrap();
        let (device, _) = Device::raw_decode(buf.as_slice()).unwrap();
    
        Some(device)
    } else {
        None
    }
}

fn load_dev_vec(path: &str) -> Option<Vec<Device>> {
    let mut dev_vec = Vec::new();
    match load_dev_by_path(path) {
        Some(dev) => {
            dev_vec.push(dev);

            Some(dev_vec)
        },
        _ => None
    }
}

async fn init_raw_noc(
        isolate: &str,
        known_objects: CyfsStackKnownObjects,
) -> BuckyResult<NamedObjectCacheRef> {
        let isolate = isolate.to_owned();

        // 这里切换线程同步初始化，否则debug下可能会导致主线程调用栈过深
        let noc = async_std::task::spawn(async move {
            match NamedObjectCacheManager::create(&isolate).await {
                Ok(noc) => {
                    info!("init named object cache manager success!");
                    Ok(noc)
                }
                Err(e) => {
                    error!("init named object cache manager failed: {}", e);
                    Err(e)
                }
            }
        })
        .await?;

        // 这里异步的初始化一些已知对象
        let noc2 = noc.clone();
        let task = async_std::task::spawn(async move {
            // 初始化known_objects
            for object in known_objects.list.into_iter() {
                let req = NamedObjectCachePutObjectRequest {
                    source: RequestSourceInfo::new_local_system(),
                    object,
                    storage_category: NamedObjectStorageCategory::Storage,
                    context: None,
                    last_access_rpath: None,
                    access_string: None,
                };
                let _ = noc2.put_object(&req).await;
            }
        });

        if known_objects.mode == CyfsStackKnownObjectsInitMode::Sync {
            task.await;
        }

        Ok(noc)
}

async fn load_sn_from_meta(meta_cache: MetaCacheRef, id: &ObjectId) -> BuckyResult<Vec<(DeviceId, Device)>> {
    let ret = meta_cache.get_object(id).await.map_err(|e| {
        error!("load sn from meta failed! id={}, {}", id, e);
        e
    })?;

    if ret.is_none() {
        let msg = format!("load sn from meta but not found! id={}", id);
        return Err(BuckyError::new(BuckyErrorCode::NotFound, msg));
    }

    let object = ret.unwrap();
    let id = object.object.object_id();

    SNDirParser::parse(Some(&id), &object.object_raw)
}

fn get_meta_miner_target(channel: &str) -> MetaMinerTarget {
    match channel {
        "nightly" => MetaMinerTarget::Dev,
        "formal" => MetaMinerTarget::Formal,
        _ => MetaMinerTarget::Test //default is beta
    }
}

async fn get_sn_from_meta(channel: &str) -> BuckyResult<Option<Vec<Device>>> {
    let noc = init_raw_noc("", CyfsStackKnownObjects {
        list: KNOWN_OBJECTS_MANAGER.clone_objects(),
        mode: KNOWN_OBJECTS_MANAGER.get_mode(),
    }).await?;
    let raw_meta_cache = RawMetaCache::new(get_meta_miner_target(channel), noc.clone());
    let name_resolver = NameResolver::new(raw_meta_cache.clone(), noc.clone());
    name_resolver.start().await?;
    name_resolver.reset_name(CYFS_SN_NAME);
    let ret = name_resolver.resolve(CYFS_SN_NAME).await;
    match ret {
        Ok(NameResult::ObjectLink(id)) => {
            let sns = load_sn_from_meta(raw_meta_cache, &id).await?;
            let mut sn_list = vec![];
            for (_, sn) in sns {
                sn_list.push(sn);
            }
            if sn_list.len() > 0 {
                return Ok(Some(sn_list))
            }
        }
        Ok(NameResult::IPLink(value)) => {
            println!(
                "get sn id from meta but not support! {} -> {}",
                CYFS_SN_NAME, value
            );
        }
        Err(e) if e.code() == BuckyErrorCode::NotFound => {
            println!("get sn id from meta but not found! {}", CYFS_SN_NAME);
        }
        Err(e) => {
            println!("get sn from meta failed! name={}, {}", CYFS_SN_NAME, e);
        }
    }

    Ok(None)
}

async fn get_device_from_meta(device_id: &str, channel: &str) -> BuckyResult<Option<Device>> {
    let id = ObjectId::from_str(device_id)?;
    let meta_client = MetaClient::new_target(get_meta_miner_target(channel));
    match meta_client.get_desc(&id).await {
        Ok(data) => {
            if let SavedMetaObject::Device(device) = data {
                Ok(Some(device))
            } else {
                Err(BuckyError::from(BuckyErrorCode::NotMatch))
            }
        }
        Err(e) => {
            Err(e)
        }
    }
}

async fn load_sn(channel: &str, sns: Vec<&str>) -> Option<Vec<Device>> {
    let mut dev_vec = Vec::new();

    if sns.len() == 0 {
        get_sn_from_meta(channel).await.unwrap()
    } else {
        for sn in sns {
            let dev = load_dev_by_path(sn).unwrap();
            dev_vec.push(dev);
        }

        Some(dev_vec)
    }
}

fn loger_init(log_level: &str, name: &str) {
    if log_level != "none" {
        cyfs_debug::CyfsLoggerBuilder::new_app(name)
            .level(log_level)
            .console(log_level)
            .build()
            .unwrap()
            .start();

        cyfs_debug::PanicBuilder::new(name, name)
        .exit_on_panic(true)
        .build()
        .start();
    }
}

pub fn command_line() -> clap::App<'static, 'static> {
    App::new("bdt-tool")
        .about("bdt tool")
        .arg(Arg::with_name("channel").long("channel").default_value("beta").help("channel env: beta/nightly, default is beta"))
        .arg(Arg::with_name("ep").long("ep").multiple(true).default_value("").help("local endpoint"))
        .arg(Arg::with_name("udp_sn_only").long("udp_sn_only").takes_value(false).default_value("0").help("udp sn only"))
        .arg(Arg::with_name("log_level").long("log_level").default_value("none").help("log level: none/info/debug/warn/error"))
        .arg(Arg::with_name("device_cache").long("device_cache").default_value("").help("device cache"))
        .arg(Arg::with_name("sn").long("sn").multiple(true).default_value("").help("sn desc file"))
        .arg(Arg::with_name("cmd").long("cmd").takes_value(false).help("sn desc file"))
        .subcommand(SubCommand::with_name("ping")
            .arg(Arg::with_name("remote").required(true))
            .arg(Arg::with_name("count").required(true))
            .arg(Arg::with_name("timeout").required(true))
        )
        .subcommand(SubCommand::with_name("nc")
            .arg(Arg::with_name("remote").required(true))
            .arg(Arg::with_name("port").required(true))
        )
        .subcommand(SubCommand::with_name("sn_bench_ping")
            .arg(Arg::with_name("remote").required(true))
            .arg(Arg::with_name("port").required(true))
        )
        .subcommand(SubCommand::with_name("sn_bench_call")
            .arg(Arg::with_name("remote").required(true))
            .arg(Arg::with_name("port").required(true))
        )
}

async fn remote_device(
    stack: &Stack, 
    str: &str,
    channel: &str) -> BuckyResult<Device> {
    let device = if let Ok(_) = DeviceId::from_str(str) {
        get_device_from_meta(str, channel).await.unwrap().unwrap()
    } else {
        let path = Path::new(str);
        if !path.exists() {
            return Err(BuckyError::new(BuckyErrorCode::NotFound, "device desc file not found"))
        } else {
            let mut buf = vec![];
            let (device, _) = Device::decode_from_file(&path, &mut buf)?;

            device
        }
    };

    let device_id = device.desc().device_id();
    if stack.device_cache().get(&device_id).await.is_none() {
        stack.device_cache().add(&device_id, &device);
    }

    Ok(device)
}

#[async_std::main]
async fn main() {
    //
    let cmd_line = std::env::args().collect::<Vec<String>>().join(" ");
    let matches = command_line().get_matches();

    let channel = matches.value_of("channel").unwrap();
    let log_level = matches.value_of("log_level").unwrap();
    let udp_sn_only = u16::from_str(matches.value_of("udp_sn_only").unwrap()).unwrap();

    let cmd_params = command_line().get_matches_from_safe(cmd_line.split(" "))
        .map_err(|err| err.message).unwrap();
    let subcommand = cmd_params.subcommand_name().ok_or_else(|| "no subcommand\r\n".to_string()).unwrap();

    let mut endpoints = vec![];
    for ep in matches.values_of("ep").unwrap() {
        if ep.len() > 0 {
            if let Ok(ep) = Endpoint::from_str(ep) {
                endpoints.push(ep);
            } else {
                println!("invalid endpoint {}", ep);
                return;
            }
        }
    }

    let mut sns = vec![];
    for sn in matches.values_of("sn").unwrap() {
        if sn.len() != 0 {
            sns.push(sn);
        }
    }
    let sns = load_sn(channel, sns).await;

    println!("Channel={}", channel);
    if let Some(sns) = sns.clone() {
        println!("SN Number={}", sns.len());
        for sn in sns {
            println!("  {}", sn.desc().device_id());
        }
    } else {
        println!("SN Number=0");
    }
    println!("");

    match subcommand {
        "sn_bench_ping" => {
            loger_init(log_level, "sn_bench_ping");

            let subcommand = cmd_params.subcommand_matches("sn_bench_ping").unwrap();
            let device_load = subcommand.value_of("load").unwrap_or("");
            let device_num = u64::from_str(subcommand.value_of("device").unwrap_or("1000")).unwrap();
            let interval_ms = u64::from_str(subcommand.value_of("interval").unwrap_or("1000")).unwrap();
            let timeout_sec = u64::from_str(subcommand.value_of("timeout").unwrap_or("3")).unwrap();
            let bench_time = u64::from_str(subcommand.value_of("time").unwrap_or("60")).unwrap();
            let exception = bool::from_str(subcommand.value_of("exception").unwrap_or("false")).unwrap();

            let result = sn_bench_ping(
                device_num, device_load, 
                sns, endpoints, bench_time,
                interval_ms, 
                timeout_sec,
                exception).await.unwrap();

            result.show();

            return;
        },
        "sn_bench_call" => {
            loger_init(log_level, "sn_bench_call");

            let subcommand = cmd_params.subcommand_matches("sn_bench_call").unwrap();
            let device_load = subcommand.value_of("load").unwrap_or("");
            let device_num = u64::from_str(subcommand.value_of("device").unwrap_or("1000")).unwrap();
            let interval_ms = u64::from_str(subcommand.value_of("interval").unwrap_or("1000")).unwrap();
            let timeout_sec = u64::from_str(subcommand.value_of("timeout").unwrap_or("3")).unwrap();
            let bench_time = u64::from_str(subcommand.value_of("time").unwrap_or("60")).unwrap();
            let exception = bool::from_str(subcommand.value_of("exception").unwrap_or("false")).unwrap();

            let result = sn_bench_call(
                device_num, device_load, 
                sns, endpoints, bench_time,
                interval_ms, 
                timeout_sec,
                exception).await.unwrap();

            result.show();

            return;
        },
        _ => {}
    }

    if endpoints.len() == 0 {
        let port = rand::thread_rng().gen_range(50000, 65000) as u16;
        for ip in cyfs_util::get_all_ips().unwrap() {
            if ip.is_ipv4() {
                endpoints.push(Endpoint::from((Protocol::Tcp, ip, port)));
                endpoints.push(Endpoint::from((Protocol::Udp, ip, port)));
            }
        }
    }

    //load device
    /*
    let desc_path = Path::new("deamon.desc");
    let sec_path = Path::new("deamon.sec");
    if !sec_path.exists() {
        println!("deamon.desc not exists, generate new one");

        let (device, private_key) = create_device(sns.clone(), endpoints.clone());

        if let Err(err) = device.encode_to_file(&desc_path, false) {
            println!("generate deamon.desc failed for {}",  err);
            return;
        } 
        if let Err(err) = private_key.encode_to_file(&sec_path, false) {
            println!("generate deamon.sec failed for {}",  err);
            return;
        }
    }
    if !desc_path.exists() {
        println!("{:?} not exists", desc_path);
        return;
    }
    let device = {
        let mut buf = vec![];
        Device::decode_from_file(&desc_path, &mut buf).map(|(d, _)| d)
    }; 
    if let Err(err) = device {
        println!("load {:?} failed for {}", desc_path, err);
        return;
    } 
    
    let private_key = {
        let mut buf = vec![];
        PrivateKey::decode_from_file(&sec_path, &mut buf).map(|(k, _)| k)
    }; 
    if let Err(err) = private_key {
        println!("load {:?} failed for {}", sec_path, err);
        return;
    } 
    let private_key = private_key.unwrap();

    let mut device = device.unwrap();
    */

    let (mut device, private_key) = create_device(sns.clone(), endpoints.clone());

    info!("device={:?}", device);

    let deamon_id = device.desc().device_id();
    let deamon_name = format!("bdt-tool-{}", deamon_id);
    loger_init(log_level, deamon_name.as_str());

    let device_endpoints = device.mut_connect_info().mut_endpoints();
    device_endpoints.clear();
    for ep in endpoints {
        device_endpoints.push(ep);
    }

    //init stack
    let mut params = StackOpenParams::new(deamon_name.as_str());
    let sns2 = sns.clone();
    params.known_sn = sns;
    if udp_sn_only != 0 {
        params.config.interface.udp.sn_only = true;
    } else {
        params.config.interface.udp.sn_only = false;
    }

    let stack = Stack::open(
        device, 
        private_key, 
        params).await;
        
    if let Err(err) = stack {
        println!("open stack failed for {}", err);
        return ;
    }

    let stack = stack.unwrap();

    if sns2.is_some() {
        stack.reset_sn_list(sns2.unwrap());
    }

    match future::timeout(
        Duration::from_secs(5),
        stack.sn_client().ping().wait_online(),
    ).await {
        Ok(res) => {
            match res {
                Ok(res) => {
                    match res {
                        SnStatus::Online => {
                        },
                        _ => {
                            println!("sn offline!");
                        }
                    }
                },
                Err(e) => {
                    println!("connect sn err={}", e);
                }
            }
        },
        Err(e) => {
            println!("wait_online err={}", e);
        }
    }

    if let Some(device_cache) = matches.value_of("device_cache") {
        if device_cache.len() > 0 {
            let dev = load_dev_by_path(device_cache).unwrap();
            let device_id = dev.desc().device_id();
            stack.device_cache().add(&device_id, &dev);
        }
    }

    //
    match subcommand {
        "ping" => {
            let subcommand = cmd_params.subcommand_matches("ping").unwrap();
            let remote = remote_device(&stack, subcommand.value_of("remote").unwrap(), channel).await
                .map_err(|err| format!("load remote desc {} failed for {}\r\n", subcommand.value_of("remote").unwrap(), err)).unwrap();
            let count = u32::from_str(subcommand.value_of("count").unwrap()).unwrap();
            let timeout = u64::from_str(subcommand.value_of("timeout").unwrap()).unwrap();

            let pinger = cyfs_bdt::debug::Pinger::open(stack.clone().to_weak()).unwrap();
            for _ in 0..count {
                match pinger.ping(remote.clone(), Duration::from_secs(timeout), "debug".as_ref()).await {
                    Ok(rtt) => {
                        match rtt {
                            Some(rtt) => {
                                println!("ping success, rtt is {:.2} ms", rtt as f64 / 1000.0);
                            },
                            None => {
                                println!("connected, but ping's seq mismatch");
                            }
                        }
                    },
                    Err(e) => {
                        println!("ping err={}", e);
                    }
                }
            }

        },
        "nc" => {
            let subcommand = cmd_params.subcommand_matches("nc").unwrap();
            let remote = remote_device(&stack, subcommand.value_of("remote").unwrap(), channel).await
                .map_err(|err| format!("load remote desc {} failed for {}\r\n", subcommand.value_of("remote").unwrap(), err)).unwrap();
            let port = u16::from_str(subcommand.value_of("port").unwrap()).unwrap();
            let question = b"question?";

            match stack.stream_manager().connect(
                port,
                question.to_vec(), 
                BuildTunnelParams {
                    remote_const: remote.desc().clone(), 
                    remote_sn: None, 
                    remote_desc: Some(remote.clone())
            }).await {
                Ok(conn) => {
                    println!("connect vport={} success!", port);
                    let _ = conn.shutdown(Shutdown::Both);
                },
                Err(err) => {
                    println!("connect vport={} fail, err={}", port, err);
                }
            }
        },
        _ => {
            println!("unspport cmd {}", subcommand);
        }
    }
}
