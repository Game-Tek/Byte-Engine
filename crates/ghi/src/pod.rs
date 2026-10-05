//! Plain-old-data types for the scalar, vector and matrix data types shaders read and write.
//!
//! Build CPU structs that the GPU reads or writes out of these types, and use them as
//! specialization constant values. Each type mirrors the BESL type of the same name, such as
//! [`Vec3f`] for `vec3f` and [`F16`] for `f16`, and every one implements [`Pod`](bytemuck::Pod),
//! so a `#[repr(C)]` struct built from them can derive it too.
//!
//! Types follow the scalar block layout that shader buffers use: a vector is its components
//! packed back to back and aligned to one component, so [`Vec3f`] takes 12 bytes and [`Vec4f`]
//! is not padded to 16-byte alignment. A matrix is its column vectors back to back, so [`Mat4x3f`]
//! is four [`Vec3f`] columns taking 48 bytes.

/// The shader `bool`. It is 32 bits wide in buffers, so it is not Rust's one-byte `bool`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Bool(u32);

impl Bool {
	pub const FALSE: Self = Self(0);
	pub const TRUE: Self = Self(1);

	pub const fn new(value: bool) -> Self {
		Self(value as u32)
	}

	pub const fn get(self) -> bool {
		self.0 != 0
	}
}

impl From<bool> for Bool {
	fn from(value: bool) -> Self {
		Self::new(value)
	}
}

impl From<Bool> for bool {
	fn from(value: Bool) -> Self {
		value.get()
	}
}

pub type U8 = u8;
pub type U16 = u16;
pub type U32 = u32;
pub type I32 = i32;
pub type F16 = f16;
pub type F32 = f32;

macro_rules! shader_vectors {
	($($(#[$meta:meta])* $name:ident($scalar:ty, $count:literal; $($component:ident),+);)+) => {$(
		$(#[$meta])*
		#[repr(C)]
		#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
		pub struct $name {
			$(pub $component: $scalar,)+
		}

		impl $name {
			pub const fn new($($component: $scalar),+) -> Self {
				Self { $($component),+ }
			}

			pub const fn splat(value: $scalar) -> Self {
				Self { $($component: value),+ }
			}
		}

		impl From<[$scalar; $count]> for $name {
			fn from([$($component),+]: [$scalar; $count]) -> Self {
				Self { $($component),+ }
			}
		}

		impl From<$name> for [$scalar; $count] {
			fn from(value: $name) -> Self {
				[$(value.$component),+]
			}
		}
	)+};
}

shader_vectors! {
	/// The shader `vec2u16`.
	Vec2u16(u16, 2; x, y);
	/// The shader `vec4u16`.
	Vec4u16(u16, 4; x, y, z, w);
	/// The shader `vec2u`.
	Vec2u(u32, 2; x, y);
	/// The shader `vec3u`.
	Vec3u(u32, 3; x, y, z);
	/// The shader `vec4u`.
	Vec4u(u32, 4; x, y, z, w);
	/// The shader `vec2i`.
	Vec2i(i32, 2; x, y);
	/// The shader `vec2f16`.
	Vec2f16(f16, 2; x, y);
	/// The shader `vec3f16`.
	Vec3f16(f16, 3; x, y, z);
	/// The shader `vec4f16`.
	Vec4f16(f16, 4; x, y, z, w);
	/// The shader `vec2f`.
	Vec2f(f32, 2; x, y);
	/// The shader `vec3f`.
	Vec3f(f32, 3; x, y, z);
	/// The shader `vec4f`.
	Vec4f(f32, 4; x, y, z, w);
}

macro_rules! shader_matrices {
	($($(#[$meta:meta])* $name:ident($column:ident, $scalar:ty, $rows:literal, $count:literal; $($component:ident),+);)+) => {$(
		$(#[$meta])*
		#[repr(C)]
		#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
		pub struct $name {
			$(pub $component: $column,)+
		}

		impl $name {
			pub const fn new($($component: $column),+) -> Self {
				Self { $($component),+ }
			}
		}

		impl From<[[$scalar; $rows]; $count]> for $name {
			fn from([$($component),+]: [[$scalar; $rows]; $count]) -> Self {
				Self { $($component: $column::from($component)),+ }
			}
		}

		impl From<$name> for [[$scalar; $rows]; $count] {
			fn from(value: $name) -> Self {
				[$(value.$component.into()),+]
			}
		}
	)+};
}

shader_matrices! {
	/// The shader `mat2f`, as two [`Vec2f`] columns.
	Mat2f(Vec2f, f32, 2, 2; x, y);
	/// The shader `mat3f`, as three [`Vec3f`] columns.
	Mat3f(Vec3f, f32, 3, 3; x, y, z);
	/// The shader `mat4f`, as four [`Vec4f`] columns.
	Mat4f(Vec4f, f32, 4, 4; x, y, z, w);
	/// The shader `mat4x3f`, as four [`Vec3f`] columns.
	Mat4x3f(Vec3f, f32, 3, 4; x, y, z, w);
}

/// Lays out an engine matrix the way the platform's shaders multiply `mat4f`: Metal reads its columns, the other
/// backends read its rows.
impl From<math::Matrix> for Mat4f {
	fn from(value: math::Matrix) -> Self {
		// The engine matrix stores its elements row by row.
		let rows: [[f32; 4]; 4] = bytemuck::cast(value.m);
		Self::from(if cfg!(target_os = "macos") {
			std::array::from_fn(|column| rows.map(|row| row[column]))
		} else {
			rows
		})
	}
}

/// Keeps the affine part of an engine matrix, taking the top three rows of each of its four columns.
impl From<math::Matrix> for Mat4x3f {
	fn from(value: math::Matrix) -> Self {
		Self::from(math::AffineMatrix::from_matrix(value))
	}
}

impl From<math::AffineMatrix> for Mat4x3f {
	fn from(value: math::AffineMatrix) -> Self {
		Self::from(value.columns())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn types_match_the_scalar_block_layout() {
		assert_eq!((size_of::<Bool>(), align_of::<Bool>()), (4, 4));
		assert_eq!((size_of::<F16>(), align_of::<F16>()), (2, 2));
		assert_eq!((size_of::<Vec2u16>(), align_of::<Vec2u16>()), (4, 2));
		assert_eq!((size_of::<Vec4u16>(), align_of::<Vec4u16>()), (8, 2));
		assert_eq!((size_of::<Vec3u>(), align_of::<Vec3u>()), (12, 4));
		assert_eq!((size_of::<Vec2i>(), align_of::<Vec2i>()), (8, 4));
		assert_eq!((size_of::<Vec3f16>(), align_of::<Vec3f16>()), (6, 2));
		assert_eq!((size_of::<Vec3f>(), align_of::<Vec3f>()), (12, 4));
		assert_eq!((size_of::<Vec4f>(), align_of::<Vec4f>()), (16, 4));
		assert_eq!((size_of::<Mat2f>(), align_of::<Mat2f>()), (16, 4));
		assert_eq!((size_of::<Mat3f>(), align_of::<Mat3f>()), (36, 4));
		assert_eq!((size_of::<Mat4f>(), align_of::<Mat4f>()), (64, 4));
		assert_eq!((size_of::<Mat4x3f>(), align_of::<Mat4x3f>()), (48, 4));
	}

	#[test]
	fn affine_matrices_keep_the_top_three_rows_of_each_column() {
		let mut matrix = math::Matrix::zero();
		for index in 0..16 {
			matrix[index] = (index + 1) as f32;
		}

		let affine: [[f32; 3]; 4] = Mat4x3f::from(matrix).into();

		assert_eq!(
			affine,
			[[1.0, 5.0, 9.0], [2.0, 6.0, 10.0], [3.0, 7.0, 11.0], [4.0, 8.0, 12.0]]
		);
	}
}
