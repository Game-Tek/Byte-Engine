//! Serde and rkyv support for math types, stored as plain `f32` arrays.
//!
//! Every serializable type is stored as the components of its [`ArrayForm::Array`], so serialized and archived data
//! looks exactly like the raw arrays resources stored before they used math types. Loading validates the array
//! through [`ArrayForm::try_from_array`], so data that breaks a type's invariant, such as a zero-length orientation,
//! fails to deserialize instead of producing an invalid value.

use rkyv::rancor::Fallible;

/// The `ArrayForm` trait converts a math type to and from the `f32` array it is stored as.
pub(crate) trait ArrayForm<const N: usize>: Sized {
	type Array: FlatArray<N>;
	type Error: std::error::Error + Send + Sync + 'static;

	fn to_array(&self) -> Self::Array;

	fn try_from_array(array: Self::Array) -> Result<Self, Self::Error>;
}

/// The `FlatArray` trait lays an `f32` array of any nesting out as its `N` components in memory order.
pub(crate) trait FlatArray<const N: usize>: Sized {
	fn flatten(self) -> [f32; N];

	fn unflatten(components: [f32; N]) -> Self;
}

impl<const N: usize> FlatArray<N> for [f32; N] {
	fn flatten(self) -> [f32; N] {
		self
	}

	fn unflatten(components: [f32; N]) -> Self {
		components
	}
}

macro_rules! nested_flat_array {
	($rows:literal, $columns:literal, $count:literal) => {
		impl FlatArray<$count> for [[f32; $columns]; $rows] {
			fn flatten(self) -> [f32; $count] {
				std::array::from_fn(|index| self[index / $columns][index % $columns])
			}

			fn unflatten(components: [f32; $count]) -> Self {
				std::array::from_fn(|row| std::array::from_fn(|column| components[row * $columns + column]))
			}
		}
	};
}

nested_flat_array!(2, 3, 6);
nested_flat_array!(4, 3, 12);

/// The `ArchivedFloats` struct is the archived form of a math type: its `N` components in memory order.
///
/// It has the same bytes as an archived `[f32; N]`, so resources archived as raw arrays still load.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArchivedFloats<const N: usize>([rkyv::Archived<f32>; N]);

impl<const N: usize> ArchivedFloats<N> {
	/// Returns the archived components in memory order.
	pub fn get(&self) -> [f32; N] {
		self.0.map(|component| component.to_native())
	}
}

// SAFETY: `ArchivedFloats` is a transparent wrapper around an array of portable archived floats.
unsafe impl<const N: usize> rkyv::Portable for ArchivedFloats<N> {}

// SAFETY: `ArchivedFloats` is a transparent wrapper around its array, so validating the array validates the wrapper.
unsafe impl<C: Fallible + ?Sized, const N: usize> rkyv::bytecheck::CheckBytes<C> for ArchivedFloats<N>
where
	[rkyv::Archived<f32>; N]: rkyv::bytecheck::CheckBytes<C>,
{
	unsafe fn check_bytes(value: *const Self, context: &mut C) -> Result<(), C::Error> {
		// SAFETY: The caller guarantees `value` points to readable memory the size of `Self`, which is the array's size.
		unsafe { <[rkyv::Archived<f32>; N] as rkyv::bytecheck::CheckBytes<C>>::check_bytes(value.cast(), context) }
	}
}

/// Implements serde and rkyv for a type through its [`ArrayForm`] with `N` components.
macro_rules! serialize_as_array {
	($type:ty, $components:literal $(, $generic:ident)*) => {
		impl<$($generic),*> serde::Serialize for $type {
			fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
				serde::Serialize::serialize(&$crate::serialization::ArrayForm::<$components>::to_array(self), serializer)
			}
		}

		impl<'de, $($generic),*> serde::Deserialize<'de> for $type {
			fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
				let array = serde::Deserialize::deserialize(deserializer)?;
				$crate::serialization::ArrayForm::<$components>::try_from_array(array).map_err(serde::de::Error::custom)
			}
		}

		impl<$($generic),*> rkyv::Archive for $type {
			type Archived = $crate::serialization::ArchivedFloats<$components>;
			type Resolver = [(); $components];

			fn resolve(&self, resolver: Self::Resolver, out: rkyv::Place<Self::Archived>) {
				let components = $crate::serialization::FlatArray::flatten($crate::serialization::ArrayForm::<$components>::to_array(self));
				// SAFETY: `ArchivedFloats` is a transparent wrapper around the archived array, so both share one layout.
				let out = unsafe { out.cast_unchecked::<rkyv::Archived<[f32; $components]>>() };
				rkyv::Archive::resolve(&components, resolver, out);
			}
		}

		impl<$($generic,)* S: rkyv::rancor::Fallible + ?Sized> rkyv::Serialize<S> for $type {
			fn serialize(&self, _serializer: &mut S) -> Result<Self::Resolver, S::Error> {
				Ok([(); $components])
			}
		}

		impl<$($generic,)* D> rkyv::Deserialize<$type, D> for $crate::serialization::ArchivedFloats<$components>
		where
			D: rkyv::rancor::Fallible + ?Sized,
			D::Error: rkyv::rancor::Source,
		{
			fn deserialize(&self, _deserializer: &mut D) -> Result<$type, D::Error> {
				let array = $crate::serialization::FlatArray::unflatten(self.get());
				<$type as $crate::serialization::ArrayForm<$components>>::try_from_array(array).map_err(rkyv::rancor::Source::new)
			}
		}
	};
}

pub(crate) use serialize_as_array;

#[cfg(test)]
mod tests {
	use crate::{AffineMatrix, Orientation, Point};

	#[test]
	fn archives_match_the_raw_arrays_they_replace() {
		let point = rkyv::to_bytes::<rkyv::rancor::Error>(&Point::<crate::WorldSpace>::new(1.0, 2.0, 3.0)).unwrap();
		let array = rkyv::to_bytes::<rkyv::rancor::Error>(&[1.0f32, 2.0, 3.0]).unwrap();
		assert_eq!(point.as_slice(), array.as_slice());

		let columns = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0], [10.0, 11.0, 12.0]];
		let matrix = rkyv::to_bytes::<rkyv::rancor::Error>(&AffineMatrix::from_columns(columns)).unwrap();
		let loaded = rkyv::from_bytes::<AffineMatrix, rkyv::rancor::Error>(&matrix).unwrap();
		assert_eq!(loaded.columns(), columns);
	}

	#[test]
	fn loading_rejects_arrays_that_break_the_type_invariant() {
		let zero = rkyv::to_bytes::<rkyv::rancor::Error>(&[0.0f32; 4]).unwrap();

		assert!(rkyv::from_bytes::<Orientation, rkyv::rancor::Error>(&zero).is_err());
	}
}
