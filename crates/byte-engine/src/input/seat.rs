#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// The `SeatHandle` struct identifies the player seat associated with input.
pub struct SeatHandle(pub(super) u32);

impl SeatHandle {
	/// Returns the placeholder seat used until platform input seats are wired through.
	pub fn stub() -> Self {
		Self(0)
	}
}
