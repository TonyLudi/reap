//! Private WebSocket connection strategies shared by the public and user
//! workers.
//!
//! The production strategy deliberately preserves the existing
//! `connect_async_with_config` path. The production-selected strategy is
//! constructed only through the paired selected-WebSocket owner and accepts
//! no caller endpoint, resolver, address list, or socket. The loopback
//! strategy remains unit-test-only evidence for the same private worker seam.

use std::{future::Future, pin::Pin};

#[cfg(any(target_os = "linux", test))]
use std::{
    io,
    net::{IpAddr, SocketAddr},
};

#[cfg(test)]
use std::{
    rc::Rc,
    thread::{self, ThreadId},
};

use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::protocol::WebSocketConfig,
};

#[cfg(target_os = "linux")]
use tokio::net::TcpSocket;
#[cfg(target_os = "linux")]
use tokio_tungstenite::client_async_tls_with_config;

#[cfg(target_os = "linux")]
use socket2::SockRef;

use crate::{PmSelectedWsSocketFacts, selected_ws::PmProductionSelectedWsRouteBinding};

#[cfg(target_os = "linux")]
use crate::{PM_PUBLIC_MARKET_WS_ENDPOINT, PM_USER_WS_ENDPOINT};

pub(crate) type PmWsSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(crate) struct PmWsDialOutcome {
    socket: PmWsSocket,
    selected_socket_facts: Option<PmSelectedWsSocketFacts>,
}

impl PmWsDialOutcome {
    fn default(socket: PmWsSocket) -> Self {
        Self {
            socket,
            selected_socket_facts: None,
        }
    }

    #[cfg(target_os = "linux")]
    fn selected(socket: PmWsSocket, selected_socket_facts: PmSelectedWsSocketFacts) -> Self {
        Self {
            socket,
            selected_socket_facts: Some(selected_socket_facts),
        }
    }

    pub(crate) fn into_parts(self) -> (PmWsSocket, Option<PmSelectedWsSocketFacts>) {
        (self.socket, self.selected_socket_facts)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmWsDialFailure {
    RetryableConnect,
    TerminalInvariant,
}

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
    type Dial<'a>: Future<Output = Result<PmWsDialOutcome, PmWsDialFailure>> + 'a
    where
        Self: 'a;

    fn dial<'a>(&'a mut self, request: PmWsDialRequest<'a>) -> Self::Dial<'a>;

    fn uses_selected_reconnect_classification(&self) -> bool {
        false
    }
}

pub(crate) struct PmDefaultWsDialer;

impl PmWsDialStrategy for PmDefaultWsDialer {
    type Dial<'a> =
        Pin<Box<dyn Future<Output = Result<PmWsDialOutcome, PmWsDialFailure>> + Send + 'a>>;

    fn dial<'a>(&'a mut self, request: PmWsDialRequest<'a>) -> Self::Dial<'a> {
        Box::pin(async move {
            let _fixed_route = request.route;
            connect_async_with_config(request.endpoint, Some(request.websocket_config), true)
                .await
                .map(|(socket, _response)| PmWsDialOutcome::default(socket))
                .map_err(|_| PmWsDialFailure::RetryableConnect)
        })
    }
}

/// Private production-selected dialer. One instance owns one route binding
/// across every reconnect epoch and never consults DNS.
pub(crate) struct PmProductionSelectedWsDialer {
    binding: PmProductionSelectedWsRouteBinding,
}

impl PmProductionSelectedWsDialer {
    pub(crate) const fn new(binding: PmProductionSelectedWsRouteBinding) -> Self {
        Self { binding }
    }
}

impl PmWsDialStrategy for PmProductionSelectedWsDialer {
    type Dial<'a> = Pin<Box<dyn Future<Output = Result<PmWsDialOutcome, PmWsDialFailure>> + 'a>>;

    fn dial<'a>(&'a mut self, request: PmWsDialRequest<'a>) -> Self::Dial<'a> {
        Box::pin(async move { dial_production_selected(&self.binding, request).await })
    }

    fn uses_selected_reconnect_classification(&self) -> bool {
        true
    }
}

async fn dial_production_selected(
    binding: &PmProductionSelectedWsRouteBinding,
    request: PmWsDialRequest<'_>,
) -> Result<PmWsDialOutcome, PmWsDialFailure> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            binding.route(),
            binding.fixed_tls_peer(),
            binding.selected_local_egress(),
            binding.revalidate_process_and_thread(),
            request,
        );
        Err(PmWsDialFailure::TerminalInvariant)
    }

    #[cfg(target_os = "linux")]
    {
        if request.route != binding.route()
            || request.endpoint != exact_endpoint(binding.route())
            || !binding.revalidate_process_and_thread()
        {
            return Err(PmWsDialFailure::TerminalInvariant);
        }
        let fixed_peer = binding.fixed_tls_peer();
        let local_egress = binding.selected_local_egress();
        if fixed_peer.dns_name() != "ws-subscriptions-clob.polymarket.com"
            || fixed_peer.peer_addr().port() != 443
            || fixed_peer.require_production().is_err()
            || local_egress.require_production().is_err()
            || fixed_peer
                .require_same_address_family(local_egress)
                .is_err()
        {
            return Err(PmWsDialFailure::TerminalInvariant);
        }

        let peer_addr = fixed_peer.peer_addr();
        let local_source_ip = local_egress.local_source_ip();
        let interface_name = local_egress.interface_name().as_bytes();
        let socket = match peer_addr.ip() {
            IpAddr::V4(_) => TcpSocket::new_v4(),
            IpAddr::V6(_) => TcpSocket::new_v6(),
        }
        .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
        socket
            .set_nodelay(true)
            .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
        socket
            .bind_device(Some(interface_name))
            .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
        if socket
            .device()
            .map_err(|_| PmWsDialFailure::TerminalInvariant)?
            .as_deref()
            != Some(interface_name)
        {
            return Err(PmWsDialFailure::TerminalInvariant);
        }
        socket
            .bind(SocketAddr::new(local_source_ip, 0))
            .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
        if socket
            .local_addr()
            .map_err(|_| PmWsDialFailure::TerminalInvariant)?
            .ip()
            != local_source_ip
        {
            return Err(PmWsDialFailure::TerminalInvariant);
        }

        // Exactly one connect call receives exactly one scalar peer address.
        // Retry policy lives outside this function and can never select a
        // second address inside one epoch.
        let stream = socket.connect(peer_addr).await.map_err(|error| {
            if retryable_exact_peer_connect_error(&error) {
                PmWsDialFailure::RetryableConnect
            } else {
                PmWsDialFailure::TerminalInvariant
            }
        })?;
        validate_connected_stream(&stream, interface_name, local_source_ip, peer_addr, binding)?;

        // The canonical URL, not the literal peer, remains the TLS server
        // name, HTTP Host, and WebSocket request path.
        let (socket, _response) = client_async_tls_with_config(
            request.endpoint,
            stream,
            Some(request.websocket_config),
            None,
        )
        .await
        .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
        let tls_stream = match socket.get_ref() {
            MaybeTlsStream::Rustls(stream) => stream.get_ref().0,
            _ => return Err(PmWsDialFailure::TerminalInvariant),
        };
        let (readback_device, local_addr, connected_peer_addr) = validate_connected_stream(
            tls_stream,
            interface_name,
            local_source_ip,
            peer_addr,
            binding,
        )?;
        let selected_socket_facts = PmSelectedWsSocketFacts::from_verified_socket(
            &readback_device,
            local_addr,
            connected_peer_addr,
        )
        .ok_or(PmWsDialFailure::TerminalInvariant)?;
        Ok(PmWsDialOutcome::selected(socket, selected_socket_facts))
    }
}

#[cfg(target_os = "linux")]
fn exact_endpoint(route: PmFixedWsRoute) -> &'static str {
    match route {
        PmFixedWsRoute::PublicMarket => PM_PUBLIC_MARKET_WS_ENDPOINT,
        PmFixedWsRoute::AuthenticatedUser => PM_USER_WS_ENDPOINT,
    }
}

#[cfg(target_os = "linux")]
fn validate_connected_stream(
    stream: &TcpStream,
    expected_interface_name: &[u8],
    expected_local_ip: IpAddr,
    expected_peer_addr: SocketAddr,
    binding: &PmProductionSelectedWsRouteBinding,
) -> Result<(Vec<u8>, SocketAddr, SocketAddr), PmWsDialFailure> {
    if !binding.revalidate_process_and_thread() {
        return Err(PmWsDialFailure::TerminalInvariant);
    }
    let local_addr = stream
        .local_addr()
        .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
    let peer_addr = stream
        .peer_addr()
        .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
    let nodelay = stream
        .nodelay()
        .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
    let readback_device = SockRef::from(stream)
        .device()
        .map_err(|_| PmWsDialFailure::TerminalInvariant)?
        .ok_or(PmWsDialFailure::TerminalInvariant)?;
    if local_addr.ip() != expected_local_ip
        || peer_addr != expected_peer_addr
        || !nodelay
        || readback_device.as_slice() != expected_interface_name
    {
        return Err(PmWsDialFailure::TerminalInvariant);
    }
    Ok((readback_device, local_addr, peer_addr))
}

#[cfg(target_os = "linux")]
fn retryable_exact_peer_connect_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::TimedOut
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::HostUnreachable
    )
}

/// Unit-test-only selected-source dialer.
///
/// This value is structurally `!Send`/`!Sync`, pins its creating OS thread,
/// accepts one literal loopback peer and one literal loopback source address,
/// binds before connecting, and performs the WebSocket handshake over that
/// already-connected socket. It is intentionally not a production egress
/// claim: it observes no interface, namespace, canonical destination profile,
/// DNS answer, authorization, or public NAT identity.
#[cfg(test)]
pub(crate) struct PmTestSelectedLoopbackWsDialer {
    route: PmFixedWsRoute,
    endpoint: Box<str>,
    peer: SocketAddr,
    interface_name: Box<str>,
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
        interface_name: &str,
        local_source_ip: IpAddr,
    ) -> io::Result<Self> {
        if !peer.ip().is_loopback()
            || !local_source_ip.is_loopback()
            || peer.is_ipv4() != local_source_ip.is_ipv4()
            || interface_name.is_empty()
            || interface_name.len() > 15
        {
            return Err(invalid_test_binding_io());
        }
        Ok(Self {
            route,
            endpoint: endpoint.into(),
            peer,
            interface_name: interface_name.into(),
            local_source_ip,
            creating_thread: thread::current().id(),
            thread_confinement: Rc::new(()),
        })
    }
}

#[cfg(test)]
impl PmWsDialStrategy for PmTestSelectedLoopbackWsDialer {
    type Dial<'a> = Pin<Box<dyn Future<Output = Result<PmWsDialOutcome, PmWsDialFailure>> + 'a>>;

    fn dial<'a>(&'a mut self, request: PmWsDialRequest<'a>) -> Self::Dial<'a> {
        Box::pin(async move {
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (
                    request,
                    self.route,
                    self.endpoint.as_ref(),
                    self.peer,
                    self.interface_name.as_ref(),
                    self.local_source_ip,
                    &self.creating_thread,
                    Rc::strong_count(&self.thread_confinement),
                );
                Err(PmWsDialFailure::TerminalInvariant)
            }

            #[cfg(target_os = "linux")]
            {
                if request.route != self.route
                    || request.endpoint != self.endpoint.as_ref()
                    || thread::current().id() != self.creating_thread
                    || Rc::strong_count(&self.thread_confinement) != 1
                {
                    return Err(PmWsDialFailure::TerminalInvariant);
                }

                let socket = if self.peer.is_ipv4() {
                    TcpSocket::new_v4()
                } else {
                    TcpSocket::new_v6()
                }
                .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                socket
                    .set_nodelay(true)
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                socket
                    .bind_device(Some(self.interface_name.as_bytes()))
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                if socket
                    .device()
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?
                    .as_deref()
                    != Some(self.interface_name.as_bytes())
                {
                    return Err(PmWsDialFailure::TerminalInvariant);
                }
                socket
                    .bind(SocketAddr::new(self.local_source_ip, 0))
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                if socket
                    .local_addr()
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?
                    .ip()
                    != self.local_source_ip
                {
                    return Err(PmWsDialFailure::TerminalInvariant);
                }

                let stream = socket.connect(self.peer).await.map_err(|error| {
                    if retryable_exact_peer_connect_error(&error) {
                        PmWsDialFailure::RetryableConnect
                    } else {
                        PmWsDialFailure::TerminalInvariant
                    }
                })?;
                let local_addr = stream
                    .local_addr()
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                let peer_addr = stream
                    .peer_addr()
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                if local_addr.ip() != self.local_source_ip
                    || peer_addr != self.peer
                    || !stream
                        .nodelay()
                        .map_err(|_| PmWsDialFailure::TerminalInvariant)?
                    || thread::current().id() != self.creating_thread
                {
                    return Err(PmWsDialFailure::TerminalInvariant);
                }

                let (socket, _response) = client_async_tls_with_config(
                    request.endpoint,
                    stream,
                    Some(request.websocket_config),
                    None,
                )
                .await
                .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                if thread::current().id() != self.creating_thread
                    || Rc::strong_count(&self.thread_confinement) != 1
                {
                    return Err(PmWsDialFailure::TerminalInvariant);
                }
                let upgraded_stream = match socket.get_ref() {
                    MaybeTlsStream::Plain(stream) => stream,
                    _ => return Err(PmWsDialFailure::TerminalInvariant),
                };
                let local_addr = upgraded_stream
                    .local_addr()
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                let peer_addr = upgraded_stream
                    .peer_addr()
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?;
                if local_addr.ip() != self.local_source_ip
                    || peer_addr != self.peer
                    || !upgraded_stream
                        .nodelay()
                        .map_err(|_| PmWsDialFailure::TerminalInvariant)?
                {
                    return Err(PmWsDialFailure::TerminalInvariant);
                }
                let readback_device = SockRef::from(upgraded_stream)
                    .device()
                    .map_err(|_| PmWsDialFailure::TerminalInvariant)?
                    .ok_or(PmWsDialFailure::TerminalInvariant)?;
                if readback_device.as_slice() != self.interface_name.as_bytes() {
                    return Err(PmWsDialFailure::TerminalInvariant);
                }
                let selected_socket_facts = PmSelectedWsSocketFacts::from_verified_socket(
                    &readback_device,
                    local_addr,
                    peer_addr,
                )
                .ok_or(PmWsDialFailure::TerminalInvariant)?;
                Ok(PmWsDialOutcome::selected(socket, selected_socket_facts))
            }
        })
    }

    fn uses_selected_reconnect_classification(&self) -> bool {
        true
    }
}

#[cfg(test)]
fn invalid_test_binding_io() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "test selected WebSocket binding changed or was mismatched",
    )
}
