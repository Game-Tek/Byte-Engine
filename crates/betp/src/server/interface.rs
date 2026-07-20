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

/// The `Server` trait provides application-level control of an authoritative BETP endpoint.
pub trait Server {
	/// Updates connection state and returns events for the application to handle.
	///
	/// This method disconnects timed-out clients and gathers unacknowledged packets
	/// for retry. Each gathered packet consumes one retry attempt.
	///
	/// Call this method periodically with the current time.
	fn update(&mut self, current_time: std::time::Instant) -> Result<Vec<Events>, ConnectionResults>;

	/// Sends a message to all connected clients.
	fn send(&mut self, reliable: bool, data: [u8; 1024]);

	/// Sends a message to one client.
	fn send_to_client(&mut self, connection_id: u64, reliable: bool, data: [u8; 1024]);

	/// Disconnects all clients and stops the server connection.
	fn disconnect(&mut self);

	/// Disconnects one client.
	fn disconnect_client(&mut self, connection_id: u64);
}

use crate::server::Events;
