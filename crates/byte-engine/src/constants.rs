use math::Vector3;

/// A vector pointing to the right in the default coordinate system.
/// That is the positive x-axis.
pub const RIGHT: Vector3 = Vector3{ x: 1.0, y: 0.0, z: 0.0 };

/// A vector pointing up in the default coordinate system.
/// That is the positive y-axis.
pub const UP: Vector3 = Vector3{ x: 0.0, y: 1.0, z: 0.0 };

/// A vector pointing forward in the default coordinate system.
/// That is the positive z-axis.
pub const FORWARD: Vector3 = Vector3{ x: 0.0, y: 0.0, z: 1.0 };
