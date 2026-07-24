//! UDP transport for a BETP client.

use betp::{client::Session, write_packet};

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

/// The `Client` struct connects one BETP client session to a UDP server endpoint.
pub struct Client {
	session: Session,
	socket: std::net::UdpSocket,
}

impl Client {
	/// Creates a client for the specified server address.
	///
	/// Call `connect` before exchanging application data.
	pub fn new(server_address: std::net::SocketAddr) -> Result<Self, ClientCreateError> {
		let local_address = local_bind_address(server_address);
		let socket = std::net::UdpSocket::bind(local_address).map_err(ClientCreateError::Bind)?;
		Self::from_socket(socket, server_address)
	}

	/// Configures a caller-bound socket for nonblocking datagrams to one server peer.
	pub fn from_socket(socket: std::net::UdpSocket, server_address: std::net::SocketAddr) -> Result<Self, ClientCreateError> {
		socket.connect(server_address).map_err(ClientCreateError::Connect)?;
		socket.set_nonblocking(true).map_err(ClientCreateError::Configure)?;

		let session = Session::new();

		Ok(Self { socket, session })
	}
}

/// Selects an ephemeral wildcard address in the same address family as the server.
fn local_bind_address(server_address: std::net::SocketAddr) -> std::net::SocketAddr {
	match server_address {
		std::net::SocketAddr::V4(_) => std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0),
		std::net::SocketAddr::V6(_) => std::net::SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 0),
	}
}

impl betp::Client for Client {
	fn connect(&mut self, current_time: std::time::Instant) {
		let salt = current_time.elapsed().as_nanos() as u64;

		self.session.connect(salt);
	}

	fn update(&mut self) -> Result<(), betp::client::Errors> {
		let socket = &mut self.socket;

		let mut buffer = [0u8; 1024];

		let bytes_read = socket.recv(&mut buffer).map_err(|_| betp::client::Errors::IoError)?;

		let session = &mut self.session;

		let packets = [];

		let packets_to_send = session.update(&packets)?;

		for packet in packets_to_send {
			let mut buffer = [0u8; 1024];

			write_packet(&mut buffer, packet);

			let bytes_sent = socket.send(&buffer).map_err(|_| ());
		}

		Ok(())
	}

	fn send(&mut self, reliable: bool, data: [u8; 1024]) -> Result<(), betp::client::Errors> {
		let sesion = &mut self.session;

		sesion.send(reliable, data);

		Ok(())
	}

	fn disconnect(&mut self) -> Result<(), betp::client::Errors> {
		self.session.disconnect();

		Ok(())
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

	#[test]
	fn client_binds_an_ephemeral_address_and_targets_the_server() {
		let server = match std::net::UdpSocket::bind("127.0.0.1:0") {
			Ok(server) => server,
			// Some restricted test runners prohibit networking entirely. The pure address-selection
			// test above still verifies the regression there, while normal CI exercises real I/O.
			Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
				eprintln!("skipping UDP I/O assertion because this runner prohibits loopback sockets: {error}");
				return;
			}
			Err(error) => panic!("test server socket should bind: {error}"),
		};
		server
			.set_read_timeout(Some(std::time::Duration::from_secs(1)))
			.expect("test server timeout should configure");
		let server_address = server.local_addr().expect("test server should expose its address");

		let client = Client::new(server_address).expect("UDP client should bind and target the test server");
		let client_address = client
			.socket
			.local_addr()
			.expect("UDP client should expose its local address");

		assert_ne!(client_address, server_address);
		assert_ne!(client_address.port(), 0);
		assert_eq!(
			client.socket.peer_addr().expect("UDP client should have a peer"),
			server_address
		);

		client
			.socket
			.send(b"probe")
			.expect("UDP client should send to its configured peer");
		let mut buffer = [0; 5];
		let (received, source) = server
			.recv_from(&mut buffer)
			.expect("test server should receive the client datagram");

		assert_eq!(received, buffer.len());
		assert_eq!(&buffer, b"probe");
		assert_eq!(source, client_address);
	}
}
