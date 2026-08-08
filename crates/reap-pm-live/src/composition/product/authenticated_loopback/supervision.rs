use tokio::task::{JoinError, JoinHandle};

/// Abortable owner for socket and read-side tasks.
///
/// Normal shutdown consumes this value through [`Self::join`]. Dropping an
/// outer run future aborts the child instead of detaching it; the child's own
/// transport guards then close any nested socket worker as well.
pub(super) struct PmAbortableTask<T> {
    task: Option<JoinHandle<T>>,
}

impl<T> PmAbortableTask<T> {
    pub(super) const fn new(task: JoinHandle<T>) -> Self {
        Self { task: Some(task) }
    }

    pub(super) async fn join(mut self) -> Result<T, JoinError> {
        let result = self
            .task
            .as_mut()
            .expect("supervised task is consumed once")
            .await;
        self.task.take();
        result
    }

    pub(super) async fn abort_and_join(&mut self) -> Result<T, JoinError> {
        let task = self
            .task
            .as_mut()
            .expect("supervised task is consumed once");
        task.abort();
        let result = task.await;
        self.task.take();
        result
    }

    pub(super) fn is_finished(&self) -> bool {
        self.task
            .as_ref()
            .expect("supervised task is consumed once")
            .is_finished()
    }
}

impl<T> Drop for PmAbortableTask<T> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Graceful-join-only owner for a post-dispatch place/cancel task.
///
/// Controlled shutdown has no abort method: after command authority crosses
/// the dispatch boundary it must drain to a terminal completion and join.
/// Dropping the root remains catastrophic/fail-closed and aborts the task;
/// restart must recover from the durable authenticated journal and never
/// blindly resend.
pub(super) struct PmMutationTask<T> {
    task: Option<JoinHandle<T>>,
}

impl<T> PmMutationTask<T> {
    pub(super) const fn new(task: JoinHandle<T>) -> Self {
        Self { task: Some(task) }
    }

    /// Await without moving this guard out of its run slot. Cancelling the
    /// caller future leaves the task owned here so a later call resumes the
    /// graceful join rather than aborting a post-dispatch operation.
    pub(super) async fn join(&mut self) -> Result<T, JoinError> {
        let result = self
            .task
            .as_mut()
            .expect("mutation task is consumed once")
            .await;
        self.task.take();
        result
    }

    pub(super) fn is_finished(&self) -> bool {
        self.task
            .as_ref()
            .expect("mutation task is consumed once")
            .is_finished()
    }
}

impl<T> Drop for PmMutationTask<T> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
