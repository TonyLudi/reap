//! Private WebSocket connection strategies shared by the public and user
//! workers.
//!
//! The production strategy deliberately preserves the existing
//! `connect_async_with_config` path. The loopback-selected strategy exists
//! only in unit tests so the workers can be exercised with a thread-confined
//! dialer before any production selected-egress constructor or proof type is
//! introduced.

use std::{future::Future, pin::Pin};

#[cfg(test)]
use std::{
    io,
    net::{IpAddr, SocketAddr},
    rc::Rc,
    thread::{self, ThreadId},
};

#[cfg(test)]
use tokio::net::TcpSocket;
use tokio::net::TcpStream;
#[cfg(test)]
use tokio_tungstenite::client_async_tls_with_config;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Error as WebSocketError, protocol::WebSocketConfig},
};

pub(crate) type PmWsSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmFixedWsRoute {
    PublicMarket,
    AuthenticatedUser,
}

pub(crate) struct PmWsDialRequest<'a> {
    route: PmFixedWsRoute,
    endpoint: &'a str,
    websocket_config: WebSocketConfig,
}

impl<'a> PmWsDialRequest<'a> {
    pub(crate) const fn new(
        route: PmFixedWsRoute,
        endpoint: &'a str,
        websocket_config: WebSocketConfig,
    ) -> Self {
        Self {
            route,
            endpoint,
            websocket_config,
        }
    }
}

/// Crate-private connection seam. It cannot be implemented by orchestration
/// and never releases a socket from the live-adapter workers.
pub(crate) trait PmWsDialStrategy: 'static {
    type Dial<'a>: Future<Output = Result<PmWsSocket, WebSocketError>> + 'a
    where
        Self: 'a;

    fn dial<'a>(&'a mut self, request: PmWsDialRequest<'a>) -> Self::Dial<'a>;
}

pub(crate) struct PmDefaultWsDialer;

impl PmWsDialStrategy for PmDefaultWsDialer {
    type Dial<'a> = Pin<Box<dyn Future<Output = Result<PmWsSocket, WebSocketError>> + Send + 'a>>;

    fn dial<'a>(&'a mut self, request: PmWsDialRequest<'a>) -> Self::Dial<'a> {
        Box::pin(async move {
            let _fixed_route = request.route;
            connect_async_with_config(request.endpoint, Some(request.websocket_config), true)
                .await
                .map(|(socket, _response)| socket)
        })
    }
}

/// Unit-test-only selected-source dialer.
///
/// This value is structurally `!Send`/`!Sync`, pins its creating OS thread,
/// accepts one literal loopback peer and one literal loopback source address,
/// binds before connecting, and performs the WebSocket handshake over that
/// already-connected socket. It is intentionally not a production egress
/// claim: it observes no interface, namespace, reviewed profile, DNS answer,
/// authorization, or public NAT identity.
#[cfg(test)]
pub(crate) struct PmTestSelectedLoopbackWsDialer {
    route: PmFixedWsRoute,
    endpoint: Box<str>,
    peer: SocketAddr,
    local_source_ip: IpAddr,
    creating_thread: ThreadId,
    thread_confinement: Rc<()>,
}

#[cfg(test)]
impl PmTestSelectedLoopbackWsDialer {
    pub(crate) fn new(
        route: PmFixedWsRoute,
        endpoint: &str,
        peer: SocketAddr,
        local_source_ip: IpAddr,
    ) -> io::Result<Self> {
        if !peer.ip().is_loopback()
            || !local_source_ip.is_loopback()
            || peer.is_ipv4() != local_source_ip.is_ipv4()
        {
            return Err(invalid_test_binding_io());
        }
        Ok(Self {
            route,
            endpoint: endpoint.into(),
            peer,
            local_source_ip,
            creating_thread: thread::current().id(),
            thread_confinement: Rc::new(()),
        })
    }
}

#[cfg(test)]
impl PmWsDialStrategy for PmTestSelectedLoopbackWsDialer {
    type Dial<'a> = Pin<Box<dyn Future<Output = Result<PmWsSocket, WebSocketError>> + 'a>>;

    fn dial<'a>(&'a mut self, request: PmWsDialRequest<'a>) -> Self::Dial<'a> {
        Box::pin(async move {
            if request.route != self.route
                || request.endpoint != self.endpoint.as_ref()
                || thread::current().id() != self.creating_thread
                || Rc::strong_count(&self.thread_confinement) != 1
            {
                return Err(invalid_test_binding());
            }

            let socket = if self.peer.is_ipv4() {
                TcpSocket::new_v4()
            } else {
                TcpSocket::new_v6()
            }
            .map_err(WebSocketError::Io)?;
            socket.set_nodelay(true).map_err(WebSocketError::Io)?;
            socket
                .bind(SocketAddr::new(self.local_source_ip, 0))
                .map_err(WebSocketError::Io)?;
            if socket.local_addr().map_err(WebSocketError::Io)?.ip() != self.local_source_ip {
                return Err(invalid_test_binding());
            }

            let stream = socket
                .connect(self.peer)
                .await
                .map_err(WebSocketError::Io)?;
            if stream.local_addr().map_err(WebSocketError::Io)?.ip() != self.local_source_ip
                || stream.peer_addr().map_err(WebSocketError::Io)? != self.peer
                || !stream.nodelay().map_err(WebSocketError::Io)?
                || thread::current().id() != self.creating_thread
            {
                return Err(invalid_test_binding());
            }

            let (socket, _response) = client_async_tls_with_config(
                request.endpoint,
                stream,
                Some(request.websocket_config),
                None,
            )
            .await?;
            if thread::current().id() != self.creating_thread
                || Rc::strong_count(&self.thread_confinement) != 1
            {
                return Err(invalid_test_binding());
            }
            Ok(socket)
        })
    }
}

#[cfg(test)]
fn invalid_test_binding() -> WebSocketError {
    WebSocketError::Io(invalid_test_binding_io())
}

#[cfg(test)]
fn invalid_test_binding_io() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "test selected WebSocket binding changed or was mismatched",
    )
}
