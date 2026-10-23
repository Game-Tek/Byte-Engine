//! The world's navigation areas and the system that keeps a navigation mesh built from them.

/// The `NavigationArea` enum declares where agents may walk and where they may not, as world components.
///
/// Create one on the world scope with a [`Transform`] under the same handle, usually beside the floor or wall it
/// describes. The rectangle lies in the area's local XZ plane, centered on its origin; the transform's position and
/// orientation place it and its scale is ignored. [`Navigation`] combines every area into its mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NavigationArea {
	/// A rectangle agents may walk on.
	Walkable { half_extents: [f32; 2] },
	/// A rectangle cut out of every walkable area it overlaps, at any height.
	Obstacle { half_extents: [f32; 2] },
}

impl NavigationArea {
	/// Creates a walkable rectangle reaching `half_x` meters along local X and `half_z` along local Z from its center.
	pub fn walkable(half_x: f32, half_z: f32) -> Self {
		Self::Walkable {
			half_extents: checked_half_extents(half_x, half_z),
		}
	}

	/// Creates an obstacle rectangle reaching `half_x` meters along local X and `half_z` along local Z from its center.
	///
	/// Agents are points to the mesh, so include the clearance they need in the extents.
	pub fn obstacle(half_x: f32, half_z: f32) -> Self {
		Self::Obstacle {
			half_extents: checked_half_extents(half_x, half_z),
		}
	}

	/// Returns the rectangle's corners in the world, as `transform` places them.
	fn corners(self, transform: &Transform) -> [Point; 4] {
		let (Self::Walkable { half_extents: [x, z] } | Self::Obstacle { half_extents: [x, z] }) = self;
		let position = transform.get_position();
		let orientation = transform.get_orientation();
		[[-x, -z], [x, -z], [x, z], [-x, z]].map(|[x, z]| position + orientation.rotate_vector(Vector::new(x, 0.0, z)))
	}
}

fn checked_half_extents(half_x: f32, half_z: f32) -> [f32; 2] {
	assert!(
		half_x.is_finite() && half_x > 0.0 && half_z.is_finite() && half_z > 0.0,
		"Invalid navigation area half extents. The most likely cause is a size that is zero, negative, or not finite."
	);
	[half_x, half_z]
}

/// The `Navigation` struct keeps the navigation mesh of a world's areas for the path searches that run on it.
///
/// Create it with [`Self::new`] before the areas it builds from, then call [`Self::update`] once per tick before the
/// searches that should see that tick's changes, and search [`Self::mesh`] with
/// [`NavigationMesh::find_path`].
pub struct Navigation {
	areas: DefaultListener<CreateMessage<NavigationArea>>,
	transforms: DefaultListener<TransformationUpdate>,
	deletions: DefaultListener<DeleteMessage>,
	/// Each area with its transform, which arrives as a separate message. Handle ids count up as entities are
	/// created, so keying by them iterates in creation order, which decides who keeps overlaps.
	areas_by_id: BTreeMap<u32, (NavigationArea, Option<Transform>)>,
	mesh: NavigationMesh,
}

impl Navigation {
	/// Creates a system that builds from the navigation areas published in `world` from now on.
	pub fn new(world: &MessageScope) -> Self {
		Self {
			areas: world.factory::<NavigationArea>().listener(),
			transforms: world.channel::<TransformationUpdate>().listener(),
			deletions: world.channel::<DeleteMessage>().listener(),
			areas_by_id: BTreeMap::new(),
			mesh: NavigationMesh::new(Vec::new(), Vec::new()).expect("an empty mesh is valid"),
		}
	}

	/// Adopts the area changes published since the last update, and rebuilds the mesh when any area changed.
	///
	/// When the rebuild fails, the previous mesh stays and the error says which polygon was invalid.
	pub fn update(&mut self) -> Result<(), NavigationMeshBuildError> {
		let mut changed = false;

		// Areas first, so the transforms published beside them in the same tick find them.
		while let Some(message) = self.areas.read() {
			let area = *message.data();
			self.areas_by_id.entry(message.handle().id()).or_insert((area, None)).0 = area;
			changed = true;
		}
		// The route carries every entity's transforms, so only the areas' own count as changes.
		while let Some(message) = self.transforms.read() {
			if let Some((_, transform)) = self.areas_by_id.get_mut(&message.handle().id()) {
				*transform = Some(message.transform().clone());
				changed = true;
			}
		}
		while let Some(message) = self.deletions.read() {
			changed |= self.areas_by_id.remove(&message.handle().id()).is_some();
		}

		if changed {
			self.mesh = self.build()?;
		}
		Ok(())
	}

	/// Returns the mesh built at the latest update that changed an area.
	pub fn mesh(&self) -> &NavigationMesh {
		&self.mesh
	}

	/// Builds a mesh from every placed area, in creation order.
	fn build(&self) -> Result<NavigationMesh, NavigationMeshBuildError> {
		let mut walkable = Vec::new();
		let mut obstacles = Vec::new();
		for (area, transform) in self.areas_by_id.values() {
			let Some(transform) = transform else { continue };
			let corners = area.corners(transform);
			match area {
				NavigationArea::Walkable { .. } => walkable.push(corners),
				NavigationArea::Obstacle { .. } => obstacles.push(corners),
			}
		}
		NavigationMesh::from_areas(&walkable, &obstacles)
	}
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Scale};

	use super::*;
	use crate::core::{
		channel::Channel as _,
		factory::{Creator as _, Handle},
		message_bus::{MessageBus, MessageBusConfig},
	};

	fn place(world: &MessageScope, area: NavigationArea, transform: Transform) -> Handle {
		world.create(area).with(transform).into()
	}

	#[test]
	fn the_mesh_follows_areas_as_they_are_created_moved_and_deleted() {
		let bus = MessageBus::new(MessageBusConfig::new(1 << 20)).expect("a valid bus");
		let world = bus.new_scope("world");
		let mut navigation = Navigation::new(&world);

		bus.begin_tick();
		place(&world, NavigationArea::walkable(5.0, 5.0), Transform::identity());
		let wall = place(
			&world,
			NavigationArea::obstacle(0.5, 6.0),
			Transform::from_position(Point::new(2.0, 0.0, 0.0)),
		);
		navigation.update().expect("a floor with a wall");

		let mesh = navigation.mesh();
		assert_eq!(mesh.locate_polygon(Point::new(2.0, 0.0, 0.0)), None);
		assert!(mesh.locate_polygon(Point::new(-2.0, 0.0, 0.0)).is_some());
		assert!(mesh.find_path(Point::new(0.0, 0.0, 0.0), Point::new(4.0, 0.0, 0.0)).is_err());

		bus.begin_tick();
		world.channel::<TransformationUpdate>().send(TransformationUpdate::new(
			wall,
			Transform::from_position(Point::new(-2.0, 0.0, 0.0)),
		));
		navigation.update().expect("the wall moved");

		let mesh = navigation.mesh();
		assert!(mesh.locate_polygon(Point::new(2.0, 0.0, 0.0)).is_some());
		assert_eq!(mesh.locate_polygon(Point::new(-2.0, 0.0, 0.0)), None);

		bus.begin_tick();
		world.channel::<DeleteMessage>().send(DeleteMessage::new(wall));
		navigation.update().expect("the wall is gone");

		assert!(navigation.mesh().locate_polygon(Point::new(-2.0, 0.0, 0.0)).is_some());
	}

	#[test]
	fn an_area_is_placed_by_its_transform_position_and_orientation() {
		let bus = MessageBus::new(MessageBusConfig::new(1 << 20)).expect("a valid bus");
		let world = bus.new_scope("world");
		let mut navigation = Navigation::new(&world);
		let quarter_turn =
			Orientation::try_from_axis_angle(crate::constants::up(), math::Radians::new(std::f32::consts::FRAC_PI_2))
				.expect("a rotation");

		bus.begin_tick();
		// Four meters along local X turn to four meters along world Z.
		place(
			&world,
			NavigationArea::walkable(4.0, 1.0),
			Transform::new(Point::new(10.0, 2.0, 0.0), Scale::identity(), quarter_turn),
		);
		// An area without a transform isn't part of the mesh.
		world.create(NavigationArea::walkable(1.0, 1.0));
		navigation.update().expect("a turned area");

		let mesh = navigation.mesh();
		assert!(mesh.locate_polygon(Point::new(10.0, 2.0, 3.5)).is_some());
		assert_eq!(mesh.locate_polygon(Point::new(13.5, 2.0, 0.0)), None);
		assert_eq!(mesh.locate_polygon(Point::new(0.0, 0.0, 0.0)), None);
		assert_eq!(
			mesh.project_point(Point::new(10.0, 2.0, -3.5)).map(|point| point.y()),
			Some(2.0)
		);
	}
}

use std::collections::BTreeMap;

use math::{Point, Vector};

use super::{NavigationMesh, NavigationMeshBuildError};
use crate::{
	core::{
		factory::CreateMessage,
		listener::{DefaultListener, Listener as _},
		message::DeleteMessage,
		message_bus::MessageScope,
	},
	gameplay::{Transform, TransformationUpdate},
};
