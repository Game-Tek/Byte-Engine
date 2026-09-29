use std::{
	any::TypeId,
	fmt,
	mem::{MaybeUninit, align_of, size_of},
	ptr,
};

/// The `InlineCopyFnError` enum identifies callable layouts that [`InlineCopyFn`] cannot store inline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineCopyFnError {
	CaptureTooLarge { size: usize, max_size: usize },
	CaptureAlignmentTooLarge { align: usize, max_align: usize },
}

impl fmt::Display for InlineCopyFnError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::CaptureTooLarge { max_size, .. } => write!(
				f,
				"Closure capture is too large. The most likely cause is that the closure stores more than {max_size} bytes of captured state.",
			),
			Self::CaptureAlignmentTooLarge { max_align, .. } => write!(
				f,
				"Closure capture alignment is too large. The most likely cause is that the closure captures a value that requires alignment above {max_align} bytes.",
			),
		}
	}
}

impl std::error::Error for InlineCopyFnError {}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy)]
struct InlineStorage<const STORAGE_SIZE: usize> {
	bytes: [MaybeUninit<u8>; STORAGE_SIZE],
}

impl<const STORAGE_SIZE: usize> InlineStorage<STORAGE_SIZE> {
	const fn uninit() -> Self {
		Self {
			bytes: [MaybeUninit::uninit(); STORAGE_SIZE],
		}
	}

	fn write<T>(&mut self, value: T)
	where
		T: Copy,
	{
		// SAFETY: `InlineCopyFn::try_new` rejects values larger or more aligned than this storage before `write` is called.
		unsafe {
			ptr::write(self.bytes.as_mut_ptr().cast::<T>(), value);
		}
	}

	fn read<T>(&self) -> T
	where
		T: Copy,
	{
		// SAFETY: Each call shim requests the same `T` that `InlineCopyFn::try_new` initialized in this storage.
		unsafe { ptr::read(self.bytes.as_ptr().cast::<T>()) }
	}
}

/// The `InlineCopyFn` struct provides allocation-free type erasure for small, copyable one-argument callables.
///
/// Use it where a copyable value must hold a closure, such as a UI flow function, without boxing it.
pub struct InlineCopyFn<A0, Output, const STORAGE_SIZE: usize = 16> {
	storage: InlineStorage<STORAGE_SIZE>,
	/// The [`call_stored`] shim monomorphized for the stored callable's concrete type.
	call: fn(&InlineStorage<STORAGE_SIZE>, A0) -> Output,
	type_id: fn() -> TypeId,
}

// Manual impls avoid the `A0: Clone` and `Output: Clone` bounds a derive would add; only function pointers
// and plain bytes are copied.
impl<A0, Output, const STORAGE_SIZE: usize> Clone for InlineCopyFn<A0, Output, STORAGE_SIZE> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<A0, Output, const STORAGE_SIZE: usize> Copy for InlineCopyFn<A0, Output, STORAGE_SIZE> {}

impl<A0, Output, const STORAGE_SIZE: usize> fmt::Debug for InlineCopyFn<A0, Output, STORAGE_SIZE> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("InlineCopyFn")
			.field("callable_type_id", &self.callable_type_id())
			.finish_non_exhaustive()
	}
}

/// Calls the `F` stored in `storage`; [`InlineCopyFn::try_new`] stores this shim for its concrete `F`.
fn call_stored<F, A0, Output, const STORAGE_SIZE: usize>(storage: &InlineStorage<STORAGE_SIZE>, arg0: A0) -> Output
where
	F: Fn(A0) -> Output + Copy + 'static,
{
	let function = storage.read::<F>();
	function(arg0)
}

impl<A0, Output, const STORAGE_SIZE: usize> InlineCopyFn<A0, Output, STORAGE_SIZE> {
	/// Stores `value` inline, panicking when it does not fit. Use [`Self::try_new`] to handle that case.
	pub fn new<F>(value: F) -> Self
	where
		F: Fn(A0) -> Output + Copy + 'static,
	{
		Self::try_new(value).unwrap_or_else(|error| panic!("{error}"))
	}

	/// Stores `value` inline, or reports why its captures do not fit the storage.
	pub fn try_new<F>(value: F) -> Result<Self, InlineCopyFnError>
	where
		F: Fn(A0) -> Output + Copy + 'static,
	{
		if size_of::<F>() > STORAGE_SIZE {
			return Err(InlineCopyFnError::CaptureTooLarge {
				size: size_of::<F>(),
				max_size: STORAGE_SIZE,
			});
		}

		let max_align = align_of::<InlineStorage<STORAGE_SIZE>>();
		if align_of::<F>() > max_align {
			return Err(InlineCopyFnError::CaptureAlignmentTooLarge {
				align: align_of::<F>(),
				max_align,
			});
		}

		let mut storage = InlineStorage::uninit();
		storage.write(value);

		Ok(Self {
			storage,
			call: call_stored::<F, A0, Output, STORAGE_SIZE>,
			type_id: TypeId::of::<F>,
		})
	}

	/// Returns the concrete callable's type, independently of its captured values.
	pub fn callable_type_id(&self) -> TypeId {
		(self.type_id)()
	}

	/// Calls the stored callable with `arg0`.
	pub fn call(&self, arg0: A0) -> Output {
		(self.call)(&self.storage, arg0)
	}
}

#[cfg(test)]
mod tests {
	use super::{InlineCopyFn, InlineCopyFnError};

	#[test]
	fn stores_small_capturing_closures_and_supports_copying() {
		let a = 3u64;
		let b = 7u64;
		let closure = move |value| value + a + b;
		let function = InlineCopyFn::<u64, u64>::new(closure);
		let copied = function;
		let cloned = function;

		assert_eq!(copied.call(1), 11);
		assert_eq!(cloned.call(5), 15);
		assert_eq!(copied.callable_type_id(), std::any::Any::type_id(&closure));
	}

	#[test]
	fn rejects_large_closure_captures() {
		let data = [1u64, 2, 3];
		let error = InlineCopyFn::<u64, u64>::try_new(move |value| value + data.into_iter().sum::<u64>()).unwrap_err();

		assert_eq!(
			error,
			InlineCopyFnError::CaptureTooLarge {
				size: std::mem::size_of::<[u64; 3]>(),
				max_size: 16,
			}
		);
	}

	#[test]
	fn rejects_alignment_that_does_not_fit_inline_storage() {
		#[repr(align(32))]
		#[derive(Clone, Copy)]
		struct Aligned(u8);

		fn read(aligned: Aligned, offset: u8) -> u8 {
			aligned.0 + offset
		}

		let aligned = Aligned(1);
		let error = InlineCopyFn::<u8, u8, 64>::try_new(move |offset| read(aligned, offset)).unwrap_err();

		assert_eq!(
			error,
			InlineCopyFnError::CaptureAlignmentTooLarge {
				align: std::mem::align_of::<Aligned>(),
				max_align: 16,
			}
		);
	}
}
