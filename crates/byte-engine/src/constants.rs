//! Common directions in the engine's default coordinate system.

use math::Vector3;

/// Points right along the positive x-axis.
pub const RIGHT: Vector3 = Vector3 { x: 1.0, y: 0.0, z: 0.0 };

/// Points up along the positive y-axis.
pub const UP: Vector3 = Vector3 { x: 0.0, y: 1.0, z: 0.0 };

/// Points forward along the positive z-axis.
pub const FORWARD: Vector3 = Vector3 { x: 0.0, y: 0.0, z: 1.0 };
