//! Client module for the Byte-Engine networking library.
//! The client is the entity that connects to a server and participates in the game.

use crate::client::Errors;

/// The client trait describes a connection to the server.
/// The client obeys the BETP.
/// For more information on how to operate a connection look to the `Session` implementation.
pub trait Client {
	/// Initiates a connection to the server.
	/// The underlying link (such as a UDP "connection") may already be established but the BETP session still has to be started.
	/// Will do nothing if already connected.
	/// The actual negotiantion won't be started until update is called and the connection packets get sent.
	fn connect(&mut self, current_time: std::time::Instant) -> ();

	/// Sends a data packet.
	/// The actual message won't be sent until update is called.
	fn send(&mut self, reliable: bool, data: [u8; 1024]) -> Result<(), ()>;

	/// Initiates a voluntary disconnect from the server.
	/// The actual message won't be sent until update is called.
	fn disconnect(&mut self) -> Result<(), ()>;

	/// Reads new messages and send pending ones to the server.
	/// This is the only method that performs I/O operations.
	/// Must be called periodically to keep the connection alive and process incoming messages.
	fn update(&mut self) -> Result<(), Errors>;
}
