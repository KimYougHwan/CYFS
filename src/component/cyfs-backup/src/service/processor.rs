use super::request::*;
use cyfs_base::*;

use std::sync::Arc;

#[async_trait::async_trait]
pub trait BackupInputProcessor: Sync + Send + 'static {
    async fn start_backup_task(
        &self,
        req: StartBackupTaskInputRequest,
    ) -> BuckyResult<StartBackupTaskInputResponse>;

    async fn get_backup_task_status(
        &self,
        req: GetBackupTaskStatusInputRequest,
    ) -> BuckyResult<GetBackupTaskStatusInputResponse>;
}

pub type BackupInputProcessorRef = Arc<Box<dyn BackupInputProcessor>>;
