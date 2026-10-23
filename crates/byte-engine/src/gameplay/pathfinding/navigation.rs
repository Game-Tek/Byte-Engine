//! The world's navigation areas and agents, and the system that builds a navigation mesh from the areas and steers
//! the agents on it.

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

/// The `NavigationAgent` struct declares something that moves on the navigation mesh and that other agents steer
/// around, as a world component.
///
/// Create one on the world scope with a [`Transform`] under the same handle, usually beside a character's body, and
/// give it the character's radius. Then call [`Navigation::steer`] each tick to pick the velocity the character
/// moves at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationAgent {
	radius: f32,
	response_time: f32,
}

impl NavigationAgent {
	/// Creates an agent that occupies a circle of `radius` meters on XZ.
	pub fn new(radius: f32) -> Self {
		assert!(
			radius.is_finite() && radius > 0.0,
			"Invalid navigation agent radius. The most likely cause is a radius that is zero, negative, or not finite."
		);
		Self {
			radius,
			response_time: 0.0,
		}
	}

	/// Makes steering expect the agent to keep its current velocity for `seconds` before a new one takes effect.
	///
	/// Use it for agents that can't change course at once, such as a character whose animation turns and stops over a
	/// moment, so steering starts avoiding others from where the agent will really be.
	pub fn with_response_time(mut self, seconds: f32) -> Self {
		assert!(
			seconds.is_finite() && seconds >= 0.0,
			"Invalid navigation agent response time. The most likely cause is a time that is negative or not finite."
		);
		self.response_time = seconds;
		self
	}
}

/// The `PlacedAgent` struct joins an agent with where its transforms put it and how fast that moved it.
struct PlacedAgent {
	agent: NavigationAgent,
	position: Option<Point>,
	/// The displacement since the previous update over the time between them.
	velocity: Vector,
}

/// The `Navigation` struct keeps the navigation mesh of a world's areas for the path searches that run on it, and
/// the world's agents for steering them around each other.
///
/// Create it with [`Self::new`] before the areas and agents it reads, then call [`Self::update`] once per tick before
/// the searches that should see that tick's changes. Search [`Self::mesh`] with [`NavigationMesh::find_path`], then
/// pass the direction toward the path's next corner to [`Self::steer`].
pub struct Navigation {
	areas: DefaultListener<CreateMessage<NavigationArea>>,
	agents: DefaultListener<CreateMessage<NavigationAgent>>,
	transforms: DefaultListener<TransformationUpdate>,
	deletions: DefaultListener<DeleteMessage>,
	/// Each area with its transform, which arrives as a separate message. Handle ids count up as entities are
	/// created, so keying by them iterates in creation order, which decides who keeps overlaps.
	areas_by_id: BTreeMap<u32, (NavigationArea, Option<Transform>)>,
	agents_by_id: BTreeMap<u32, PlacedAgent>,
	mesh: NavigationMesh,
}

impl Navigation {
	/// Creates a system that builds from the navigation areas published in `world` from now on.
	pub fn new(world: &MessageScope) -> Self {
		Self {
			areas: world.factory::<NavigationArea>().listener(),
			agents: world.factory::<NavigationAgent>().listener(),
			transforms: world.channel::<TransformationUpdate>().listener(),
			deletions: world.channel::<DeleteMessage>().listener(),
			areas_by_id: BTreeMap::new(),
			agents_by_id: BTreeMap::new(),
			mesh: NavigationMesh::new(Vec::new(), Vec::new()).expect("an empty mesh is valid"),
		}
	}

	/// Adopts the area and agent changes published since the last update, `delta` ago, and rebuilds the mesh when any
	/// area changed.
	///
	/// When the rebuild fails, the previous mesh stays and the error says which polygon was invalid.
	pub fn update(&mut self, delta: MediaTime) -> Result<(), NavigationMeshBuildError> {
		let mut changed = false;
		let seconds = delta.as_seconds_f32();

		// Areas and agents first, so the transforms published beside them in the same tick find them.
		while let Some(message) = self.areas.read() {
			let area = *message.data();
			self.areas_by_id.entry(message.handle().id()).or_insert((area, None)).0 = area;
			changed = true;
		}
		while let Some(message) = self.agents.read() {
			let agent = *message.data();
			self.agents_by_id
				.entry(message.handle().id())
				.or_insert(PlacedAgent {
					agent,
					position: None,
					velocity: Vector::new(0.0, 0.0, 0.0),
				})
				.agent = agent;
		}
		// Each agent's velocity sums the moves its transforms make in this update, so one that didn't move is still.
		for agent in self.agents_by_id.values_mut() {
			agent.velocity = Vector::new(0.0, 0.0, 0.0);
		}
		// The route carries every entity's transforms, so only the areas' own count as changes.
		while let Some(message) = self.transforms.read() {
			let id = message.handle().id();
			if let Some((_, transform)) = self.areas_by_id.get_mut(&id) {
				*transform = Some(message.transform().clone());
				changed = true;
			}
			if let Some(agent) = self.agents_by_id.get_mut(&id) {
				let position = message.transform().get_position();
				if let Some(previous) = agent.position.filter(|_| seconds > 0.0) {
					agent.velocity += (position - previous) / seconds;
				}
				agent.position = Some(position);
			}
		}
		while let Some(message) = self.deletions.read() {
			let id = message.handle().id();
			changed |= self.areas_by_id.remove(&id).is_some();
			self.agents_by_id.remove(&id);
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

	/// Returns the velocity `agent` moves at to head along `direction` at one of `speeds` without running into other
	/// agents or off the mesh.
	///
	/// List the speeds the agent can move at, preferred first; a preferred speed of zero stands unless others crowd the
	/// agent, and a zero velocity means standing still. Each agent steers
	/// on its own against where the others are and how fast they moved at the latest update, so call it for each agent
	/// after [`Self::update`]. An agent without a transform steers at `direction` and its preferred speed.
	pub fn steer(&self, agent: Handle, direction: Vector, speeds: &[f32]) -> Vector {
		let id = agent.id();
		let Some((placed, position)) = self.agents_by_id.get(&id).and_then(|placed| Some((placed, placed.position?))) else {
			return steering::preferred_velocity(direction, speeds);
		};
		let others = self
			.agents_by_id
			.iter()
			.filter(move |&(&other, _)| other != id)
			.filter_map(|(_, other)| Some((other.position?, other.velocity, other.agent.radius)));
		let NavigationAgent { radius, response_time } = placed.agent;
		let agent = (position, placed.velocity, radius);
		steering::steer(&self.mesh, agent, response_time, direction, speeds, others)
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
		factory::Creator as _,
		message_bus::{MessageBus, MessageBusConfig},
	};

	fn place(world: &MessageScope, area: NavigationArea, transform: Transform) -> Handle {
		world.create(area).with(transform).into()
	}

	fn tick() -> MediaTime {
		MediaTime::from_seconds_f32(1.0 / 60.0)
	}

	/// Returns a world with a 10 m square floor and the navigation system that reads it, adopted.
	fn floor() -> (MessageBus, MessageScope, Navigation) {
		let bus = MessageBus::new(MessageBusConfig::new(1 << 20)).expect("a valid bus");
		let world = bus.new_scope("world");
		let mut navigation = Navigation::new(&world);
		bus.begin_tick();
		place(&world, NavigationArea::walkable(5.0, 5.0), Transform::identity());
		navigation.update(tick()).expect("a floor");
		(bus, world, navigation)
	}

	fn agent(world: &MessageScope, x: f32, z: f32) -> Handle {
		world
			.create(NavigationAgent::new(0.35))
			.with(Transform::from_position(Point::new(x, 0.0, z)))
			.into()
	}

	fn moved(world: &MessageScope, agent: Handle, x: f32, z: f32) {
		world.channel::<TransformationUpdate>().send(TransformationUpdate::new(
			agent,
			Transform::from_position(Point::new(x, 0.0, z)),
		));
	}

	const WALK: f32 = 1.4;
	fn east() -> Vector {
		Vector::new(1.0, 0.0, 0.0)
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
		navigation.update(tick()).expect("a floor with a wall");

		let mesh = navigation.mesh();
		assert_eq!(mesh.locate_polygon(Point::new(2.0, 0.0, 0.0)), None);
		assert!(mesh.locate_polygon(Point::new(-2.0, 0.0, 0.0)).is_some());
		assert!(mesh.find_path(Point::new(0.0, 0.0, 0.0), Point::new(4.0, 0.0, 0.0)).is_err());

		bus.begin_tick();
		world.channel::<TransformationUpdate>().send(TransformationUpdate::new(
			wall,
			Transform::from_position(Point::new(-2.0, 0.0, 0.0)),
		));
		navigation.update(tick()).expect("the wall moved");

		let mesh = navigation.mesh();
		assert!(mesh.locate_polygon(Point::new(2.0, 0.0, 0.0)).is_some());
		assert_eq!(mesh.locate_polygon(Point::new(-2.0, 0.0, 0.0)), None);

		bus.begin_tick();
		world.channel::<DeleteMessage>().send(DeleteMessage::new(wall));
		navigation.update(tick()).expect("the wall is gone");

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
		navigation.update(tick()).expect("a turned area");

		let mesh = navigation.mesh();
		assert!(mesh.locate_polygon(Point::new(10.0, 2.0, 3.5)).is_some());
		assert_eq!(mesh.locate_polygon(Point::new(13.5, 2.0, 0.0)), None);
		assert_eq!(mesh.locate_polygon(Point::new(0.0, 0.0, 0.0)), None);
		assert_eq!(
			mesh.project_point(Point::new(10.0, 2.0, -3.5)).map(|point| point.y()),
			Some(2.0)
		);
	}

	#[test]
	fn an_agent_with_open_ground_heads_straight_at_its_preferred_speed() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let walker = agent(&world, 0.0, 0.0);
		navigation.update(tick()).expect("an agent");

		assert_eq!(navigation.steer(walker, east() * 3.0, &[WALK]), east() * WALK);
	}

	#[test]
	fn an_agent_walks_around_a_standing_one_in_its_way() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let walker = agent(&world, 0.0, 0.0);
		agent(&world, 1.5, 0.0);
		navigation.update(tick()).expect("two agents");

		let velocity = navigation.steer(walker, east(), &[WALK]);
		assert!(velocity.z().abs() > 0.0, "it turns aside instead of walking into the other");
		assert!(velocity.x() > 0.0, "it still makes progress");
	}

	#[test]
	fn an_agent_avoids_one_that_moves_across_its_way() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let walker = agent(&world, 0.0, 0.0);
		// Standing, the other would pass 2 m to the side. Walking north, it reaches the walker's line when the walker
		// does.
		let crosser = agent(&world, 2.0, -2.0 - WALK / 60.0);
		navigation.update(tick()).expect("two agents");
		bus.begin_tick();
		moved(&world, crosser, 2.0, -2.0);
		navigation.update(tick()).expect("the crosser moved");

		assert_ne!(navigation.steer(walker, east(), &[WALK]), east() * WALK);
	}

	#[test]
	fn an_agent_boxed_in_by_others_stands_still() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let boxed = agent(&world, 0.0, 0.0);
		for step in 0..8 {
			let (sine, cosine) = (step as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
			agent(&world, 0.8 * cosine, 0.8 * sine);
		}
		navigation.update(tick()).expect("a ring of agents");

		assert_eq!(navigation.steer(boxed, east(), &[WALK]), Vector::new(0.0, 0.0, 0.0));
	}

	#[test]
	fn an_agent_that_prefers_standing_stays_unless_another_touches_it() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let alone = agent(&world, -3.0, 0.0);
		let crowded = agent(&world, 2.0, 0.0);
		agent(&world, 2.3, 0.0);
		navigation.update(tick()).expect("three agents");

		assert_eq!(navigation.steer(alone, east(), &[0.0, WALK]), Vector::new(0.0, 0.0, 0.0));
		let step = navigation.steer(crowded, east(), &[0.0, WALK]);
		assert!(step.length() > 0.0, "it steps out of the other's way");
	}

	#[test]
	fn an_agent_at_the_edge_turns_instead_of_leaving_the_mesh() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let walker = agent(&world, 4.9, 0.0);
		navigation.update(tick()).expect("an agent at the edge");

		let velocity = navigation.steer(walker, east(), &[WALK]);
		assert!(velocity.length() > 0.0);
		assert!(
			navigation
				.mesh()
				.locate_polygon(Point::new(4.9, 0.0, 0.0) + velocity * 0.5)
				.is_some()
		);
	}

	#[test]
	fn steering_starts_from_where_the_response_time_carries_the_agent() {
		let (bus, world, mut navigation) = floor();
		let north = Vector::new(0.0, 0.0, 1.0);
		bus.begin_tick();
		// Both walk east toward the floor's edge at x = 5, one of them unable to change course for a second.
		let nimble = agent(&world, 4.0 - WALK / 60.0, -2.0);
		let sluggish: Handle = world
			.create(NavigationAgent::new(0.35).with_response_time(1.0))
			.with(Transform::from_position(Point::new(4.0 - WALK / 60.0, 0.0, 2.0)))
			.into();
		navigation.update(tick()).expect("two agents");
		bus.begin_tick();
		moved(&world, nimble, 4.0, -2.0);
		moved(&world, sluggish, 4.0, 2.0);
		navigation.update(tick()).expect("both moved");

		assert_eq!(navigation.steer(nimble, north, &[WALK]), north * WALK);
		// A second at its velocity carries it past the edge, so it heads back onto the mesh.
		assert!(navigation.steer(sluggish, north, &[WALK]).x() < 0.0);
	}

	#[test]
	fn an_unknown_agent_heads_along_its_direction_at_its_preferred_speed() {
		let (_bus, world, navigation) = floor();
		let unplaced: Handle = world.create(NavigationAgent::new(0.35)).into();

		assert_eq!(navigation.steer(unplaced, east() * 2.0, &[WALK, 0.5]), east() * WALK);
	}
}

use std::collections::BTreeMap;

use math::{Point, Vector};

use super::{NavigationMesh, NavigationMeshBuildError, steering};
use crate::{
	MediaTime,
	core::{
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener as _},
		message::DeleteMessage,
		message_bus::MessageScope,
	},
	gameplay::{Transform, TransformationUpdate},
};
