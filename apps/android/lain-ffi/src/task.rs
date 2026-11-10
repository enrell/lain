//! Runs a future on the bindings' runtime and aborts it when the caller
//! stops waiting: a cancelled Kotlin coroutine drops the UniFFI future,
//! which drops this, which aborts the task (so a cancelled download
//! stops and keeps its part file, D-239).

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::JoinHandle;

use crate::LainError;

pub(crate) struct Task<T>(JoinHandle<Result<T, LainError>>);

impl<T> Future for Task<T> {
    type Output = Result<T, LainError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx).map(|r| match r {
            Ok(v) => v,
            Err(e) => Err(LainError::Internal { detail: format!("task failed: {e}") }),
        })
    }
}

impl<T> Task<T> {
    pub(crate) fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.0.abort_handle()
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn spawn<T, F>(rt: &tokio::runtime::Handle, f: F) -> Task<T>
where
    T: Send + 'static,
    F: Future<Output = Result<T, LainError>> + Send + 'static,
{
    Task(rt.spawn(f))
}
