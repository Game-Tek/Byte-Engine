use std::convert::Infallible;
use std::ops::Mul;

use maths_rs::mat::MatNew4;

use crate::Matrix;
use crate::serialization::{ArrayForm, serialize_as_array};

/// The `AffineMatrix` struct stores an affine transform compactly as four columns of three.
///
/// The first three columns are the linear basis and the fourth is the translation. The omitted bottom row is always
/// `[0.0, 0.0, 0.0, 1.0]`, so an affine transform such as a skeleton pose or inverse-bind matrix takes 12 floats
/// instead of 16. Use [`Self::from_matrix`] and [`Self::into_matrix`] to cross to a full [`Matrix`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AffineMatrix {
	columns: [[f32; 3]; 4],
}

impl AffineMatrix {
	/// Creates the transform that leaves every point in place.
	pub const fn identity() -> Self {
		Self::from_columns([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]])
	}

	/// Creates a transform from its three basis columns followed by its translation.
	pub const fn from_columns(columns: [[f32; 3]; 4]) -> Self {
		Self { columns }
	}

	/// Returns the three basis columns followed by the translation.
	pub const fn columns(&self) -> [[f32; 3]; 4] {
		self.columns
	}

	/// Returns whether every component is finite.
	pub fn is_finite(&self) -> bool {
		self.columns.iter().flatten().all(|component| component.is_finite())
	}

	/// Keeps the top three rows of a full matrix and drops its bottom row.
	///
	/// The bottom row is assumed to be `[0.0, 0.0, 0.0, 1.0]`. Validate a matrix from outside the engine before
	/// converting it, because a projective bottom row is discarded rather than reported.
	pub fn from_matrix(matrix: Matrix) -> Self {
		Self::from_columns(std::array::from_fn(|column| std::array::from_fn(|row| matrix[(row, column)])))
	}

	/// Returns the full matrix with the implied `[0.0, 0.0, 0.0, 1.0]` bottom row.
	pub fn into_matrix(self) -> Matrix {
		let [x, y, z, translation] = self.columns;
		Matrix::new(
			x[0],
			y[0],
			z[0],
			translation[0],
			x[1],
			y[1],
			z[1],
			translation[1],
			x[2],
			y[2],
			z[2],
			translation[2],
			0.0,
			0.0,
			0.0,
			1.0,
		)
	}
}

impl Default for AffineMatrix {
	fn default() -> Self {
		Self::identity()
	}
}

/// Composes two transforms so that `right` applies first and `self` second, like matrix multiplication.
impl Mul for AffineMatrix {
	type Output = Self;

	// Composing affine transforms adds the left translation to the rotated right translation.
	#[allow(clippy::suspicious_arithmetic_impl)]
	fn mul(self, right: Self) -> Self {
		let left = self.columns;
		let right = right.columns;
		let mut product = [[0.0; 3]; 4];
		for column in 0..4 {
			for row in 0..3 {
				product[column][row] = (0..3).map(|index| left[index][row] * right[column][index]).sum();
			}
		}
		for row in 0..3 {
			product[3][row] += left[3][row];
		}
		Self::from_columns(product)
	}
}

impl ArrayForm<12> for AffineMatrix {
	type Array = [[f32; 3]; 4];
	type Error = Infallible;

	fn to_array(&self) -> Self::Array {
		self.columns
	}

	fn try_from_array(array: Self::Array) -> Result<Self, Self::Error> {
		Ok(Self::from_columns(array))
	}
}

serialize_as_array!(AffineMatrix, 12);

#[cfg(test)]
mod tests {
	use super::AffineMatrix;
	use crate::Matrix;

	fn sequential_matrix() -> Matrix {
		let mut matrix = Matrix::identity();
		for row in 0..3 {
			for column in 0..4 {
				matrix[(row, column)] = (row * 4 + column + 1) as f32;
			}
		}
		matrix
	}

	#[test]
	fn matrix_round_trip_keeps_the_top_three_rows_of_each_column() {
		let affine = AffineMatrix::from_matrix(sequential_matrix());

		assert_eq!(
			affine.columns(),
			[[1.0, 5.0, 9.0], [2.0, 6.0, 10.0], [3.0, 7.0, 11.0], [4.0, 8.0, 12.0]]
		);
		assert_eq!(affine.into_matrix(), sequential_matrix());
	}

	#[test]
	fn composition_matches_full_matrix_multiplication() {
		let left = AffineMatrix::from_matrix(sequential_matrix());
		let right = AffineMatrix::from_columns([[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 2.0], [3.0, -4.0, 5.0]]);

		assert_eq!((left * right).into_matrix(), left.into_matrix() * right.into_matrix());
	}
}
