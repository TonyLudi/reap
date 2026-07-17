use super::*;

pub(super) fn is_zero_order_reconciliation(report: &ReconcileReport) -> bool {
    report.is_clean() && report.local_live_orders == 0 && report.remote_live_orders == 0
}

/// Owns spawned tasks until the runtime has adopted them. Dropping a partially
/// constructed runtime cannot orphan a task that still has exchange authority.
#[derive(Default)]
pub(super) struct StartupTaskGroup(Vec<JoinHandle<()>>);

impl StartupTaskGroup {
    pub(super) fn take(&mut self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.0)
    }
}

impl std::ops::Deref for StartupTaskGroup {
    type Target = Vec<JoinHandle<()>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for StartupTaskGroup {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for StartupTaskGroup {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

#[cfg(unix)]
pub(super) async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
pub(super) async fn shutdown_signal() -> Result<(), std::io::Error> {
    tokio::signal::ctrl_c().await
}
