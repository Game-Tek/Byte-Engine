use crate::server::Events;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionResults {
	ServerFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketHandlingResults {
	Undefined,
	ClientNotFound,
	BadConnectionId,
	UnhandleablePacket,
}

pub struct Settings {
	pub max_clients: usize,
	pub timeout: std::time::Duration,
}

/// A BETP authoritative server.
pub trait Server {
	/// Runs periodic updates on the server.
	/// Performs the following tasks:
	/// - Disconnects clients timed out clients.
	/// - Gathers unacknowledged packets to retry. This will count as a retry attempt.
	///
	/// - `current_time`: The current time.
	/// Returns a list of packets to send to the clients.
	/// Returns an error if the server encountered an error.
	///
	/// This function should be called periodically.
	fn update(&mut self, current_time: std::time::Instant) -> Result<Vec<Events>, ConnectionResults>;

	/// Send a message to all connected clients.
	fn send(&mut self, reliable: bool, data: [u8; 1024]);

	/// Send a message to particular client.
	fn send_to_client(&mut self, connection_id: u64, reliable: bool, data: [u8; 1024]);

	/// Disconnect all clients / disconnect self.
	fn disconnect(&mut self);

	/// Disconnect a particular client.
	fn disconnect_client(&mut self, connection_id: u64);
}
