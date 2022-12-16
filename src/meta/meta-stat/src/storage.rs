use async_trait::async_trait;
use cyfs_base::{BuckyResult, BuckyError, BuckyErrorCode};
use crate::sqlite_storage::SqliteStorage;
use log::*;
use serde::{Serialize};

pub fn map_sql_err(e: sqlx::Error) -> BuckyError {
    match e {
        sqlx::Error::RowNotFound => {
            BuckyError::from(BuckyErrorCode::NotFound)
        }
        _ => {
            let msg = format!("sql error: {:?}", e);
            error!("{}", &msg);
            BuckyError::new(BuckyErrorCode::SqliteError, msg)
        }
    }
}

#[derive(Serialize, Debug)]
pub struct MetaStat {
    pub id: String,
    pub success: u64,
    pub failed: u64,
}

#[async_trait]
pub trait Storage {
    async fn open(&mut self, db_path: &str) -> BuckyResult<()>;

    async fn init(&self) -> BuckyResult<()>;
    // people/device 数目
    async fn get_desc(&self, obj_type: u8) -> BuckyResult<u64>;
    // people/device 新增
    async fn get_desc_add(&self, obj_type: u8, start: u64, end: u64) -> BuckyResult<u64>;
    // people/device 活跃
    async fn get_desc_active(&self, obj_type: u8, start: u64, end: u64) -> BuckyResult<u64>;

    // meta success/failed
    async fn get_meta_stat(&self, meta_type: u8, start: u64, end: u64) -> BuckyResult<Vec<MetaStat>>;

}

pub async fn create_storage(db_path: &str) -> BuckyResult<Box<dyn Storage + Send + Sync>> {
    let mut storage = SqliteStorage::new();
    storage.open(db_path).await?;
    storage.init().await?;
    Ok(Box::new(storage))
}