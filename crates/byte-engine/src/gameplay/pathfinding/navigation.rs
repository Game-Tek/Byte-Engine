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
/// moves at. Publish it again under the same handle to change its precedence as the character settles or sets off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationAgent {
	radius: f32,
	precedence: f32,
}

impl NavigationAgent {
	/// Creates an agent that occupies a circle of `radius` meters on XZ.
	pub fn new(radius: f32) -> Self {
		assert!(
			radius.is_finite() && radius > 0.0,
			"Invalid navigation agent radius. The most likely cause is a radius that is zero, negative, or not finite."
		);
		Self { radius, precedence: 0.0 }
	}

	/// Ranks the agent among those it meets: it steps aside only for agents of equal or greater `precedence`, and
	/// agents of less precedence step aside for it.
	///
	/// Use it so a character that has settled holds its spot against one still arriving, and a player holds its
	/// course against every NPC. Any number does, infinities included.
	pub fn with_precedence(mut self, precedence: f32) -> Self {
		assert!(
			!precedence.is_nan(),
			"Invalid navigation agent precedence. The most likely cause is a value computed from a NaN."
		);
		self.precedence = precedence;
		self
	}
}

/// The `PlacedAgent` struct joins an agent with where its transforms put it and how fast that moved it.
struct PlacedAgent {
	agent: NavigationAgent,
	position: Option<Point>,
	/// The displacement per second, averaged over about [`VELOCITY_AVERAGE_SECONDS`].
	velocity: Vector,
	/// How far the agent's transforms moved it in this update, until it joins the average.
	displacement: Vector,
}

/// The `Navigation` struct keeps the navigation mesh of a world's areas for the path searches that run on it, and
/// the world's agents for finding their paths and steering them around each other.
///
/// Create it with [`Self::new`] before the areas and agents it reads, then call [`Self::update`] once per tick before
/// the searches that should see that tick's changes. Find each agent's path with [`Self::find_path`], then pass the
/// velocity toward a point along it to [`Self::steer`].
///
/// # Configuration
/// Read from the parameters the caller routes to it, usually `app.parameters().scope("navigation")`:
/// - `debug`: When `true`, draws the mesh's boundary, each agent's radius, and each agent's latest path in the world
///   as debug meshes, which the application shows by registering
///   [`setup_debug_mesh_render_pass`](crate::application::graphics::setup_debug_mesh_render_pass) first.
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
	drawing: Option<Drawing>,
}

/// The `Drawing` struct keeps the debug meshes that show the mesh's boundary, the agents' radii, and their paths, so
/// each redraw replaces them under the same handles instead of piling up.
struct Drawing {
	meshes: Factory<DebugMesh>,
	deletions: DefaultChannel<DeleteMessage>,
	boundary: Vec<Handle>,
	radii: BTreeMap<u32, Vec<Handle>>,
	paths: BTreeMap<u32, Vec<Handle>>,
}

impl Drawing {
	/// Shows `meshes` under `handles`, reusing them, creating the handles it lacks, and deleting the ones it no
	/// longer needs.
	fn show(&mut self, handles: &mut Vec<Handle>, meshes: impl Iterator<Item = DebugMesh>) {
		let mut count = 0;
		for (index, mesh) in meshes.enumerate() {
			match handles.get(index) {
				Some(&handle) => self.meshes.derive(handle, mesh),
				None => handles.push(self.meshes.create(mesh)),
			}
			count = index + 1;
		}
		for handle in handles.drain(count..) {
			self.deletions.send(DeleteMessage::new(handle));
		}
	}

	/// Shows `meshes` under the handles kept for `id` in `kept`, the agent's radius or its path.
	fn show_for(
		&mut self,
		kept: fn(&mut Self) -> &mut BTreeMap<u32, Vec<Handle>>,
		id: u32,
		meshes: impl Iterator<Item = DebugMesh>,
	) {
		let mut handles = kept(self).remove(&id).unwrap_or_default();
		self.show(&mut handles, meshes);
		if !handles.is_empty() {
			kept(self).insert(id, handles);
		}
	}
}

/// Returns a debug line from `start` to `end` in `color`, just above the surface so the floor doesn't hide it.
fn line(start: Point, end: Point, color: RGBA) -> DebugMesh {
	let lift = Vector::new(0.0, LINE_HEIGHT, 0.0);
	DebugMesh::new(
		DebugShape::Segment {
			start: start + lift,
			end: end + lift,
		},
		color,
	)
}

impl Navigation {
	/// Creates a system that builds from the navigation areas published in `world` from now on, set up from
	/// `parameters`, the navigation subtree of the application's.
	pub fn new(world: &MessageScope, parameters: impl Parameters) -> Self {
		let debug = parameters
			.get_parameter("debug")
			.is_some_and(|parameter| parameter.as_bool_simple());
		Self {
			areas: world.factory::<NavigationArea>().listener(),
			agents: world.factory::<NavigationAgent>().listener(),
			transforms: world.channel::<TransformationUpdate>().listener(),
			deletions: world.channel::<DeleteMessage>().listener(),
			areas_by_id: BTreeMap::new(),
			agents_by_id: BTreeMap::new(),
			mesh: NavigationMesh::new(Vec::new(), Vec::new()).expect("an empty mesh is valid"),
			drawing: debug.then(|| Drawing {
				meshes: world.factory(),
				deletions: world.channel(),
				boundary: Vec::new(),
				radii: BTreeMap::new(),
				paths: BTreeMap::new(),
			}),
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
					displacement: Vector::new(0.0, 0.0, 0.0),
				})
				.agent = agent;
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
				if let Some(previous) = agent.position {
					agent.displacement += position - previous;
				}
				agent.position = Some(position);
			}
		}
		// Root-motion strides speed up and slow down within each step, so velocities average this update's displacement
		// into the last quarter of a second's instead of taking it alone.
		if seconds > 0.0 {
			let follow = 1.0 - (-seconds / VELOCITY_AVERAGE_SECONDS).exp();
			let drawing = &mut self.drawing;
			for (&id, agent) in &mut self.agents_by_id {
				agent.velocity += (agent.displacement / seconds - agent.velocity) * follow;
				// An agent's radius is drawn where it stands, and again wherever it moves.
				if let (Some(drawing), Some(position)) = (drawing.as_mut(), agent.position)
					&& (agent.displacement.length() > 0.0 || !drawing.radii.contains_key(&id))
				{
					let circle = DebugShape::Circle {
						center: position + Vector::new(0.0, LINE_HEIGHT, 0.0),
						radius: agent.agent.radius,
						normal: UnitVector::y_axis(),
					};
					let mesh = DebugMesh::new(circle, RADIUS_COLOR);
					drawing.show_for(|drawing| &mut drawing.radii, id, std::iter::once(mesh));
				}
				agent.displacement = Vector::new(0.0, 0.0, 0.0);
			}
		}
		while let Some(message) = self.deletions.read() {
			let id = message.handle().id();
			changed |= self.areas_by_id.remove(&id).is_some();
			if self.agents_by_id.remove(&id).is_some()
				&& let Some(drawing) = &mut self.drawing
			{
				drawing.show_for(|drawing| &mut drawing.radii, id, std::iter::empty());
				drawing.show_for(|drawing| &mut drawing.paths, id, std::iter::empty());
			}
		}

		if changed {
			self.mesh = self.build()?;
			if let Some(drawing) = &mut self.drawing {
				let mut lines = std::mem::take(&mut drawing.boundary);
				let edges = self.mesh.boundary().map(|(from, to)| line(from, to, BOUNDARY_COLOR));
				drawing.show(&mut lines, edges);
				drawing.boundary = lines;
			}
		}
		Ok(())
	}

	/// Finds the path from where `agent` stands to `target` and writes its corners into `path`, both ends moved onto
	/// the mesh first.
	///
	/// Either end may be just off the mesh, the agent from being pushed and the target inside an obstacle's
	/// clearance, so each starts from the closest point on it. Aim the agent a little way along the result, then
	/// pass that velocity to [`Self::steer`]. With debug drawing on, the path shows in the world until the agent's
	/// next search. An agent without a transform has no start.
	pub fn find_path<'path>(
		&mut self,
		agent: Handle,
		target: Point,
		path: &'path mut Vec<Point>,
	) -> Result<&'path [Point], NavigationPathError> {
		let id = agent.id();
		let position = self.agents_by_id.get(&id).and_then(|placed| placed.position);
		let start = position
			.and_then(|position| self.mesh.closest_point(position))
			.ok_or(NavigationPathError::StartOutsideMesh)?;
		let target = self
			.mesh
			.closest_point(target)
			.ok_or(NavigationPathError::TargetOutsideMesh)?;
		self.mesh.find_path_into(start, target, path)?;
		if let Some(drawing) = &mut self.drawing {
			let legs = path.windows(2).map(|pair| line(pair[0], pair[1], PATH_COLOR));
			drawing.show_for(|drawing| &mut drawing.paths, id, legs);
		}
		Ok(path)
	}

	/// Returns the mesh built at the latest update that changed an area.
	pub fn mesh(&self) -> &NavigationMesh {
		&self.mesh
	}

	/// Returns `velocity`, the velocity `agent` means to move at, adjusted by the least change that keeps it clear of
	/// the other agents and on the mesh.
	///
	/// Call it for each agent after [`Self::update`], with the velocity toward the agent's next path corner at the
	/// speed it goes, or zero when it means to stand; the result's direction is where to head and its length how fast.
	/// It changes continuously as the agents move, so turn the agent toward it at a bounded rate rather than snapping
	/// to it. An agent without a transform gets `velocity` back unchanged.
	pub fn steer(&self, agent: Handle, velocity: Vector) -> Vector {
		let id = agent.id();
		let Some((placed, position)) = self.agents_by_id.get(&id).and_then(|placed| Some((placed, placed.position?))) else {
			return velocity;
		};
		let NavigationAgent { radius, precedence } = placed.agent;
		steering::steer(&self.mesh, (position, radius, precedence), velocity, self.others(id))
	}

	/// Returns the position, velocity, radius, and precedence of every placed agent but the one with `id`.
	fn others(&self, id: u32) -> impl Iterator<Item = (Point, Vector, f32, f32)> + '_ {
		self.agents_by_id
			.iter()
			.filter(move |&(&other, _)| other != id)
			.filter_map(|(_, other)| {
				let NavigationAgent { radius, precedence } = other.agent;
				Some((other.position?, other.velocity, radius, precedence))
			})
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

/// How long, in seconds, agent velocities average displacements over.
const VELOCITY_AVERAGE_SECONDS: f32 = 0.25;

/// How far above the surface, in meters, debug lines are drawn.
const LINE_HEIGHT: f32 = 0.05;

const BOUNDARY_COLOR: RGBA = RGBA::new(0.2, 0.9, 1.0, 1.0);
const RADIUS_COLOR: RGBA = RGBA::new(0.7, 0.3, 1.0, 1.0);
const PATH_COLOR: RGBA = RGBA::new(1.0, 0.8, 0.1, 1.0);

#[cfg(test)]
mod tests {
	use math::{Orientation, Scale};

	use super::*;
	use crate::core::{
		channel::Channel as _,
		factory::Creator as _,
		message_bus::{MessageBus, MessageBusConfig},
	};

	fn navigation(world: &MessageScope) -> Navigation {
		Navigation::new(world, &[][..] as &[crate::application::Parameter])
	}

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
		let mut navigation = navigation(&world);
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

	/// Walks each agent at its velocity for a second of ticks, ending at its point, so its averaged velocity settles.
	///
	/// Create each agent where the walk starts, a second's travel before its point.
	fn walk(bus: &MessageBus, world: &MessageScope, navigation: &mut Navigation, walks: &[(Handle, [f32; 2], Vector)]) {
		for step in (0..WALK_TICKS).rev() {
			bus.begin_tick();
			for &(agent, [x, z], velocity) in walks {
				let behind = velocity * (step as f32 / WALK_TICKS as f32);
				moved(world, agent, x - behind.x(), z - behind.z());
			}
			navigation.update(tick()).expect("the agents walked");
		}
	}

	const WALK_TICKS: usize = 60;

	const WALK: f32 = 1.4;
	fn east() -> Vector {
		Vector::new(1.0, 0.0, 0.0)
	}

	#[test]
	fn the_mesh_follows_areas_as_they_are_created_moved_and_deleted() {
		let bus = MessageBus::new(MessageBusConfig::new(1 << 20)).expect("a valid bus");
		let world = bus.new_scope("world");
		let mut navigation = navigation(&world);

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
		let mut navigation = navigation(&world);
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
	fn an_agent_with_open_ground_keeps_its_velocity() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let walker = agent(&world, 0.0, 0.0);
		agent(&world, 0.0, 3.0);
		navigation.update(tick()).expect("two agents");

		assert_eq!(navigation.steer(walker, east() * WALK), east() * WALK);
		assert_eq!(
			navigation.steer(walker, Vector::new(0.0, 0.0, 0.0)),
			Vector::new(0.0, 0.0, 0.0)
		);
	}

	#[test]
	fn an_agent_steps_aside_from_a_standing_one_in_its_way_and_keeps_going() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let walker = agent(&world, 0.0, 0.0);
		agent(&world, 1.5, 0.0);
		navigation.update(tick()).expect("two agents");

		let steered = navigation.steer(walker, east() * WALK);
		assert!(steered.z().abs() > 0.0, "it steps aside instead of walking into the other");
		assert_eq!(steered.x(), WALK, "a sidestep is across its way, not against it");
	}

	#[test]
	fn an_agent_passes_on_the_side_it_is_already_offset_to() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let left = agent(&world, 0.0, 0.1);
		let right = agent(&world, 0.0, -0.1);
		agent(&world, 1.5, 0.0);
		navigation.update(tick()).expect("three agents");

		assert!(navigation.steer(left, east() * WALK).z() > 0.0);
		assert!(navigation.steer(right, east() * WALK).z() < 0.0);
	}

	#[test]
	fn an_agent_steps_aside_from_one_that_crosses_its_way_but_not_from_one_that_has_passed() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let walker = agent(&world, 0.0, 0.0);
		// Standing, the crosser would be 2 m aside. Walking north, it reaches the walker's line when the walker does.
		let crosser = agent(&world, 2.0, -2.0 - WALK);
		let passed = agent(&world, -1.0, -1.0 - WALK);
		let north = Vector::new(0.0, 0.0, WALK);
		walk(
			&bus,
			&world,
			&mut navigation,
			&[(crosser, [2.0, -2.0], north), (passed, [-1.0, -1.0], north)],
		);

		assert_ne!(navigation.steer(walker, east() * WALK), east() * WALK);
		world.channel::<DeleteMessage>().send(DeleteMessage::new(crosser));
		bus.begin_tick();
		navigation.update(tick()).expect("the crosser is gone");
		// The one behind and to the side moves away from the walker's way.
		assert_eq!(navigation.steer(walker, east() * WALK), east() * WALK);
	}

	#[test]
	fn standing_agents_too_close_step_apart_and_lone_ones_stay() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let alone = agent(&world, -3.0, 0.0);
		let crowded = agent(&world, 2.0, 0.0);
		agent(&world, 2.5, 0.0);
		navigation.update(tick()).expect("three agents");
		let still = Vector::new(0.0, 0.0, 0.0);

		assert_eq!(navigation.steer(alone, still), still);
		let step = navigation.steer(crowded, still);
		assert!(step.x() < 0.0, "it steps away from the one beside it");
		assert!(step.length() <= 2.0, "at a bounded speed");
	}

	#[test]
	fn an_agent_heading_at_the_edge_slows_to_a_slide_and_one_heading_along_it_keeps_going() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let walker = agent(&world, 4.9, 0.0);
		navigation.update(tick()).expect("an agent at the edge");
		let north = Vector::new(0.0, 0.0, WALK);

		// A tenth of a meter from the edge, most of the approach is given up; the rest goes the next time around.
		let into = navigation.steer(walker, east() * WALK);
		assert!(into.x() < 0.25 * WALK && into.x() >= 0.0);
		assert_eq!(navigation.steer(walker, north), north);
		// Approaching at a slant keeps the part along the edge.
		let along = WALK / 2f32.sqrt();
		let slant = navigation.steer(walker, Vector::new(along, 0.0, along));
		assert_eq!(slant.z(), along);
		assert!(slant.x() < 0.25 * WALK);

		// Off the mesh, heading back across the edge loses nothing.
		bus.begin_tick();
		let outside = agent(&world, 5.5, 3.0);
		navigation.update(tick()).expect("an agent off the mesh");
		assert_eq!(navigation.steer(outside, -east() * WALK), -east() * WALK);
	}

	#[test]
	fn an_agent_holds_its_course_for_those_of_less_precedence_and_they_step_aside_for_it() {
		let (bus, world, mut navigation) = floor();
		bus.begin_tick();
		let settled: Handle = world
			.create(NavigationAgent::new(0.35).with_precedence(1.0))
			.with(Transform::from_position(Point::new(2.0, 0.0, 0.0)))
			.into();
		let late = agent(&world, 2.5, 0.0);
		navigation.update(tick()).expect("two agents too close");
		let still = Vector::new(0.0, 0.0, 0.0);

		assert_eq!(navigation.steer(settled, still), still);
		assert!(navigation.steer(late, still).x() > 0.0, "the late one steps away");
	}

	#[test]
	fn an_agent_finds_its_path_from_where_it_stands_and_shows_it_when_drawing() {
		let bus = MessageBus::new(MessageBusConfig::new(1 << 20)).expect("a valid bus");
		let world = bus.new_scope("world");
		let mut navigation = Navigation::new(&world, &[crate::application::Parameter::new("debug", "true")][..]);
		let mut meshes = world.factory::<DebugMesh>().listener();
		bus.begin_tick();
		place(&world, NavigationArea::walkable(5.0, 5.0), Transform::identity());
		let walker = agent(&world, -3.0, 0.0);
		let unplaced: Handle = world.create(NavigationAgent::new(0.35)).into();
		navigation.update(tick()).expect("a floor and an agent");
		let mut path = Vec::new();

		// The target off the edge is pulled onto it.
		let found = navigation
			.find_path(walker, Point::new(7.0, 0.0, 0.0), &mut path)
			.expect("a path");
		assert_eq!(found, [Point::new(-3.0, 0.0, 0.0), Point::new(5.0, 0.0, 0.0)]);
		assert_eq!(
			navigation.find_path(unplaced, Point::new(0.0, 0.0, 0.0), &mut path),
			Err(NavigationPathError::StartOutsideMesh)
		);

		// Four boundary edges, the agent's radius, and one path segment were drawn.
		let mut drawn = 0;
		while meshes.read().is_some() {
			drawn += 1;
		}
		assert_eq!(drawn, 6);
	}

	#[test]
	fn an_unknown_agent_gets_its_velocity_back() {
		let (_bus, world, navigation) = floor();
		let unplaced: Handle = world.create(NavigationAgent::new(0.35)).into();

		assert_eq!(navigation.steer(unplaced, east() * 2.0), east() * 2.0);
	}
}

use std::collections::BTreeMap;

use math::{Point, UnitVector, Vector};
use utils::RGBA;

use super::{NavigationMesh, NavigationMeshBuildError, NavigationPathError, steering};
use crate::{
	MediaTime,
	application::parameters::Parameters,
	core::{
		channel::{Channel as _, DefaultChannel},
		factory::{CreateMessage, Factory, Handle},
		listener::{DefaultListener, Listener as _},
		message::DeleteMessage,
		message_bus::MessageScope,
	},
	gameplay::{Transform, TransformationUpdate},
	rendering::debug::{DebugMesh, DebugShape},
};
