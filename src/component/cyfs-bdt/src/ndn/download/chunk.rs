use std::{
    sync::RwLock, 
    io::SeekFrom, 
};
use async_std::{
    sync::Arc, 
    pin::Pin, 
    task::{Context, Poll},
};

use cyfs_base::*;
use crate::{
    types::*, 
    stack::{WeakStack, Stack}
};
use super::super::{
    chunk::*, 
};
use super::{
    common::*
};


enum TaskStateImpl {
    Downloading(IncreaseId, ChunkCache),
    Error(BuckyError), 
    Finished(ChunkCache),
}

enum ControlStateImpl {
    Normal(StateWaiter), 
    Canceled,
}

struct StateImpl {
    control_state: ControlStateImpl, 
    task_state: TaskStateImpl,
}

struct ChunkTaskImpl {
    stack: WeakStack, 
    chunk: ChunkId, 
    context: Box<dyn DownloadContext>, 
    state: RwLock<StateImpl>,  
}

#[derive(Clone)]
pub struct ChunkTask(Arc<ChunkTaskImpl>);

impl std::fmt::Display for ChunkTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ChunkTask{{chunk:{}}}", self.chunk())
    }
}

impl ChunkTask {
    pub fn new(
        stack: WeakStack, 
        chunk: ChunkId, 
        context: Box<dyn DownloadContext>, 
    ) -> Self {
        let strong_stack = Stack::from(&stack);
        let cache = strong_stack.ndn().chunk_manager().create_cache(&chunk);
        let id = cache.downloader().context().add_context(context.as_ref());
        
        Self(Arc::new(ChunkTaskImpl {
            stack, 
            chunk, 
            context, 
            state: RwLock::new(StateImpl {
                task_state: TaskStateImpl::Downloading(id, cache.clone()), 
                control_state: ControlStateImpl::Normal(StateWaiter::new()),
            }),
        }))
    } 

    pub fn chunk(&self) -> &ChunkId {
        &self.0.chunk
    }

}

#[async_trait::async_trait]
impl DownloadTask for ChunkTask {
    fn clone_as_task(&self) -> Box<dyn DownloadTask> {
        Box::new(self.clone())
    }

    fn state(&self) -> DownloadTaskState {
        match &self.0.state.read().unwrap().task_state {
            TaskStateImpl::Downloading(_, cache) => DownloadTaskState::Downloading(cache.downloader().cur_speed(), 0.0), 
            TaskStateImpl::Error(err) => DownloadTaskState::Error(err.clone()), 
            TaskStateImpl::Finished(_) => DownloadTaskState::Finished
        }
    }

    fn control_state(&self) -> DownloadTaskControlState {
        match &self.0.state.read().unwrap().control_state {
            ControlStateImpl::Normal(_) => DownloadTaskControlState::Normal, 
            ControlStateImpl::Canceled => DownloadTaskControlState::Canceled
        }
    }

    fn priority_score(&self) -> u8 {
        DownloadTaskPriority::Normal as u8
    }

    fn sub_task(&self, _path: &str) -> Option<Box<dyn DownloadTask>> {
        None
    }

    fn calc_speed(&self, when: Timestamp) -> u32 {
        if let Some(cache) = {
            let state = self.0.state.read().unwrap();
            match &state.task_state {
                TaskStateImpl::Downloading(_, cache) => Some(cache.clone()), 
                _ => None
            }
        } {
            cache.downloader().calc_speed(when)
        } else {
            0
        }
    }

    fn cur_speed(&self) -> u32 {
        if let Some(cache) = {
            let state = self.0.state.read().unwrap();
            match &state.task_state {
                TaskStateImpl::Downloading(_, cache) => Some(cache.clone()), 
                _ => None
            }
        } {
            cache.downloader().cur_speed()
        } else {
            0
        }
    }

    fn history_speed(&self) -> u32 {
        if let Some(cache) = {
            let state = self.0.state.read().unwrap();
            match &state.task_state {
                TaskStateImpl::Downloading(_, cache) => Some(cache.clone()), 
                _ => None
            }
        } {
            cache.downloader().history_speed()
        } else {
            0
        }
    }

    fn drain_score(&self) -> i64 {
        if let Some(cache) = {
            let state = self.0.state.read().unwrap();
            match &state.task_state {
                TaskStateImpl::Downloading(_, cache) => Some(cache.clone()), 
                _ => None
            }
        } {
            cache.downloader().drain_score()
        } else {
            0
        }
    }

    fn on_drain(&self, expect_speed: u32) -> u32 {
        if let Some(cache) = {
            let state = self.0.state.read().unwrap();
            match &state.task_state {
                TaskStateImpl::Downloading(_, cache) => Some(cache.clone()), 
                _ => None
            }
        } {
            cache.downloader().on_drain(expect_speed)
        } else {
            0
        }
    }

    fn cancel(&self) -> BuckyResult<DownloadTaskControlState> {
        let (waiters, cancel) = {
            let mut state = self.0.state.write().unwrap();
            let waiters = match &mut state.control_state {
                ControlStateImpl::Normal(waiters) => {
                    let waiters = Some(waiters.transfer());
                    state.control_state = ControlStateImpl::Canceled;
                    waiters
                }, 
                _ => None
            };

            let cancel = match &state.task_state {
                TaskStateImpl::Downloading(id, cache) => {
                    let cancel = Some((*id, cache.clone()));
                    state.task_state = TaskStateImpl::Error(BuckyError::new(BuckyErrorCode::UserCanceled, "cancel invoked"));
                    cancel
                }, 
                _ => None
            };

            (waiters, cancel)
        };

        if let Some(waiters) = waiters {
            waiters.wake();
        }

        if let Some((id, cache)) = cancel {
            cache.downloader().context().remove_context(&id);
        }

        Ok(DownloadTaskControlState::Canceled)
    }

    async fn wait_user_canceled(&self) -> BuckyError {
        let waiter = {
            let mut state = self.0.state.write().unwrap();
            match &mut state.control_state {
                ControlStateImpl::Normal(waiters) => Some(waiters.new_waiter()), 
                _ => None
            }
        };
        
        if let Some(waiter) = waiter {
            let _ = StateWaiter::wait(waiter, || self.control_state()).await;
        } 

        BuckyError::new(BuckyErrorCode::UserCanceled, "")
    }
}


pub struct ChunkTaskReader(DownloadTaskReader);

impl Drop for ChunkTaskReader {
    fn drop(&mut self) {
        let _ = self.0.task().cancel();
    }
}

impl std::io::Seek for ChunkTaskReader {
    fn seek(
        self: &mut Self,
        pos: SeekFrom,
    ) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.0, pos)
    }
}

impl async_std::io::Read for ChunkTaskReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        async_std::io::Read::poll_read(Pin::new(&mut self.get_mut().0), cx, buffer)
    }
}
impl ChunkTask {
    pub fn reader(
        stack: WeakStack, 
        chunk: ChunkId, 
        context: Box<dyn DownloadContext>, 
    ) -> (Self, ChunkTaskReader) {
        let strong_stack = Stack::from(&stack);
        let cache = strong_stack.ndn().chunk_manager().create_cache(&chunk);
        let id = cache.downloader().context().add_context(context.as_ref());
        
        let task = Self(Arc::new(ChunkTaskImpl {
            stack, 
            chunk, 
            context, 
            state: RwLock::new(StateImpl {
                task_state: TaskStateImpl::Downloading(id, cache.clone()), 
                control_state: ControlStateImpl::Normal(StateWaiter::new()),
            }),
        }));

        let reader = ChunkTaskReader(DownloadTaskReader::new(cache, task.clone_as_task()));

        (task, reader)
    }
}