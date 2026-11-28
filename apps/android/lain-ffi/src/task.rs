//! Runs a future on the bindings' runtime and aborts it when the caller
//! stops waiting: a cancelled Kotlin coroutine drops the UniFFI future,
//! which drops this, which aborts the task (so a cancelled download
//! stops and keeps its part file, D-239).

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::JoinHandle;

use crate::LainError;

/// A task's future as UniFFI polls it.
///
/// A future that is ready on UniFFI's *first* poll makes UniFFI invoke the
/// Kotlin continuation synchronously, on the JVM thread that is polling,
/// from inside that JNA call. On Android (ART with JNI checks, as in a
/// debuggable build) JNA then detaches that thread on the way out "while
/// still running code" and the runtime aborts the process. So a result that
/// is ready at the first poll is held back and delivered on the next poll,
/// after a wake from one of the runtime's own threads, which JNA attaches
/// and detaches cleanly.
pub(crate) struct Task<T> {
    join: JoinHandle<Result<T, LainError>>,
    rt: tokio::runtime::Handle,
    polled: bool,
    held: Option<Result<Result<T, LainError>, tokio::task::JoinError>>,
}

/// How long the deferred wake waits: long enough for UniFFI to store the
/// continuation after the first poll returns (microseconds), so the wake
/// finds it stored instead of calling it back on the JVM thread.
const DEFERRED_WAKE: std::time::Duration = std::time::Duration::from_millis(2);

fn finish<T>(r: Result<Result<T, LainError>, tokio::task::JoinError>) -> Result<T, LainError> {
    match r {
        Ok(v) => v,
        Err(e) => Err(LainError::Internal { detail: format!("task failed: {e}") }),
    }
}

impl<T> Unpin for Task<T> {}

impl<T> Future for Task<T> {
    type Output = Result<T, LainError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(r) = self.held.take() {
            return Poll::Ready(finish(r));
        }
        let first = !self.polled;
        self.polled = true;
        match Pin::new(&mut self.join).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(r) if first => {
                self.held = Some(r);
                let waker = cx.waker().clone();
                self.rt.spawn(async move {
                    tokio::time::sleep(DEFERRED_WAKE).await;
                    waker.wake();
                });
                Poll::Pending
            }
            Poll::Ready(r) => Poll::Ready(finish(r)),
        }
    }
}

impl<T> Task<T> {
    pub(crate) fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.join.abort_handle()
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.join.abort();
    }
}

pub(crate) fn spawn<T, F>(rt: &tokio::runtime::Handle, f: F) -> Task<T>
where
    T: Send + 'static,
    F: Future<Output = Result<T, LainError>> + Send + 'static,
{
    Task { join: rt.spawn(f), rt: rt.clone(), polled: false, held: None }
}

/// Runs blocking work (parsing a book, drawing a PDF page) on the
/// runtime's blocking pool, so it never stalls the async workers.
pub(crate) async fn blocking<T, F>(f: F) -> Result<T, LainError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, LainError> + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|e| LainError::Internal { detail: format!("task failed: {e}") })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Wake, Waker};

    struct Count(AtomicUsize, std::thread::ThreadId, AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
            if std::thread::current().id() == self.1 {
                self.2.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// A task already finished at the first poll is not ready then: the
    /// result comes on the next poll, after a wake from a runtime thread,
    /// never from the polling thread.
    #[test]
    fn a_result_ready_at_the_first_poll_is_delivered_after_a_runtime_wake() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_time().build().unwrap();
        let mut task = spawn(rt.handle(), async { Ok::<_, LainError>(7) });
        while !task.join.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let count = Arc::new(Count(AtomicUsize::new(0), std::thread::current().id(), AtomicUsize::new(0)));
        let waker = Waker::from(count.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(Pin::new(&mut task).poll(&mut cx).is_pending());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while count.0.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(count.0.load(Ordering::SeqCst), 1, "woken once");
        assert_eq!(count.2.load(Ordering::SeqCst), 0, "not from the polling thread");
        assert!(matches!(Pin::new(&mut task).poll(&mut cx), Poll::Ready(Ok(7))));
    }
}
