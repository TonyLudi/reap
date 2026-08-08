use tokio::task::{JoinError, JoinHandle};

/// Owns one nested transport task across every await in its parent future.
///
/// Tokio detaches a task when a bare [`JoinHandle`] is dropped. Transport run
/// futures are themselves supervised and therefore cancellable, so their
/// nested socket workers must instead be aborted when the outer future is
/// dropped. Explicit joins still consume the same owner.
pub(crate) struct AbortOnDropTask<T> {
    task: Option<JoinHandle<T>>,
}

impl<T> AbortOnDropTask<T> {
    pub(crate) const fn new(task: JoinHandle<T>) -> Self {
        Self { task: Some(task) }
    }

    pub(crate) async fn abort_and_join(&mut self) -> Result<T, JoinError> {
        let task = self.task.as_mut().expect("task guard is consumed once");
        task.abort();
        let result = task.await;
        self.task.take();
        result
    }

    pub(crate) async fn join(mut self) -> Result<T, JoinError> {
        let result = self
            .task
            .as_mut()
            .expect("task guard is consumed once")
            .await;
        self.task.take();
        result
    }
}

impl<T> Drop for AbortOnDropTask<T> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
