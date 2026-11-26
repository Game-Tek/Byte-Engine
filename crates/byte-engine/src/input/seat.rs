input_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, facet::Facet)]
	/// The `SeatHandle` struct identifies the player seat associated with input.
	SeatHandle
}

impl SeatHandle {
	/// Returns the placeholder seat used until platform input seats are wired through.
	pub fn stub() -> Self {
		Self::from_index(0)
	}
}
