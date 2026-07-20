/// Venue-neutral lifecycle classification for one supervised connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatusKind {
    Ready,
    Heartbeat,
    Disconnected,
    Fatal,
}
