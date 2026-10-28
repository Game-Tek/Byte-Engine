#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, facet::Facet)]
/// The `SeatHandle` struct identifies the player seat associated with input.
#[repr(transparent)]
pub struct SeatHandle(pub(super) std::num::NonZeroU32);

impl SeatHandle {
	/// Encodes a bounded registration index as a nonzero identity.
	pub(super) fn from_index(index: u32) -> Self {
		assert!(
			index < super::HANDLE_TAG.get(),
			"Input handle capacity exceeded. A local index exhausted its 31-bit space. Reuse existing input declarations."
		);
		Self(super::HANDLE_TAG | index)
	}

	pub(super) const fn index(self) -> u32 {
		self.0.get() ^ super::HANDLE_TAG.get()
	}
}

const _: () = assert!(size_of::<Option<SeatHandle>>() == size_of::<u32>());

impl SeatHandle {
	/// Returns the placeholder seat used until platform input seats are wired through.
	pub fn stub() -> Self {
		Self::from_index(0)
	}
}
