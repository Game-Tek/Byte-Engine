use std::{
	fmt,
	marker::PhantomData,
	mem::{align_of, size_of, MaybeUninit},
	ptr,
};

/// The `InlineCopyFnError` enum reports why an erased callable could not fit in the inline container.
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
		unsafe {
			ptr::write(self.bytes.as_mut_ptr().cast::<T>(), value);
		}
	}

	fn read<T>(&self) -> T
	where
		T: Copy,
	{
		unsafe { ptr::read(self.bytes.as_ptr().cast::<T>()) }
	}
}

/// The `InlineCopyFn` struct stores a small copyable callable inline while hiding its concrete type.
#[derive(Debug, Clone, Copy)]
pub struct InlineCopyFn<Signature, const STORAGE_SIZE: usize = 16> {
	storage: InlineStorage<STORAGE_SIZE>,
	call: *const (),
	_signature: PhantomData<Signature>,
}

impl<Signature, const STORAGE_SIZE: usize> InlineCopyFn<Signature, STORAGE_SIZE> {
	/// Validates whether a callable can be stored inline.
	fn validate<F>() -> Result<(), InlineCopyFnError> {
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

		Ok(())
	}

	fn store<F>(value: F, call: *const ()) -> Result<Self, InlineCopyFnError>
	where
		F: Copy + 'static,
	{
		Self::validate::<F>()?;

		let mut storage = InlineStorage::uninit();
		storage.write(value);

		Ok(Self {
			storage,
			call,
			_signature: PhantomData,
		})
	}
}

macro_rules! impl_inline_copy_fn {
	($call_impl:ident, (), ()) => {
		fn $call_impl<F, Output, const STORAGE_SIZE: usize>(storage: &InlineStorage<STORAGE_SIZE>) -> Output
		where
			F: Fn() -> Output + Copy + 'static,
		{
			let function = storage.read::<F>();
			function()
		}

		impl<Output, const STORAGE_SIZE: usize> InlineCopyFn<fn() -> Output, STORAGE_SIZE> {
			pub fn new<F>(value: F) -> Self
			where
				F: Fn() -> Output + Copy + 'static,
			{
				Self::try_new(value).unwrap_or_else(|error| panic!("{error}"))
			}

			pub fn try_new<F>(value: F) -> Result<Self, InlineCopyFnError>
			where
				F: Fn() -> Output + Copy + 'static,
			{
				Self::store(value, $call_impl::<F, Output, STORAGE_SIZE> as *const ())
			}

			pub fn call(&self) -> Output {
				let call = unsafe {
					std::mem::transmute::<*const (), fn(&InlineStorage<STORAGE_SIZE>) -> Output>(self.call)
				};
				call(&self.storage)
			}
		}
	};
	($call_impl:ident, ($($arg:ident),+), ($($value:ident),+)) => {
		fn $call_impl<F, $($arg,)+ Output, const STORAGE_SIZE: usize>(
			storage: &InlineStorage<STORAGE_SIZE>,
			$($value: $arg),+
		) -> Output
		where
			F: Fn($($arg),+) -> Output + Copy + 'static,
		{
			let function = storage.read::<F>();
			function($($value),+)
		}

		impl<$($arg,)+ Output, const STORAGE_SIZE: usize> InlineCopyFn<fn($($arg),+) -> Output, STORAGE_SIZE> {
			pub fn new<F>(value: F) -> Self
			where
				F: Fn($($arg),+) -> Output + Copy + 'static,
			{
				Self::try_new(value).unwrap_or_else(|error| panic!("{error}"))
			}

			pub fn try_new<F>(value: F) -> Result<Self, InlineCopyFnError>
			where
				F: Fn($($arg),+) -> Output + Copy + 'static,
			{
				Self::store(value, $call_impl::<F, $($arg,)+ Output, STORAGE_SIZE> as *const ())
			}

			pub fn call(&self, $($value: $arg),+) -> Output {
				let call = unsafe {
					std::mem::transmute::<*const (), fn(&InlineStorage<STORAGE_SIZE>, $($arg),+) -> Output>(self.call)
				};
				call(&self.storage, $($value),+)
			}
		}
	};
}

impl_inline_copy_fn!(call0, (), ());
impl_inline_copy_fn!(call1, (A0), (arg0));
impl_inline_copy_fn!(call2, (A0, A1), (arg0, arg1));
impl_inline_copy_fn!(call3, (A0, A1, A2), (arg0, arg1, arg2));

#[cfg(test)]
mod tests {
	use super::{InlineCopyFn, InlineCopyFnError};

	#[test]
	fn stores_function_items_inline() {
		fn add(a: u32, b: u32) -> u32 {
			a + b
		}

		let function = InlineCopyFn::<fn(u32, u32) -> u32>::new(add);

		assert_eq!(function.call(2, 3), 5);
	}

	#[test]
	fn stores_small_capturing_closures_and_supports_copying() {
		let a = 3u64;
		let b = 7u64;
		let function = InlineCopyFn::<fn(u64) -> u64>::new(move |value| value + a + b);
		let copied = function;
		let cloned = function.clone();

		assert_eq!(copied.call(1), 11);
		assert_eq!(cloned.call(5), 15);
	}

	#[test]
	fn rejects_large_closure_captures() {
		let data = [1u64, 2, 3];
		let error = InlineCopyFn::<fn() -> u64>::try_new(move || data.into_iter().sum::<u64>()).unwrap_err();

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

		fn read(aligned: Aligned) -> u8 {
			aligned.0
		}

		let aligned = Aligned(1);
		let error = InlineCopyFn::<fn() -> u8, 64>::try_new(move || read(aligned)).unwrap_err();

		assert_eq!(
			error,
			InlineCopyFnError::CaptureAlignmentTooLarge {
				align: std::mem::align_of::<Aligned>(),
				max_align: 16,
			}
		);
	}
}
