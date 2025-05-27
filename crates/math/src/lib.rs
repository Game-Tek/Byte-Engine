pub mod plane;
pub mod sphere;

pub mod collision;

pub use maths_rs::Vec2f as Vector2;
pub use maths_rs::Vec3f as Vector3;
pub use maths_rs::Vec4f as Vector4;

pub use maths_rs::Mat3f as Matrix3;
pub use maths_rs::Mat4f as Matrix4;
pub use maths_rs::Quatf as Quaternion;

pub use maths_rs::normalize as normalize;

#[macro_use]
pub mod macros {
	#[macro_export]
	macro_rules! assert_float_eq_with_epsilon {
		($left:expr, $right:expr, $epsilon:expr) => {
			match (&$left, &$right) {
				(left_val, right_val) => {
					if ((*left_val as f64) - (*right_val as f64)).abs() > $epsilon as f64 {
						panic!(
							"assertion failed: `(left == right)`\n  left: `{:?}`,\n right: `{:?}`",
							*left_val, *right_val
						)
					}
				}
			}
		};
		($left:expr, $right:expr, $epsilon:expr, $($arg:tt)+) => {
			match (&$left, &$right) {
				(left_val, right_val) => {
					if ((*left_val as f64) - (*right_val as f64)).abs() > $epsilon as f64 {
						panic!(
							"assertion failed: `(left == right)`\n  left: `{:?}`,\n right: `{:?}`\n{}",
							*left_val, *right_val, format_args!($($arg)+)
						)
					}
				}
			}
		};
	}

	#[macro_export]
	macro_rules! assert_float_eq {
		($left:expr, $right:expr) => {
			$crate::assert_float_eq_with_epsilon!($left, $right, 0.001)
		};
		($left:expr, $right:expr, $($arg:tt)+) => {
			$crate::assert_float_eq_with_epsilon!($left, $right, 0.001, $($arg)+)
		};
	}

	#[macro_export]
	macro_rules! assert_vec3f_near {
		($left:expr, $right:expr) => {
			$crate::assert_float_eq!($left.x, $right.x);
			$crate::assert_float_eq!($left.y, $right.y);
			$crate::assert_float_eq!($left.z, $right.z);
		};
		($left:expr, $right:expr, $($arg:tt)+) => {
			$crate::assert_float_eq!($left.x, $right.x, $($arg)+);
			$crate::assert_float_eq!($left.y, $right.y, $($arg)+);
			$crate::assert_float_eq!($left.z, $right.z, $($arg)+);
		};
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::Vec3f;

	#[test]
	fn test_float_equality() {
		let result = 2.0 + 2.0;
		assert_float_eq!(result, 4.0);
	}

	#[test]
	#[should_panic(expected = "assertion failed")]
	fn test_float_inequality() {
		let result = 2.0 + 2.0;
		assert_float_eq!(result, 5.0);
	}

	#[test]
	fn test_vec3f_near() {
		let vec1 = Vec3f::new(1.0, 2.0, 3.0);
		let vec2 = Vec3f::new(1.0, 2.0, 3.00005);
		assert_vec3f_near!(vec1, vec2);
	}
}