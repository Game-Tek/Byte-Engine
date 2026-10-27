//! A sample crate for the API model tests.

/// Engine-facing items.
pub mod engine {
	#[doc(hidden)]
	pub mod hidden {
		/// The `Hidden` struct is defined in a hidden module and reached through a re-export.
		pub struct Hidden;

		/// The `Unreachable` struct is never re-exported, so the API leaves it out.
		pub struct Unreachable;
	}

	pub use hidden::Hidden;

	/// The `Point` struct holds a position. Pass it to [`shape_at`] or start from [`Point::origin`].
	///
	/// It replaces [`hidden::Unreachable`], which has no page.
	#[derive(Clone, Debug, PartialEq)]
	pub struct Point<T> {
		/// The horizontal coordinate.
		pub x: T,
		/// The vertical coordinate.
		pub y: T,
	}

	impl<T: Default> Point<T> {
		/// Returns the point at the origin.
		pub fn origin() -> Self {
			Self { x: T::default(), y: T::default() }
		}
	}

	/// The `Shape` enum lists the shapes the sample measures.
	///
	/// Make one with [`shape_at`](crate::engine::shape_at), as the [Rust book](https://doc.rust-lang.org/book/) shows.
	pub enum Shape {
		/// A circle.
		Circle {
			/// The circle radius.
			radius: f32,
		},
		/// A square with the given side length.
		Square(f32),
		/// No shape.
		Empty,
	}

	/// The `Area` trait measures a shape.
	pub trait Area {
		/// Returns the area.
		fn area(&self) -> f32;

		/// Returns twice the area.
		fn double(&self) -> f32 {
			self.area() * 2.0
		}
	}

	impl Area for Shape {
		fn area(&self) -> f32 {
			0.0
		}
	}

	/// Builds a shape at a point.
	pub fn shape_at(point: &Point<f32>) -> Shape {
		let _ = point;
		Shape::Empty
	}

	/// The largest shape count.
	///
	/// ```
	/// # let hidden_setup = 1;
	/// let visible = sample::engine::LIMIT;
	/// ```
	pub const LIMIT: u32 = 4;
}

pub use engine::Point;
