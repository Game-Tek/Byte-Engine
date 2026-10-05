//! Serde and rkyv support for math types, stored as plain `f32` arrays.
//!
//! Every serializable type is stored as an `f32` array, so serialized and archived data looks exactly like the raw
//! arrays resources stored before they used math types. Loading validates the array, so data that breaks a type's
//! invariant, such as a zero-length orientation, fails to deserialize instead of producing an invalid value.

/// Implements serde and rkyv for `$type` by storing the `f32` array that its `$to` method returns.
///
/// Pass `from:` with a conversion that accepts every array, or `try_from:` with one that validates the array and
/// returns a `Result`. List the type parameters of `$type` last.
macro_rules! serialize_as_array {
	($type:ty, [$element:ty; $count:literal], $to:ident, from: $from:expr $(, $generic:ident)*) => {
		$crate::serialization::serialize_as_array!(
			$type, [$element; $count], $to, try_from: |array| Ok::<_, std::convert::Infallible>(($from)(array)) $(, $generic)*
		);
	};
	($type:ty, [$element:ty; $count:literal], $to:ident, try_from: $try_from:expr $(, $generic:ident)*) => {
		impl<$($generic),*> serde::Serialize for $type {
			fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
				serde::Serialize::serialize(&self.$to(), serializer)
			}
		}

		impl<'de, $($generic),*> serde::Deserialize<'de> for $type {
			fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
				let array: [$element; $count] = serde::Deserialize::deserialize(deserializer)?;
				($try_from)(array).map_err(serde::de::Error::custom)
			}
		}

		impl<$($generic),*> rkyv::Archive for $type {
			type Archived = [rkyv::Archived<$element>; $count];
			type Resolver = [rkyv::Resolver<$element>; $count];

			fn resolve(&self, resolver: Self::Resolver, out: rkyv::Place<Self::Archived>) {
				rkyv::Archive::resolve(&self.$to(), resolver, out);
			}
		}

		impl<$($generic,)* S: rkyv::rancor::Fallible + ?Sized> rkyv::Serialize<S> for $type {
			fn serialize(&self, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
				rkyv::Serialize::serialize(&self.$to(), serializer)
			}
		}

		// The archived array is spelled out, because coherence cannot see through an `rkyv::Archived` projection and
		// would report a conflict with rkyv's blanket `Deserialize` impl for `With`.
		impl<$($generic,)* D> rkyv::Deserialize<$type, D> for [rkyv::Archived<$element>; $count]
		where
			D: rkyv::rancor::Fallible + ?Sized,
			D::Error: rkyv::rancor::Source,
		{
			fn deserialize(&self, deserializer: &mut D) -> Result<$type, D::Error> {
				let array: [$element; $count] = rkyv::Deserialize::deserialize(self, deserializer)?;
				($try_from)(array).map_err(rkyv::rancor::Source::new)
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
