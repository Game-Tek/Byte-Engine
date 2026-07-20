//! Defines the client-facing BETP connection API.

/// The `Client` trait provides application-level control of a BETP server connection.
///
/// Call [`Self::connect`], then call [`Self::update`] periodically. Queue payloads
/// with [`Self::send`] and finish the session with [`Self::disconnect`].
pub trait Client {
	/// Queues a connection attempt to the server.
	///
	/// Call [`Client::update`] to send the negotiation packets. This method has no
	/// effect when the client is already connected.
	fn connect(&mut self, current_time: std::time::Instant) -> ();

	/// Queues a data packet for the next call to [`Client::update`].
	///
	/// Next, call [`Client::update`] to perform the network I/O.
	fn send(&mut self, reliable: bool, data: [u8; 1024]) -> Result<(), Errors>;

	/// Queues a voluntary disconnect for the next call to [`Client::update`].
	///
	/// Call [`Client::update`] once more to send the disconnect packet.
	fn disconnect(&mut self) -> Result<(), Errors>;

	/// Receives new packets and sends queued packets.
	///
	/// Call this method periodically to maintain the connection. This is the only
	/// method on this trait that performs I/O.
	fn update(&mut self) -> Result<(), Errors>;
}

use crate::client::Errors;
