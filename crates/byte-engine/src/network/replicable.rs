//! Reliability policy for application messages sent over engine networking.
//!
//! Implement [`Replicable`] on payloads submitted to network sessions.
//! [`Importance::Essential`] retains retry behavior for state that must arrive;
//! use [`Importance::Optional`] for replaceable cosmetic or high-frequency data.

/// The [`Replicable`] trait supplies payload access and delivery importance to
/// network sessions.
pub trait Replicable {
	fn payload(&self) -> &u8;

	/// Returns the improtance of a message. By default all messages will be retried until succesfully acknowledged unless a lower importance is specified.
	/// Using lower importacnes for non-critical messages such as cosmetic events can free up bandwidth for essentail messages such as input events.
	fn importance(&self) -> Importance {
		Importance::Essential
	}
}

/// The [`Importance`] enum selects whether delivery should be retried.
pub enum Importance {
	Essential,
	Optional,
}
