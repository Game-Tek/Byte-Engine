//! Compact identities for resources stored at zero-based indices.

/// Reserves zero for `None` without leaving an unused entry in resource storage.
macro_rules! index_handle {
	($(#[$attribute:meta])* $visibility:vis struct $name:ident;) => {
		$(#[$attribute])*
		#[repr(transparent)]
		$visibility struct $name(std::num::NonZeroU64);

		#[allow(dead_code, reason = "Not every backend creates or reads every handle by index.")]
		impl $name {
			/// Encodes a storage index as a nonzero identity.
			pub(crate) const fn from_index(index: u64) -> Self {
				Self(std::num::NonZeroU64::MIN.checked_add(index).expect(
					"Resource handle space is exhausted. The storage index reached u64::MAX. Reduce the number of stored resources.",
				))
			}

			/// Returns the zero-based storage index without exposing the encoded identity.
			pub(crate) const fn index(self) -> u64 {
				self.0.get() - 1
			}
		}

		const _: () = assert!(std::mem::size_of::<Option<$name>>() == std::mem::size_of::<u64>());
	};
}

pub(crate) use index_handle;
