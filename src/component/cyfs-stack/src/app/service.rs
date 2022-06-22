use crate::front::*;
use crate::root_state::GlobalStateInputProcessorRef;
use crate::root_state::GlobalStateOutputTransformer;
use crate::ZoneManager;
use cyfs_base::*;
use cyfs_debug::Mutex;
use cyfs_lib::GlobalStateStub;

use std::collections::HashMap;
use std::sync::Arc;

pub enum AppInstallStatus {
    Installed((ObjectId, ObjectId)),
    NotInstalled(ObjectId),
}

#[derive(Clone)]
pub struct AppService {
    cache: Arc<Mutex<HashMap<String, ObjectId>>>,
    root_state_stub: GlobalStateStub,
}

impl AppService {
    pub async fn new(
        zone_manager: &ZoneManager,
        root_state: GlobalStateInputProcessorRef,
    ) -> BuckyResult<Self> {
        let info = zone_manager.get_current_info().await?;

        let processor = GlobalStateOutputTransformer::new(root_state, info.device_id.clone());
        let root_state_stub = GlobalStateStub::new(
            processor,
            Some(info.zone_device_ood_id.object_id().clone()),
            Some(cyfs_core::get_system_dec_app().object_id().to_owned()),
        );

        Ok(Self {
            root_state_stub,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn get_app_web_dir(
        &self,
        dec: &FrontARequestDec,
        ver: &FrontARequestVersion,
    ) -> BuckyResult<AppInstallStatus> {
        let dec_id = self.get_app(dec).await?;

        let ret = self.search_app_web_dir(&dec_id, ver).await?;
        let status = match ret {
            Some(dir_id) => AppInstallStatus::Installed((dec_id, dir_id)),
            None => AppInstallStatus::NotInstalled(dec_id),
        };

        Ok(status)
    }

    pub async fn get_app_local_status(
        &self,
        dec: &FrontARequestDec,
    ) -> BuckyResult<AppInstallStatus> {
        let dec_id = self.get_app(dec).await?;

        let ret = self.search_local_status(&dec_id).await?;
        let status = match ret {
            Some(local_status_id) => AppInstallStatus::Installed((dec_id, local_status_id)),
            None => AppInstallStatus::NotInstalled(dec_id),
        };

        Ok(status)
    }

    async fn get_app(&self, dec: &FrontARequestDec) -> BuckyResult<ObjectId> {
        let dec_id = match dec {
            FrontARequestDec::DecID(dec_id) => dec_id.to_owned(),
            FrontARequestDec::Name(name) => self.get_app_by_name(name).await?,
        };

        Ok(dec_id)
    }

    fn get_app_from_cache(&self, name: &str) -> Option<ObjectId> {
        let cache = self.cache.lock().unwrap();
        cache.get(name).map(|v| v.to_owned())
    }

    fn cache_app(&self, name: &str, dec_id: ObjectId) {
        let mut cache = self.cache.lock().unwrap();
        cache.insert(name.to_owned(), dec_id);
    }

    // 获取dec_app的状态
    async fn search_local_status(&self, dec_id: &ObjectId) -> BuckyResult<Option<ObjectId>> {
        let op_env = self.root_state_stub.create_path_op_env().await?;

        let path = format!("/app/{}/local_status", dec_id.to_string());
        let ret = op_env.get_by_path(&path).await?;
        let _ = op_env.abort().await;
        if ret.is_none() {
            let msg = format!(
                "get app local_status by name but not found! dec={}, path={}",
                dec_id, path,
            );
            warn!("{}", msg);
            return Ok(None);
        }

        let local_status_id = ret.unwrap();
        info!("get app local_status: {} -> {}", dec_id, local_status_id);

        Ok(Some(local_status_id))
    }

    async fn search_app_web_dir(
        &self,
        dec_id: &ObjectId,
        ver: &FrontARequestVersion,
    ) -> BuckyResult<Option<ObjectId>> {
        let ver_seg = match ver {
            FrontARequestVersion::Current | FrontARequestVersion::DirID(_) => "current",
            FrontARequestVersion::Version(ver) => ver.as_str(),
        };

        let op_env = self.root_state_stub.create_path_op_env().await?;

        let path = format!("/app/{}/versions/{}", dec_id, ver_seg);
        let ret = op_env.get_by_path(&path).await?;
        let _ = op_env.abort().await;
        if ret.is_none() {
            let msg = format!(
                "get app dir_id by version but not found! dec={}, path={}",
                dec_id, path,
            );
            warn!("{}", msg);
            return Ok(None);
        }

        let dir_id = ret.unwrap();
        match ver {
            FrontARequestVersion::DirID(id) => {
                if *id != dir_id {
                    let msg = format!(
                        "get app dir-id by version but not match! dec-id={}, current={}, request={}",
                        dec_id, dir_id, id,
                    );
                    warn!("{}", msg);
                    return Ok(None);
                }
            }
            _ => {}
        };

        info!(
            "get app dir-id by version: dec={}, ver={}, dir={}",
            dec_id, ver_seg, dir_id,
        );

        Ok(Some(dir_id))
    }

    async fn get_app_by_name(&self, name: &str) -> BuckyResult<ObjectId> {
        if let Some(dec_id) = self.get_app_from_cache(name) {
            return Ok(dec_id);
        }

        // TODO add failure cache

        self.search_app_by_name(name).await
    }

    // get dec-id by name from /system/app/names/${name}
    async fn search_app_by_name(&self, name: &str) -> BuckyResult<ObjectId> {
        let op_env = self.root_state_stub.create_path_op_env().await?;

        let name_path = format!("/app/names/{}", name);
        let ret = op_env.get_by_path(&name_path).await?;
        let _ = op_env.abort().await;
        if ret.is_none() {
            let msg = format!(
                "get app by name but not found! name={}, path={}",
                name, name_path,
            );
            warn!("{}", msg);
            return Err(BuckyError::new(BuckyErrorCode::NotFound, msg));
        }

        info!("get app by name: {} -> {}", name, ret.as_ref().unwrap());

        let dec_id = ret.unwrap();
        self.cache_app(name, dec_id.clone());

        Ok(dec_id)
    }
}
