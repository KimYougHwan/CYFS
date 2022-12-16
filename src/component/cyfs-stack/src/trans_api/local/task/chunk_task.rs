use super::super::download_task_manager::DownloadTaskState;
use crate::NamedDataComponents;
use crate::ndn_api::{
    ChunkListReaderAdapter, ChunkManagerWriter, ChunkWriter, ChunkWriterRef, LocalChunkWriter,
};
use crate::trans_api::TransStore;
use cyfs_base::*;
use cyfs_bdt::{self, SingleDownloadContext, StackGuard};
use cyfs_task_manager::*;

use cyfs_debug::Mutex;
use sha2::Digest;
use std::path::PathBuf;
use std::sync::Arc;

pub struct DownloadChunkTask {
    task_id: TaskId,
    chunk_id: ChunkId,
    bdt_stack: StackGuard,
    device_list: Vec<DeviceId>,
    referer: String,
    group: Option<String>,
    context_id: Option<ObjectId>,
    session: async_std::sync::Mutex<Option<String>>,
    writer: ChunkWriterRef,
    task_store: Option<Arc<dyn TaskStore>>,
    task_status: Mutex<TaskStatus>,
}

impl DownloadChunkTask {
    pub(crate) fn new(
        chunk_id: ChunkId,
        bdt_stack: StackGuard,
        device_list: Vec<DeviceId>,
        referer: String,
        group: Option<String>,
        context_id: Option<ObjectId>,
        task_label_data: Vec<u8>,
        writer: Box<dyn ChunkWriter>,
    ) -> Self {
        let mut sha256 = sha2::Sha256::new();
        sha256.input(DOWNLOAD_CHUNK_TASK.0.to_le_bytes());
        sha256.input(chunk_id.as_slice());
        sha256.input(task_label_data.as_slice());
        let task_id = sha256.result().into();
        Self {
            task_id,
            chunk_id,
            bdt_stack,
            device_list,
            referer,
            group,
            context_id,
            session: async_std::sync::Mutex::new(None),
            writer: Arc::new(writer),
            task_store: None,
            task_status: Mutex::new(TaskStatus::Stopped),
        }
    }
}

#[async_trait::async_trait]
impl Task for DownloadChunkTask {
    fn get_task_id(&self) -> TaskId {
        self.task_id.clone()
    }

    fn get_task_type(&self) -> TaskType {
        DOWNLOAD_CHUNK_TASK
    }

    fn get_task_category(&self) -> TaskCategory {
        DOWNLOAD_TASK_CATEGORY
    }

    async fn get_task_status(&self) -> TaskStatus {
        *self.task_status.lock().unwrap()
    }

    async fn set_task_store(&mut self, task_store: Arc<dyn TaskStore>) {
        self.task_store = Some(task_store);
    }

    async fn start_task(&self) -> BuckyResult<()> {
        let mut session = self.session.lock().await;
        // if session.is_some() {
        //     session.as_ref().unwrap().resume()?;
        //     return Ok(());
        // }

        {
            if *self.task_status.lock().unwrap() == TaskStatus::Running {
                return Ok(());
            }
        }

        let context = SingleDownloadContext::id_streams(
            &self.bdt_stack,
            self.referer.clone(),
            &self.device_list,
        )
        .await?;

        // 创建bdt层的传输任务
        let (id, reader) =
            cyfs_bdt::download_chunk(&self.bdt_stack, self.chunk_id.clone(), self.group.clone(), context)
                .await
                .map_err(|e| {
                    error!(
                        "start bdt chunk trans session error! task_id={}, {}",
                        self.task_id.to_string(),
                        e
                    );
                    e
                })?;

        *session = Some(id);

        ChunkListReaderAdapter::new_chunk(self.writer.clone(), reader, &self.chunk_id).async_run();

        info!(
            "create bdt chunk trans session success: task={}, device={:?}",
            self.task_id.to_string(),
            self.device_list,
        );
        *self.task_status.lock().unwrap() = TaskStatus::Running;
        self.task_store
            .as_ref()
            .unwrap()
            .save_task_status(&self.task_id, TaskStatus::Running)
            .await?;

        Ok(())
    }

    async fn pause_task(&self) -> BuckyResult<()> {
        let task_group = self.session.lock().await.clone();
        if let Some(id) = task_group {
            let task = self
                .bdt_stack
                .ndn()
                .root_task()
                .download()
                .sub_task(&id)
                .ok_or_else(|| {
                    let msg = format!("get task but ot found! task={}, group={}", self.task_id, id);
                    error!("{}", msg);
                    BuckyError::new(BuckyErrorCode::NotFound, msg)
                })?;

            task.pause().map_err(|e| {
                error!(
                    "pause task failed! task={}, group={}, {}",
                    self.task_id, id, e
                );
                e
            })?;
        } else {
            let msg = format!(
                "pause task but task group not exists! task={}",
                self.task_id
            );
            error!("{}", msg);
        }

        *self.task_status.lock().unwrap() = TaskStatus::Paused;
        self.task_store
            .as_ref()
            .unwrap()
            .save_task_status(&self.task_id, TaskStatus::Paused)
            .await?;
        Ok(())
    }

    async fn stop_task(&self) -> BuckyResult<()> {
        let task_group = self.session.lock().await.take();
        if let Some(id) = task_group {
            let task = self
                .bdt_stack
                .ndn()
                .root_task()
                .download()
                .sub_task(&id)
                .ok_or_else(|| {
                    let msg = format!("get task but ot found! task={}, group={}", self.task_id, id);
                    error!("{}", msg);
                    BuckyError::new(BuckyErrorCode::NotFound, msg)
                })?;

            task.cancel().map_err(|e| {
                error!(
                    "stop task failed! task={}, group={}, {}",
                    self.task_id, id, e
                );
                e
            })?;
        } else {
            let msg = format!("stop task but task group not exists! task={}", self.task_id);
            error!("{}", msg);
        }

        *self.task_status.lock().unwrap() = TaskStatus::Stopped;
        self.task_store
            .as_ref()
            .unwrap()
            .save_task_status(&self.task_id, TaskStatus::Stopped)
            .await?;
        Ok(())
    }

    async fn get_task_detail_status(&self) -> BuckyResult<Vec<u8>> {
        let task_group = self.session.lock().await.clone();
        let task_state = if let Some(id) = task_group {
            let task = self
                .bdt_stack
                .ndn()
                .root_task()
                .download()
                .sub_task(&id)
                .ok_or_else(|| {
                    let msg = format!("get task but ot found! task={}, group={}", self.task_id, id);
                    error!("{}", msg);
                    BuckyError::new(BuckyErrorCode::NotFound, msg)
                })?;

            let state = task.state();
            match state {
                cyfs_bdt::DownloadTaskState::Downloading(speed, progress) => DownloadTaskState {
                    task_status: TaskStatus::Running,
                    err_code: None,
                    speed: speed as u64,
                    upload_speed: 0,
                    downloaded_progress: progress as u64,
                    sum_size: self.chunk_id.len() as u64,
                },
                cyfs_bdt::DownloadTaskState::Paused => DownloadTaskState {
                    task_status: TaskStatus::Paused,
                    err_code: None,
                    speed: 0,
                    upload_speed: 0,
                    downloaded_progress: 0,
                    sum_size: self.chunk_id.len() as u64,
                },
                cyfs_bdt::DownloadTaskState::Error(err) => {
                    if err.code() == BuckyErrorCode::Interrupted {
                        DownloadTaskState {
                            task_status: TaskStatus::Stopped,
                            err_code: None,
                            speed: 0,
                            upload_speed: 0,
                            downloaded_progress: 0,
                            sum_size: self.chunk_id.len() as u64,
                        }
                    } else {
                        *self.task_status.lock().unwrap() = TaskStatus::Failed;
                        self.task_store
                            .as_ref()
                            .unwrap()
                            .save_task_status(&self.task_id, TaskStatus::Failed)
                            .await?;
                        DownloadTaskState {
                            task_status: TaskStatus::Failed,
                            err_code: Some(err.code()),
                            speed: 0,
                            upload_speed: 0,
                            downloaded_progress: 0,
                            sum_size: 0,
                        }
                    }
                }
                cyfs_bdt::DownloadTaskState::Finished => {
                    *self.task_status.lock().unwrap() = TaskStatus::Finished;
                    self.task_store
                        .as_ref()
                        .unwrap()
                        .save_task_status(&self.task_id, TaskStatus::Finished)
                        .await?;
                    DownloadTaskState {
                        task_status: TaskStatus::Finished,
                        err_code: None,
                        speed: 0,
                        upload_speed: 0,
                        downloaded_progress: 100,
                        sum_size: self.chunk_id.len() as u64,
                    }
                }
            }
        } else {
            *self.task_status.lock().unwrap() = TaskStatus::Stopped;
            self.task_store
                .as_ref()
                .unwrap()
                .save_task_status(&self.task_id, TaskStatus::Stopped)
                .await?;
            DownloadTaskState {
                task_status: TaskStatus::Stopped,
                err_code: None,
                speed: 0,
                upload_speed: 0,
                downloaded_progress: 0,
                sum_size: self.chunk_id.len() as u64,
            }
        };
        Ok(task_state.to_vec()?)
    }
}

#[derive(Clone, ProtobufEncode, ProtobufDecode, ProtobufTransformType)]
#[cyfs_protobuf_type(super::super::trans_proto::DownloadChunkParam)]
pub struct DownloadChunkParam {
    pub chunk_id: ChunkId,
    pub device_list: Vec<DeviceId>,
    pub referer: String,
    pub save_path: Option<String>,
    pub group: Option<String>,
    pub context_id: Option<ObjectId>,
}

impl ProtobufTransform<super::super::trans_proto::DownloadChunkParam> for DownloadChunkParam {
    fn transform(
        value: crate::trans_api::local::trans_proto::DownloadChunkParam,
    ) -> BuckyResult<Self> {
        let mut device_list = Vec::new();
        for item in value.device_list.iter() {
            device_list.push(DeviceId::clone_from_slice(item.as_slice())?);
        }
        Ok(Self {
            chunk_id: ChunkId::from(value.chunk_id),
            device_list,
            referer: value.referer,
            save_path: value.save_path,
            context_id: if value.context_id.is_some() {
                Some(ObjectId::clone_from_slice(
                    value.context_id.as_ref().unwrap().as_slice(),
                ))
            } else {
                None
            },
            group: value.group,
        })
    }
}

impl ProtobufTransform<&DownloadChunkParam> for super::super::trans_proto::DownloadChunkParam {
    fn transform(value: &DownloadChunkParam) -> BuckyResult<Self> {
        let mut device_list = Vec::new();
        for item in value.device_list.iter() {
            device_list.push(item.to_vec()?);
        }
        Ok(Self {
            chunk_id: value.chunk_id.as_slice().to_vec(),
            device_list,
            referer: value.referer.clone(),
            save_path: value.save_path.clone(),
            context_id: if value.context_id.is_some() {
                Some(value.context_id.as_ref().unwrap().to_vec()?)
            } else {
                None
            },
            group: value.group.clone(),
        })
    }
}

impl DownloadChunkParam {
    pub fn chunk_id(&self) -> &ChunkId {
        &self.chunk_id
    }

    pub fn device_list(&self) -> &Vec<DeviceId> {
        &self.device_list
    }

    pub fn referer(&self) -> &str {
        self.referer.as_str()
    }

    pub fn save_path(&self) -> &Option<String> {
        &self.save_path
    }

    pub fn context_id(&self) -> &Option<ObjectId> {
        &self.context_id
    }

    pub fn group(&self) -> &Option<String> {
        &self.group
    }
}

pub struct DownloadChunkTaskFactory {
    stack: StackGuard,
    named_data_components: NamedDataComponents,
    trans_store: Arc<TransStore>,
}

impl DownloadChunkTaskFactory {
    pub fn new(
        stack: StackGuard,
        named_data_components: NamedDataComponents,
        trans_store: Arc<TransStore>,
    ) -> Self {
        Self {
            stack,
            named_data_components,
            trans_store,
        }
    }
}

#[async_trait::async_trait]
impl TaskFactory for DownloadChunkTaskFactory {
    fn get_task_type(&self) -> TaskType {
        DOWNLOAD_CHUNK_TASK
    }

    async fn create(&self, params: &[u8]) -> BuckyResult<Box<dyn Task>> {
        let param = DownloadChunkParam::clone_from_slice(params)?;
        let (writer, label_data) =
            if param.save_path().is_some() && !param.save_path().as_ref().unwrap().is_empty() {
                let chunk_writer: Box<dyn ChunkWriter> = Box::new(LocalChunkWriter::new(
                    PathBuf::from(param.save_path().as_ref().unwrap().clone()),
                    self.named_data_components.ndc.clone(),
                    self.named_data_components.tracker.clone(),
                ));
                (
                    chunk_writer,
                    param.save_path().as_ref().unwrap().as_bytes().to_vec(),
                )
            } else {
                let chunk_writer: Box<dyn ChunkWriter> = Box::new(ChunkManagerWriter::new(
                    self.named_data_components.chunk_manager.clone(),
                    self.named_data_components.ndc.clone(),
                    self.named_data_components.tracker.clone(),
                ));
                (chunk_writer, Vec::new())
            };

        let task = DownloadChunkTask::new(
            param.chunk_id,
            self.stack.clone(),
            param.device_list,
            param.referer,
            param.group,
            param.context_id,
            label_data,
            writer,
        );
        Ok(Box::new(task))
    }

    async fn restore(
        &self,
        _task_status: TaskStatus,
        params: &[u8],
        _data: &[u8],
    ) -> BuckyResult<Box<dyn Task>> {
        let param = DownloadChunkParam::clone_from_slice(params)?;
        let (writer, label_data) =
            if param.save_path().is_some() && !param.save_path().as_ref().unwrap().is_empty() {
                let chunk_writer: Box<dyn ChunkWriter> = Box::new(LocalChunkWriter::new(
                    PathBuf::from(param.save_path().as_ref().unwrap().clone()),
                    self.named_data_components.ndc.clone(),
                    self.named_data_components.tracker.clone(),
                ));
                (
                    chunk_writer,
                    param.save_path().as_ref().unwrap().as_bytes().to_vec(),
                )
            } else {
                let chunk_writer: Box<dyn ChunkWriter> = Box::new(ChunkManagerWriter::new(
                    self.named_data_components.chunk_manager.clone(),
                    self.named_data_components.ndc.clone(),
                    self.named_data_components.tracker.clone(),
                ));
                (chunk_writer, Vec::new())
            };

        let task = DownloadChunkTask::new(
            param.chunk_id,
            self.stack.clone(),
            param.device_list,
            param.referer,
            param.group,
            param.context_id,
            label_data,
            writer,
        );
        Ok(Box::new(task))
    }
}
