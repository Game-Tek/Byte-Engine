//! Reliability policy for application messages sent over engine networking.
//!
//! Implement [`Replicable`] on payloads submitted to network sessions.
//! [`Importance::Essential`] retains retry behavior for state that must arrive;
//! use [`Importance::Optional`] for replaceable cosmetic or high-frequency data.

/// The [`Replicable`] trait supplies payload access and delivery importance to
/// network sessions.
pub trait Replicable {
	fn payload(&self) -> &u8;

	/// Returns the delivery importance of the message.
	///
	/// The default is [`Importance::Essential`], which retries delivery until the
	/// peer acknowledges it. Use [`Importance::Optional`] for replaceable data so
	/// essential messages retain bandwidth.
	fn importance(&self) -> Importance {
		Importance::Essential
	}
}

/// The [`Importance`] enum selects whether delivery should be retried.
pub enum Importance {
	Essential,
	Optional,
}
