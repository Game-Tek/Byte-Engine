pub trait Replicable {
	fn payload(&self) -> &u8;

	/// Returns the improtance of a message. By default all messages will be retried until succesfully acknowledged unless a lower importance is specified.
	/// Using lower importacnes for non-critical messages such as cosmetic events can free up bandwidth for essentail messages such as input events.
	fn importance(&self) -> Importance {
		Importance::Essential
	}
}

pub enum Importance {
	Essential,
	Optional,
}
