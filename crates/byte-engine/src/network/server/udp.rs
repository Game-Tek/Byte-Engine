//! UDP transport for an authoritative BETP server.

use std::{
	io::ErrorKind,
	net::{SocketAddr, ToSocketAddrs, UdpSocket},
	time::{Duration, Instant},
};

use betp::{packets::DisconnectPacket, server::Events};

use crate::network::{
	datagram::{DatagramOutcome, EncodedDatagram, MAX_BETP_DATAGRAM_SIZE, ServerDatagramPipeline},
	is_transient, random_salt,
};

/// The most peers the server tracks at once, counting handshakes still in progress.
const MAX_PEERS: usize = 64;
/// How long a peer may take to finish the handshake before its slot is freed.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// The `ServerCreateError` enum separates failures that callers can report while opening a server socket.
#[derive(Debug)]
pub enum ServerCreateError {
	Bind(std::io::Error),
	Configure(std::io::Error),
}

impl std::fmt::Display for ServerCreateError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Bind(error) => write!(
				formatter,
				"Failed to bind UDP server socket: {error}. The most likely cause is that another process already uses the port."
			),
			Self::Configure(error) => write!(
				formatter,
				"Failed to configure nonblocking UDP I/O: {error}. The most likely cause is that the operating system rejected the socket mode."
			),
		}
	}
}

impl std::error::Error for ServerCreateError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Bind(error) | Self::Configure(error) => Some(error),
		}
	}
}

/// The `Server` struct lets an application host one BETP session per client on a single UDP socket.
///
/// Create it with [`Self::bind`], then call [`Self::update`] once per application tick. After each update, handle
/// [`Self::drain_events`] and [`Self::drain_received`], and queue payloads with [`Self::send`] or
/// [`Self::send_to_client`]. Dropping the server tells every connected client that the session ended.
pub struct Server {
	socket: UdpSocket,
	peers: Vec<Peer>,
	events: Vec<Events>,
	received: Vec<(u64, [u8; 1024])>,
	outbound: Vec<EncodedDatagram>,
}

/// One remote address and the session it is negotiating or using.
struct Peer {
	address: SocketAddr,
	pipeline: ServerDatagramPipeline,
	opened: Instant,
	/// Set when the session times out, so the update forgets the peer once every peer has advanced.
	timed_out: bool,
}

impl Server {
	/// Opens a nonblocking server socket on `address`.
	///
	/// Bind port `0` to let the operating system pick a free port, and read it back with [`Self::local_addr`].
	/// Next, call [`Self::update`] on every application tick so clients can connect.
	pub fn bind<A: ToSocketAddrs>(address: A) -> Result<Self, ServerCreateError> {
		let socket = UdpSocket::bind(address).map_err(ServerCreateError::Bind)?;
		socket.set_nonblocking(true).map_err(ServerCreateError::Configure)?;

		Ok(Self {
			socket,
			peers: Vec::with_capacity(MAX_PEERS),
			events: Vec::new(),
			received: Vec::new(),
			outbound: Vec::new(),
		})
	}

	/// Returns the address clients connect to.
	pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
		self.socket.local_addr()
	}

	/// Reads every pending datagram, then advances retries, keepalives, and timeouts for each peer.
	///
	/// A client that sends nothing, not even a keepalive, for [`betp::TIMEOUT`] is disconnected.
	///
	/// Afterwards, [`Self::drain_events`] reports connected and disconnected clients and [`Self::drain_received`]
	/// returns their payloads. Returns an error only when the socket itself fails.
	pub fn update(&mut self, current_time: Instant) -> std::io::Result<()> {
		// One spare byte lets the pipeline reject an oversized datagram instead of reading a truncated one.
		let mut buffer = [0u8; MAX_BETP_DATAGRAM_SIZE + 1];
		loop {
			let (length, address) = match self.socket.recv_from(&mut buffer) {
				Ok(received) => received,
				Err(error) if error.kind() == ErrorKind::WouldBlock => break,
				Err(error) if is_transient(&error) => continue,
				Err(error) => return Err(error),
			};
			self.route(address, &buffer[..length], current_time)?;
		}

		for peer in &mut self.peers {
			let outcome = peer
				.pipeline
				.advance(current_time, &mut self.outbound)
				.map_err(std::io::Error::other)?;
			send_datagrams(&self.socket, peer.address, &self.outbound);
			if let DatagramOutcome::Disconnected { id } = outcome {
				// The client went quiet. Tell it the session ended in case it is still listening.
				send_disconnect(&self.socket, peer.address, id);
				self.events.push(Events::ClientDisconnected { id });
				peer.timed_out = true;
			}
		}

		self.peers.retain(|peer| {
			!peer.timed_out && (peer.pipeline.is_connected() || current_time.duration_since(peer.opened) < HANDSHAKE_TIMEOUT)
		});

		Ok(())
	}

	/// Feeds one datagram to the session of the address that sent it. A new address gets a session only when its
	/// datagram is a connection request, so stray traffic cannot hold client slots.
	fn route(&mut self, address: SocketAddr, datagram: &[u8], current_time: Instant) -> std::io::Result<()> {
		let (index, new) = match self.peers.iter().position(|peer| peer.address == address) {
			Some(index) => (index, false),
			None if self.peers.len() < MAX_PEERS => {
				self.peers.push(Peer {
					address,
					pipeline: ServerDatagramPipeline::new(random_salt()),
					opened: current_time,
					timed_out: false,
				});
				(self.peers.len() - 1, true)
			}
			None => {
				log::debug!("Ignored a datagram from {address} because every client slot is in use.");
				return Ok(());
			}
		};

		let peer = &mut self.peers[index];
		let outcome = peer
			.pipeline
			.process_datagram(datagram, current_time, &mut self.outbound)
			.map_err(std::io::Error::other)?;
		send_datagrams(&self.socket, address, &self.outbound);

		if let Some(id) = peer.pipeline.connection_id() {
			self.received.extend(peer.pipeline.drain_received().map(|data| (id, data)));
		}

		match outcome {
			DatagramOutcome::Connected { id } => self.events.push(Events::ClientConnected { id }),
			DatagramOutcome::Disconnected { id } => self.events.push(Events::ClientDisconnected { id }),
			DatagramOutcome::Handled => {}
			DatagramOutcome::Dropped(reason) => log::debug!("Dropped a datagram from {address}: {reason:?}"),
		}

		// A connection request is the only datagram that answers with `Handled` from a fresh session.
		if (new && outcome != DatagramOutcome::Handled) || matches!(outcome, DatagramOutcome::Disconnected { .. }) {
			self.peers.swap_remove(index);
		}

		Ok(())
	}

	/// Removes the connection events gathered by [`Self::update`] and [`Self::disconnect_client`].
	pub fn drain_events(&mut self) -> impl Iterator<Item = Events> + '_ {
		self.events.drain(..)
	}

	/// Removes every payload received since the last drain, each with the connection id of its sender. Each client's
	/// reliable payloads come in the order that client sent them.
	pub fn drain_received(&mut self) -> impl Iterator<Item = (u64, [u8; 1024])> + '_ {
		self.received.drain(..)
	}

	/// Returns how many clients finished the handshake and have not disconnected.
	pub fn connected_clients(&self) -> usize {
		self.connected().count()
	}

	/// Queues a payload for every connected client. The next [`Self::update`] sends it.
	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) {
		for peer in self.peers.iter_mut().filter(|peer| peer.pipeline.is_connected()) {
			peer.pipeline.send(reliable, data);
		}
	}

	/// Queues a payload for one connected client. Payloads for an unknown connection are discarded.
	pub fn send_to_client(&mut self, connection_id: u64, reliable: bool, data: [u8; 1024]) {
		if let Some(peer) = self
			.peers
			.iter_mut()
			.find(|peer| peer.pipeline.connection_id() == Some(connection_id))
		{
			peer.pipeline.send(reliable, data);
		}
	}

	/// Ends one client's session now and reports it through [`Self::drain_events`].
	pub fn disconnect_client(&mut self, connection_id: u64) {
		if let Some(index) = self
			.peers
			.iter()
			.position(|peer| peer.pipeline.connection_id() == Some(connection_id))
		{
			let peer = self.peers.swap_remove(index);
			send_disconnect(&self.socket, peer.address, connection_id);
			self.events.push(Events::ClientDisconnected { id: connection_id });
		}
	}

	fn connected(&self) -> impl Iterator<Item = (SocketAddr, u64)> + '_ {
		self.peers
			.iter()
			.filter_map(|peer| Some((peer.address, peer.pipeline.connection_id()?)))
	}
}

impl Drop for Server {
	/// Tells every connected client that the session ended, so none waits for a timeout.
	fn drop(&mut self) {
		for (address, id) in self.connected() {
			send_disconnect(&self.socket, address, id);
		}
	}
}

/// Sends one best-effort disconnect. A lost one leaves the peer to notice through its own timeout.
fn send_disconnect(socket: &UdpSocket, address: SocketAddr, connection_id: u64) {
	if let Ok(datagram) = EncodedDatagram::encode(DisconnectPacket::new(connection_id).into()) {
		send_datagrams(socket, address, std::slice::from_ref(&datagram));
	}
}

/// Sends datagrams to one peer best effort. UDP may lose any of them anyway, and BETP retries the reliable ones.
fn send_datagrams(socket: &UdpSocket, address: SocketAddr, datagrams: &[EncodedDatagram]) {
	for datagram in datagrams {
		let _ = socket.send_to(datagram.as_bytes(), address);
	}
}
