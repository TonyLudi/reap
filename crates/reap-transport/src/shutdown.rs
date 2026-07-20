use tokio::sync::watch;

/// Opaque, one-way authority to request cooperative shutdown.
///
/// The raw watch sender is deliberately not exposed: callers can request the
/// terminal transition, but cannot write `false` and revive a stopped
/// supervisor.
#[derive(Clone)]
pub struct ShutdownSender {
    inner: watch::Sender<bool>,
}

/// Opaque observation side of a monotonic cooperative-shutdown signal.
///
/// Losing every sender is treated as shutdown rather than as permission to
/// keep running.
#[derive(Clone)]
pub struct ShutdownReceiver {
    inner: watch::Receiver<bool>,
}

#[must_use]
pub fn shutdown_channel() -> (ShutdownSender, ShutdownReceiver) {
    let (sender, receiver) = watch::channel(false);
    (
        ShutdownSender { inner: sender },
        ShutdownReceiver { inner: receiver },
    )
}

/// Requests cooperative shutdown. The boolean reports whether a receiver was
/// still present to observe the transition.
pub fn request_shutdown(sender: &ShutdownSender) -> bool {
    sender.inner.send(true).is_ok()
}

#[must_use]
pub fn shutdown_requested(receiver: &ShutdownReceiver) -> bool {
    receiver.inner.has_changed().is_err() || *receiver.inner.borrow()
}

impl ShutdownReceiver {
    /// Waits for a shutdown transition or sender closure.
    ///
    /// Sender closure is returned as an error and is also observable through
    /// [`shutdown_requested`], so supervision loops fail closed.
    pub async fn changed(&mut self) -> Result<(), ShutdownChannelClosed> {
        self.inner
            .changed()
            .await
            .map_err(|_| ShutdownChannelClosed)
    }
}

impl std::fmt::Debug for ShutdownSender {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShutdownSender")
            .field("receiver_count", &self.inner.receiver_count())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ShutdownReceiver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShutdownReceiver")
            .field("requested", &shutdown_requested(self))
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownChannelClosed;

impl std::fmt::Display for ShutdownChannelClosed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("every shutdown sender was closed")
    }
}

impl std::error::Error for ShutdownChannelClosed {}
