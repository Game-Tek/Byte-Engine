use math::AABB;

use crate::physics::LocalSpace;

/// The `Bounds` type aliases a collider-local axis-aligned bounding box.
///
/// Use [`Aabb::from_center_and_half_extents`] to construct bounds from authored extents.
pub type Bounds = AABB<LocalSpace>;
