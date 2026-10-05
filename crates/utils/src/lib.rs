//! Shared allocation, callable, collection, and data-conversion utilities for Byte-Engine crates.

#![feature(allocator_api)]

pub type BoxedFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

pub mod sync;

pub mod r#async;
pub mod availability_graph;
pub mod bit_array;
pub mod copy_fn;
pub mod hex;
pub mod range_allocator;
pub mod smoothed_value;
pub mod stable_vec;

/// The base of every link to the online documentation. Set `BYTE_ENGINE_DOCS_BASE_URL` at build time to link elsewhere.
const ONLINE_DOCS_BASE_URL: &str = match option_env!("BYTE_ENGINE_DOCS_BASE_URL") {
	Some(url) => url,
	None => "https://byte-engine.0x44491229.dev/docs",
};

/// Builds a link to one online documentation page, such as the recovery guide an error message points to.
pub fn online_docs_url(path: &str) -> String {
	format!(
		"{}/{}",
		ONLINE_DOCS_BASE_URL.trim_end_matches('/'),
		path.trim_start_matches('/')
	)
}

pub type Box<T> = smallbox::SmallBox<T, [u8; 32]>;
pub use availability_graph::{AvailabilityGraph, AvailabilityGraphError, AvailabilityHandle};
pub use copy_fn::{InlineCopyFn, InlineCopyFnError};
/// Fast in-memory hashing for engine collections.
///
/// Use [`hash::HashMap`] or [`hash::HashSet`] for global allocations. For a custom
/// allocator, pass [`hash::FxBuildHasher`] to an allocator-aware collection.
pub mod hash {
	pub use rustc_hash::{FxBuildHasher, FxHashMap as HashMap, FxHashSet as HashSet, FxHasher};
}
pub use range_allocator::RangeAllocator;
pub use sonic_rs as json;
pub use stable_vec::{StableVec, StableVecHandle};
pub struct BufferAllocator<'a> {
	buffer: &'a mut [u8],
	offset: usize,
}

impl<'a> BufferAllocator<'a> {
	pub fn new(buffer: &'a mut [u8]) -> Self {
		Self { buffer, offset: 0 }
	}

	pub fn take(&mut self, size: usize) -> &'a mut [u8] {
		self.take_with_offset(size).1
	}

	pub fn take_with_offset(&mut self, size: usize) -> (usize, &'a mut [u8]) {
		let offset = self.offset;
		let buffer = &mut self.buffer[self.offset..][..size];
		self.offset += size;
		// SAFETY: We know that the buffer is valid for the lifetime of the splitter.
		(offset, unsafe { std::mem::transmute::<&mut [u8], &'a mut [u8]>(buffer) })
	}

	pub fn take_with_offset_aligned(&mut self, size: usize, alignment: usize) -> (usize, &'a mut [u8]) {
		self.offset = self.offset.next_multiple_of(alignment.max(1));
		self.take_with_offset(size)
	}
}

/// The `Extent` struct represents the size and dimensionality of a region.
///
/// A zero marks an unused trailing dimension. Use [`Self::line`] for one-dimensional
/// regions, [`Self::rectangle`] or [`Self::square`] for two-dimensional regions,
/// and [`Self::cube`] when all three dimensions are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Extent {
	width: u32,
	height: u32,
	depth: u32,
}

impl Extent {
	/// Creates an extent from explicit axis sizes.
	///
	/// Set each unused trailing dimension to zero. Prefer [`Self::line`],
	/// [`Self::rectangle`], or [`Self::cube`] when the dimensionality is known.
	pub fn new(width: u32, height: u32, depth: u32) -> Self {
		Self { width, height, depth }
	}

	/// Creates a one-dimensional extent.
	pub fn line(width: u32) -> Self {
		Self {
			width,
			height: 0,
			depth: 0,
		}
	}

	/// Creates a square two-dimensional extent.
	pub const fn square(size: u32) -> Self {
		Self {
			width: size,
			height: size,
			depth: 0,
		}
	}

	/// Creates a rectangular two-dimensional extent.
	pub const fn rectangle(width: u32, height: u32) -> Self {
		Self { width, height, depth: 0 }
	}

	/// Creates a three-dimensional extent.
	pub fn cube(width: u32, height: u32, depth: u32) -> Self {
		Self { width, height, depth }
	}

	pub fn as_tuple(&self) -> (u32, u32, u32) {
		(self.width, self.height, self.depth)
	}

	pub fn as_array(&self) -> [u32; 3] {
		[self.width, self.height, self.depth]
	}

	#[inline]
	pub fn width(&self) -> u32 {
		self.width
	}
	#[inline]
	pub fn height(&self) -> u32 {
		self.height
	}
	#[inline]
	pub fn depth(&self) -> u32 {
		self.depth
	}

	pub fn aspect_ratio(&self) -> f32 {
		(self.width as f32) / (self.height as f32)
	}

	/// Returns the dimensions of mip `level`, halving each axis per level and keeping it at least one.
	///
	/// An unused height or depth stays zero, so the mip keeps the image's dimensionality.
	pub fn mip(self, level: u32) -> Self {
		let shrink = |size: u32| size.checked_shr(level).unwrap_or(0).max(1);
		Self {
			width: shrink(self.width),
			height: if self.height == 0 { 0 } else { shrink(self.height) },
			depth: if self.depth == 0 { 0 } else { shrink(self.depth) },
		}
	}

	/// Divides a two-dimensional extent for a reduced-resolution image, keeping each side at least one.
	///
	/// A `divisor` of `2` gives a half-resolution image.
	pub fn scaled_down(self, divisor: u32) -> Self {
		Self::rectangle((self.width / divisor).max(1), (self.height / divisor).max(1))
	}

	/// Returns the number of active axes encoded by nonzero dimensions.
	pub fn dimensions(&self) -> u32 {
		if self.width == 0 {
			0
		} else if self.depth != 0 {
			3
		} else if self.height != 0 {
			2
		} else {
			1
		}
	}
}

impl From<[u32; 3]> for Extent {
	fn from(array: [u32; 3]) -> Self {
		Self {
			width: array[0],
			height: array[1],
			depth: array[2],
		}
	}
}

/// Color transfer and luminance functions shared by CPU image processing, lighting, and diagnostics.
///
/// Use these instead of restating the constants, so every CPU path encodes and weighs color the same way. GPU
/// shaders keep their own copies.
pub mod color {
	/// Removes the IEC 61966-2-1 sRGB transfer function from one normalized channel.
	pub fn srgb_to_linear(encoded: f32) -> f32 {
		if encoded <= 0.04045 {
			encoded / 12.92
		} else {
			((encoded + 0.055) / 1.055).powf(2.4)
		}
	}

	/// Applies the IEC 61966-2-1 sRGB transfer function to one linear channel.
	pub fn linear_to_srgb(value: f32) -> f32 {
		if value <= 0.0031308 {
			12.92 * value
		} else {
			1.055 * value.powf(1.0 / 2.4) - 0.055
		}
	}

	/// Returns the Rec. 709 relative luminance of linear RGB.
	pub fn rec709_luminance(red: f32, green: f32, blue: f32) -> f32 {
		0.2126 * red + 0.7152 * green + 0.0722 * blue
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RGBA {
	pub r: f32,
	pub g: f32,
	pub b: f32,
	pub a: f32,
}

impl std::ops::Mul for RGBA {
	type Output = Self;

	fn mul(self, rhs: Self) -> Self::Output {
		Self {
			r: self.r * rhs.r,
			g: self.g * rhs.g,
			b: self.b * rhs.b,
			a: self.a * rhs.a,
		}
	}
}

impl std::ops::Mul<f32> for RGBA {
	type Output = Self;

	fn mul(self, rhs: f32) -> Self::Output {
		Self {
			r: self.r * rhs,
			g: self.g * rhs,
			b: self.b * rhs,
			a: self.a * rhs,
		}
	}
}

impl std::hash::Hash for RGBA {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.r.to_bits().hash(state);
		self.g.to_bits().hash(state);
		self.b.to_bits().hash(state);
		self.a.to_bits().hash(state);
	}
}

impl RGBA {
	pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
		Self { r, g, b, a }
	}

	pub fn black() -> Self {
		Self {
			r: 0.0,
			g: 0.0,
			b: 0.0,
			a: 1.0,
		}
	}

	pub fn white() -> Self {
		Self {
			r: 1.0,
			g: 1.0,
			b: 1.0,
			a: 1.0,
		}
	}

	pub fn transparent() -> Self {
		Self {
			r: 0.0,
			g: 0.0,
			b: 0.0,
			a: 0.0,
		}
	}
}

impl Default for RGBA {
	fn default() -> Self {
		Self::black()
	}
}

impl From<RGBA> for [f32; 4] {
	fn from(val: RGBA) -> Self {
		[val.r, val.g, val.b, val.a]
	}
}

/// Views a typed slice as its native bytes, for backends that hand raw host memory to a driver.
///
/// Prefer [`bytemuck::cast_slice`](https://docs.rs/bytemuck) where the element type is `Pod`: it proves at
/// compile time that `T` has no padding, which this function leaves to the caller.
pub fn as_byte_slice<T>(slice: &[T]) -> &[u8] {
	// SAFETY: The byte slice covers the same live allocation and cannot outlive the typed source slice.
	unsafe { std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice)) }
}

#[cfg(test)]
mod tests {
	use super::{BufferAllocator, Extent, as_byte_slice};

	#[test]
	fn buffer_allocator_returns_disjoint_ranges_and_tracks_padding() {
		let mut storage = [0u8; 16];
		{
			let mut allocator = BufferAllocator::new(&mut storage);

			let (first_offset, first) = allocator.take_with_offset(3);
			first.copy_from_slice(&[1, 2, 3]);
			let (second_offset, second) = allocator.take_with_offset_aligned(4, 4);
			second.copy_from_slice(&[4, 5, 6, 7]);

			assert_eq!(first_offset, 0);
			assert_eq!(second_offset, 4);
		}

		assert_eq!(&storage[..8], &[1, 2, 3, 0, 4, 5, 6, 7]);
	}

	#[test]
	fn extent_dimensions_mips_and_reductions_preserve_active_axes() {
		assert_eq!(Extent::line(8).dimensions(), 1);
		assert_eq!(Extent::square(8).dimensions(), 2);
		assert_eq!(Extent::cube(8, 4, 2).dimensions(), 3);
		assert_eq!(Extent::new(8, 1, 1).dimensions(), 3);
		assert_eq!(Extent::rectangle(8, 1).dimensions(), 2);
		assert_eq!(Extent::new(0, 0, 0).dimensions(), 0);
		assert_eq!(Extent::rectangle(17, 9).mip(1), Extent::rectangle(8, 4));
		assert_eq!(Extent::rectangle(17, 9).mip(40), Extent::rectangle(1, 1));
		assert_eq!(Extent::cube(8, 4, 2).mip(2), Extent::cube(2, 1, 1));
		assert_eq!(Extent::rectangle(1919, 1079).scaled_down(2), Extent::rectangle(959, 539));
		assert_eq!(Extent::square(1).scaled_down(2), Extent::square(1));
		assert_eq!(Extent::rectangle(16, 9).aspect_ratio(), 16.0 / 9.0);
	}

	#[test]
	fn byte_slice_views_preserve_native_object_representation() {
		let values = [0x1122u16, 0x3344u16];
		let bytes = as_byte_slice(&values);

		assert_eq!(bytes.len(), std::mem::size_of_val(&values));

		assert_eq!(bytes, [values[0].to_ne_bytes(), values[1].to_ne_bytes()].concat());
	}
}
