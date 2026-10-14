//! UDP transport for a BETP client.

use std::{
	io::ErrorKind,
	net::{SocketAddr, UdpSocket},
	time::Instant,
};

use betp::packets::DisconnectPacket;

use crate::network::{
	datagram::{ClientDatagramPipeline, DatagramOutcome, EncodedDatagram, MAX_BETP_DATAGRAM_SIZE},
	is_transient, random_salt,
};

/// The `ClientCreateError` enum separates failures that callers can report during UDP setup.
#[derive(Debug)]
pub enum ClientCreateError {
	Bind(std::io::Error),
	Connect(std::io::Error),
	Configure(std::io::Error),
}

impl std::fmt::Display for ClientCreateError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Bind(error) => write!(
				formatter,
				"Failed to bind UDP client socket: {error}. The most likely cause is that the local network interface is unavailable."
			),
			Self::Connect(error) => write!(
				formatter,
				"Failed to configure UDP server peer: {error}. The most likely cause is that the server address is incompatible with the client socket."
			),
			Self::Configure(error) => write!(
				formatter,
				"Failed to configure nonblocking UDP I/O: {error}. The most likely cause is that the operating system rejected the socket mode."
			),
		}
	}
}

impl std::error::Error for ClientCreateError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Bind(error) | Self::Connect(error) | Self::Configure(error) => Some(error),
		}
	}
}

/// The `Client` struct lets an application join a BETP server over UDP.
///
/// Create it with [`Self::connect`], then call [`Self::update`] once per application tick. Once
/// [`Self::is_connected`] is `true`, queue payloads with [`Self::send`] and read the server's with
/// [`Self::drain_received`]. Dropping the client tells the server that the session ended.
pub struct Client {
	socket: UdpSocket,
	pipeline: ClientDatagramPipeline,
	outbound: Vec<EncodedDatagram>,
	/// Set once the server ends the session or times out, after which the client performs no more I/O.
	ended: bool,
}

impl Client {
	/// Binds an ephemeral local socket and starts the handshake with `server_address`.
	///
	/// Next, call [`Self::update`] on every application tick to finish the handshake.
	pub fn connect(server_address: SocketAddr) -> Result<Self, ClientCreateError> {
		let socket = UdpSocket::bind(local_bind_address(server_address)).map_err(ClientCreateError::Bind)?;
		Self::from_socket(socket, server_address)
	}

	/// Starts the handshake with `server_address` over a caller-bound socket.
	///
	/// Next, call [`Self::update`] on every application tick to finish the handshake.
	pub fn from_socket(socket: UdpSocket, server_address: SocketAddr) -> Result<Self, ClientCreateError> {
		socket.connect(server_address).map_err(ClientCreateError::Connect)?;
		socket.set_nonblocking(true).map_err(ClientCreateError::Configure)?;

		Ok(Self {
			socket,
			pipeline: ClientDatagramPipeline::new(random_salt()),
			outbound: Vec::new(),
			ended: false,
		})
	}

	/// Reads every pending datagram from the server, then sends handshake, payload, retry, and keepalive datagrams.
	///
	/// The session ends when the server disconnects or sends nothing for [`betp::TIMEOUT`].
	///
	/// Returns an error only when the socket itself fails. A server that is not listening yet is not an error: the
	/// client keeps asking until it answers.
	pub fn update(&mut self, current_time: Instant) -> std::io::Result<()> {
		if self.ended {
			return Ok(());
		}

		// One spare byte lets the pipeline reject an oversized datagram instead of reading a truncated one.
		let mut buffer = [0u8; MAX_BETP_DATAGRAM_SIZE + 1];
		loop {
			let length = match self.socket.recv(&mut buffer) {
				Ok(length) => length,
				Err(error) if error.kind() == ErrorKind::WouldBlock => break,
				Err(error) if is_transient(&error) => continue,
				Err(error) => return Err(error),
			};
			let outcome = self
				.pipeline
				.process_datagram(&buffer[..length], current_time, &mut self.outbound)
				.map_err(std::io::Error::other)?;
			self.send_outbound();

			match outcome {
				DatagramOutcome::Disconnected { .. } => {
					self.ended = true;
					return Ok(());
				}
				DatagramOutcome::Dropped(reason) => log::debug!("Dropped a datagram from the server: {reason:?}"),
				DatagramOutcome::Connected { .. } | DatagramOutcome::Handled => {}
			}
		}

		let outcome = self
			.pipeline
			.advance(current_time, &mut self.outbound)
			.map_err(std::io::Error::other)?;
		self.send_outbound();
		// The server went quiet for longer than the timeout.
		self.ended = matches!(outcome, DatagramOutcome::Disconnected { .. });

		Ok(())
	}

	/// Returns whether the handshake finished and the server has not ended the session.
	pub fn is_connected(&self) -> bool {
		!self.ended && self.pipeline.is_connected()
	}

	/// Returns the id the server knows this client by, once connected.
	pub fn connection_id(&self) -> Option<u64> {
		self.pipeline.connection_id().filter(|_| !self.ended)
	}

	/// Queues a payload for the server. The next [`Self::update`] sends it.
	///
	/// Reliable payloads arrive once and in the order they were queued. Payloads queued before [`Self::is_connected`]
	/// turns `true` go out once it does.
	pub fn send(&mut self, reliable: bool, data: [u8; 1024]) {
		self.pipeline.send(reliable, data);
	}

	/// Removes and returns every payload received since the last drain, reliable ones in the order they were sent.
	pub fn drain_received(&mut self) -> impl Iterator<Item = [u8; 1024]> + '_ {
		self.pipeline.drain_received()
	}

	fn send_outbound(&self) {
		for datagram in &self.outbound {
			// UDP may lose any datagram anyway, and BETP retries the reliable ones.
			let _ = self.socket.send(datagram.as_bytes());
		}
	}
}

impl Drop for Client {
	/// Tells the server that the session ended, so it frees the slot without waiting for a timeout.
	fn drop(&mut self) {
		if let Some(id) = self.connection_id()
			&& let Ok(datagram) = EncodedDatagram::encode(DisconnectPacket::new(id).into())
		{
			let _ = self.socket.send(datagram.as_bytes());
		}
	}
}

/// Selects an ephemeral wildcard address in the same address family as the server.
fn local_bind_address(server_address: SocketAddr) -> SocketAddr {
	match server_address {
		SocketAddr::V4(_) => SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0),
		SocketAddr::V6(_) => SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 0),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn client_bind_address_is_ephemeral_and_matches_the_server_family() {
		let ipv4_server = "127.0.0.1:6669".parse().expect("IPv4 server address should parse");
		let ipv6_server = "[::1]:6669".parse().expect("IPv6 server address should parse");

		assert_eq!(
			local_bind_address(ipv4_server),
			"0.0.0.0:0"
				.parse::<std::net::SocketAddr>()
				.expect("IPv4 wildcard address should parse")
		);
		assert_eq!(
			local_bind_address(ipv6_server),
			"[::]:0"
				.parse::<std::net::SocketAddr>()
				.expect("IPv6 wildcard address should parse")
		);
	}
}
