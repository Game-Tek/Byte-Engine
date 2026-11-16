use math::{Orientation, Point, Scale};
use resource_management::asset::handler::implementations::{level::bake_level, prefab::bake_prefab};

use super::{EntityRef, Loaded, Placement, Spawner};
use crate::{
	core::{
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		message_bus::MessageScope,
	},
	gameplay::{Name, Scene, SceneGraph, SceneNode, Transform},
	physics::{Body, BodyTypes, Constraint},
};

/// The `Team` struct is an application kind published as it's authored.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Team {
	number: u32,
}

/// The `FollowPayload` struct is the authored form of [`Follow`], naming its target by id.
#[derive(serde::Deserialize)]
struct FollowPayload {
	target: EntityRef,
}

/// The `Follow` struct is an application component that holds another entity's handle.
#[derive(Clone, Debug, PartialEq)]
struct Follow(Handle);

fn spawner() -> Spawner {
	let mut spawner = Spawner::with_engine_kinds();
	spawner.register::<Team>("team").unwrap();
	spawner
		.register_with("follow", |follow: FollowPayload| Ok(Follow(follow.target.handle())))
		.unwrap();
	spawner
}

/// Bakes `source` and spawns it into `world`.
fn spawn(spawner: &Spawner, world: &MessageScope, source: &str) -> Result<Handle, String> {
	let (level, payload) = bake_level(source.as_bytes()).expect("the test level should bake");
	spawner.spawn(world, &Loaded::level("test.level", level, payload), Placement::default())
}

/// Bakes the prefab `source` in memory.
fn prefab(source: &str) -> (super::Prefab, Vec<u8>) {
	bake_prefab(source.as_bytes()).expect("the test prefab should bake")
}

/// Reads every creation `listener` holds, as handle and value.
fn drain<T: Clone + Send + Sync + 'static>(listener: &mut DefaultListener<CreateMessage<T>>) -> Vec<(Handle, T)> {
	std::iter::from_fn(|| listener.read())
		.map(|creation| (creation.handle(), creation.data().clone()))
		.collect()
}

#[test]
fn every_component_of_an_entity_arrives_under_one_handle() {
	let world = MessageScope::standalone("world");
	let mut transforms = world.factory::<Transform>().listener();
	let mut teams = world.factory::<Team>().listener();
	let mut names = world.factory::<Name>().listener();

	spawn(
		&spawner(),
		&world,
		r#"{ entities: [
			{ components: { name: "red", transform: { position: [1, 2, 3] }, team: { number: 1 } } },
			{ components: { name: "blue", team: { number: 2 } } },
		] }"#,
	)
	.unwrap();

	let names = drain(&mut names);
	let teams = drain(&mut teams);
	let transforms = drain(&mut transforms);
	assert_eq!(names.len(), 2);
	assert_eq!(teams, [(names[0].0, Team { number: 1 }), (names[1].0, Team { number: 2 })]);
	assert_eq!(names[0].1.as_str(), "red");
	assert_eq!(
		transforms,
		[(names[0].0, Transform::from_position(Point::new(1.0, 2.0, 3.0)))],
		"a transform's missing fields take the identity's values"
	);
}

#[test]
fn entities_belong_to_the_level_or_their_parent() {
	let world = MessageScope::standalone("world");
	let mut scenes = world.factory::<Scene>().listener();
	let mut members = world.factory::<SceneNode>().listener();
	let mut names = world.factory::<Name>().listener();

	let root = spawn(
		&spawner(),
		&world,
		r#"{ entities: [
			{ parent: "frame", components: { name: "door" } },
			{ id: "frame", components: { name: "frame" } },
		] }"#,
	)
	.unwrap();

	assert_eq!(drain(&mut scenes), [(root, Scene)]);
	let handle_of = |names: &[(Handle, Name)], name: &str| names.iter().find(|(_, n)| n.as_str() == name).unwrap().0;
	let names = drain(&mut names);
	let (door, frame) = (handle_of(&names, "door"), handle_of(&names, "frame"));
	assert_eq!(
		drain(&mut members),
		[(frame, SceneNode::under(root)), (door, SceneNode::under(frame))],
		"parents are published before their children"
	);
}

#[test]
fn entity_references_hold_the_referenced_entity_handle() {
	let world = MessageScope::standalone("world");
	let mut follows = world.factory::<Follow>().listener();
	let mut names = world.factory::<Name>().listener();

	spawn(
		&spawner(),
		&world,
		r#"{ entities: [
			{ components: { name: "hound", follow: { target: { "$entity": "fox" } } } },
			{ id: "fox", components: { name: "fox" } },
		] }"#,
	)
	.unwrap();

	let names = drain(&mut names);
	let [(hound, _), (fox, _)] = names.as_slice() else {
		panic!("both entities should be named");
	};
	assert_eq!(drain(&mut follows), [(*hound, Follow(*fox))]);
}

#[test]
fn a_level_that_cannot_spawn_publishes_nothing() {
	for (problem, source) in [
		(
			"unregistered kind",
			r#"{ entities: [{ components: { name: "a", score: 3 } }] }"#,
		),
		(
			"payload of the wrong shape",
			r#"{ entities: [{ components: { name: "a", team: { number: "one" } } }] }"#,
		),
		(
			"payload with an unknown field",
			r#"{ entities: [{ components: { name: "a", team: { number: 1, color: 2 } } }] }"#,
		),
		(
			"payload the converter rejects",
			r#"{ entities: [{ components: { name: "" } }] }"#,
		),
	] {
		let world = MessageScope::standalone("world");
		let mut scenes = world.factory::<Scene>().listener();
		let mut names = world.factory::<Name>().listener();

		let error = spawn(&spawner(), &world, source).expect_err(problem);

		assert!(error.starts_with("'test.level' could not be spawned."), "{problem}: {error}");
		assert!(
			drain(&mut scenes).is_empty() && drain(&mut names).is_empty(),
			"{problem} published something"
		);
	}
}

#[test]
fn a_failed_component_is_named_by_its_entity_and_kind() {
	let world = MessageScope::standalone("world");

	let error = spawn(
		&spawner(),
		&world,
		r#"{ entities: [{ id: "keeper", components: { team: { number: -1 } } }] }"#,
	)
	.unwrap_err();

	assert!(error.contains("`team` component of entity 'keeper'"), "{error}");
}

#[test]
fn deleting_the_root_deletes_every_entity() {
	let world = MessageScope::standalone("world");
	let mut scenes = SceneGraph::new(&world);
	let deletes: DefaultChannel<DeleteMessage> = world.channel();
	let mut deleted = deletes.listener();
	let mut names = world.factory::<Name>().listener();

	let root = spawn(
		&spawner(),
		&world,
		r#"{ entities: [
			{ id: "a", components: { name: "a" } },
			{ parent: "a", components: { name: "b" } },
		] }"#,
	)
	.unwrap();
	deletes.send(DeleteMessage::new(root));
	scenes.cascade();

	let mut deleted = std::iter::from_fn(|| deleted.read())
		.map(|deletion| deletion.into_handle())
		.collect::<Vec<_>>();
	let mut expected = drain(&mut names).into_iter().map(|(handle, _)| handle).collect::<Vec<_>>();
	expected.push(root);
	deleted.sort_by_key(|handle| handle.id());
	expected.sort_by_key(|handle| handle.id());
	assert_eq!(deleted, expected);
}

#[test]
fn engine_kinds_read_the_payloads_the_reference_specifies() {
	let world = MessageScope::standalone("world");
	let mut bodies = world.factory::<Body>().listener();
	let mut hinges = world.factory::<Constraint>().listener();
	let mut areas = world.factory::<crate::gameplay::pathfinding::NavigationArea>().listener();

	spawn(
		&spawner(),
		&world,
		r#"{ entities: [
			{ id: "frame", components: {
				body: { type: "static", shape: { cube: { half_extents: [0.1, 1, 0.5] } } },
				navigation_area: { obstacle: { half_extents: [0.5, 0.5] } },
			} },
			{ id: "door", components: {
				body: { type: "dynamic", shape: { capsule: { radius: 0.2, height: 2 } }, mass: 20, rotates: false },
			} },
			{ components: { hinge: { first: { "$entity": "frame" }, second: { "$entity": "door" }, axis: [0, 1, 0], limit: [-1.5, 1.5] } } },
		] }"#,
	)
	.unwrap();

	let bodies = drain(&mut bodies);
	assert_eq!(bodies.len(), 2);
	assert_eq!(bodies[0].1.body_type(), BodyTypes::Static);
	assert_eq!(bodies[1].1.body_type(), BodyTypes::Dynamic);
	assert_eq!(drain(&mut areas).len(), 1);
	assert_eq!(drain(&mut hinges).len(), 1);
}

#[cfg(feature = "headed")]
#[test]
fn rendering_kinds_read_the_payloads_the_reference_specifies() {
	use crate::rendering::{ConeLight, DirectionalLight, PointLight, RenderableMesh, RenderableTerrain};

	let world = MessageScope::standalone("world");
	let mut meshes = world.factory::<RenderableMesh>().listener();
	let mut terrains = world.factory::<RenderableTerrain>().listener();
	let mut suns = world.factory::<DirectionalLight>().listener();
	let mut lamps = world.factory::<PointLight>().listener();
	let mut spots = world.factory::<ConeLight>().listener();

	spawn(
		&spawner(),
		&world,
		r#"{ entities: [
			{ components: { mesh: { resource: "props/crate.glb" } } },
			{ components: { mesh: { box: { half_extents: [1, 0.5, 1], material: "floor.bema", meter_uvs: true } } } },
			{ components: { mesh: { sphere: { radius: 0.5 } } } },
			{ components: { mesh: { capsule: { radius: 0.3, height: 1.8 } } } },
			{ components: { directional_light: { color: { kelvin: 6500 }, intensity: { illuminance: { lux: 100000 } }, angular_radius: 0.0047 } } },
			{ components: { point_light: { color: { linear_srgb: [1, 0.8, 0.6] }, intensity: { luminous_flux: { lumens: 800 } } } } },
			{ components: { point_light: { color: { kelvin: 3000 }, intensity: { ies: { profile: "lamp.ies", dimmer: 0.5 } }, shadow_far: 12 } } },
			{ components: { cone_light: { color: { kelvin: 4000 }, intensity: { luminous_intensity: { candela: 900 } }, inner_angle: 0.3, outer_angle: 0.5, shadow_near: 0.2, shadow_far: 20 } } },
			{ components: { cone_light: { color: { kelvin: 4000 }, intensity: { ies: { profile: "spot.ies", dimmer: 1 } }, inner_angle: 0.6, outer_angle: 0.8 } } },
			{ components: { terrain: "valley.terrain.bead" } },
		] }"#,
	)
	.unwrap();

	assert_eq!(drain(&mut meshes).len(), 4);
	let terrains = drain(&mut terrains);
	assert_eq!(terrains.len(), 1);
	assert_eq!(&**terrains[0].1.resource_id(), "valley.terrain.bead");
	assert_eq!(drain(&mut suns).len(), 1);

	let lamps = drain(&mut lamps);
	assert_eq!(lamps.len(), 2);
	let bulb = &lamps[1].1.emission;
	assert_eq!(
		bulb.ies_profile().map(|profile| (profile.resource_id(), profile.dimmer())),
		Some(("lamp.ies", 0.5))
	);
	assert_eq!((bulb.shadow_near_override(), bulb.shadow_far_override()), (None, Some(12.0)));

	let spots = drain(&mut spots);
	assert_eq!(spots.len(), 2);
	let (photometric, profiled) = (&spots[0].1, &spots[1].1);
	assert_eq!((photometric.inner_angle.value(), photometric.outer_angle.value()), (0.3, 0.5));
	assert_eq!(
		(
			photometric.emission.shadow_near_override(),
			photometric.emission.shadow_far_override()
		),
		(Some(0.2), Some(20.0))
	);
	assert!(photometric.emission.ies_profile().is_none());
	assert_eq!(
		profiled.emission.ies_profile().map(|profile| profile.resource_id()),
		Some("spot.ies")
	);
}

#[cfg(feature = "headed")]
#[test]
fn lights_that_cannot_take_shape_fail_the_spawn() {
	for (problem, light) in [
		(
			"inner angle not below the outer one",
			r#"cone_light: { color: { kelvin: 4000 }, intensity: { luminous_intensity: { candela: 900 } }, inner_angle: 0.5, outer_angle: 0.5 }"#,
		),
		(
			"outer angle past π",
			r#"cone_light: { color: { kelvin: 4000 }, intensity: { luminous_intensity: { candela: 900 } }, inner_angle: 0.5, outer_angle: 3.5 }"#,
		),
		(
			"IES dimmer above 1",
			r#"point_light: { color: { kelvin: 3000 }, intensity: { ies: { profile: "lamp.ies", dimmer: 1.5 } } }"#,
		),
		(
			"empty IES profile",
			r#"point_light: { color: { kelvin: 3000 }, intensity: { ies: { profile: "", dimmer: 0.5 } } }"#,
		),
		(
			"shadow range that ends before it starts",
			r#"point_light: { color: { kelvin: 3000 }, intensity: { luminous_flux: { lumens: 800 } }, shadow_near: 5, shadow_far: 2 }"#,
		),
		(
			"directional light with an IES profile",
			r#"directional_light: { color: { kelvin: 6500 }, intensity: { ies: { profile: "sun.ies", dimmer: 1 } } }"#,
		),
		(
			"angular radius past π/2",
			r#"directional_light: { color: { kelvin: 6500 }, intensity: { illuminance: { lux: 1000 } }, angular_radius: 2 }"#,
		),
	] {
		let world = MessageScope::standalone("world");
		let mut scenes = world.factory::<Scene>().listener();

		let error = spawn(
			&spawner(),
			&world,
			&format!("{{ entities: [{{ id: \"lamp\", components: {{ {light} }} }}] }}"),
		)
		.expect_err(problem);

		assert!(error.contains("component of entity 'lamp'"), "{problem}: {error}");
		assert!(drain(&mut scenes).is_empty(), "{problem} published something");
	}
}

/// Writes a level of `count` entities, each with a `team` numbered by its position, and `extra` as one more entity.
fn many_teams(count: u32, extra: &str) -> String {
	let teams = (0..count)
		.map(|number| format!("{{ components: {{ team: {{ number: {number} }} }} }}"))
		.collect::<Vec<_>>()
		.join(",\n");
	format!("{{ entities: [{teams}, {extra}] }}")
}

#[test]
fn large_levels_publish_every_component_in_entity_order() {
	let world = MessageScope::standalone("world");
	let mut teams = world.factory::<Team>().listener();

	spawn(&spawner(), &world, &many_teams(2_500, "{}")).unwrap();

	let teams = drain(&mut teams);
	assert_eq!(teams.len(), 2_500);
	assert!(
		teams
			.iter()
			.enumerate()
			.all(|(position, (_, team))| team.number == position as u32)
	);
	assert!(
		teams.windows(2).all(|pair| pair[1].0.id() == pair[0].0.id() + 1),
		"the entities of a level spawn on consecutive handles"
	);
}

#[test]
fn a_failure_far_into_a_level_names_its_entity() {
	let world = MessageScope::standalone("world");
	let mut teams = world.factory::<Team>().listener();

	let error = spawn(
		&spawner(),
		&world,
		&many_teams(2_500, r#"{ id: "late", components: { team: { number: "last" } } }"#),
	)
	.unwrap_err();

	assert!(error.contains("`team` component of entity 'late'"), "{error}");
	assert!(drain(&mut teams).is_empty());
}

/// A prefab whose `team` kind takes its number from a parameter, at an offset from its instance.
const SOLDIER: &str = r#"{
	parameters: { number: 7 },
	entities: [
		{ id: "body", components: { name: "soldier", transform: { position: [0, 1, 0] }, team: { number: { "$parameter": "number" } } } },
		{ parent: "body", components: { name: "helmet", transform: { position: [0, 2, 0] } } },
	],
}"#;

#[test]
fn a_prefab_spawns_at_its_placement_with_the_parameters_code_sets() {
	let world = MessageScope::standalone("world");
	let mut transforms = world.factory::<Transform>().listener();
	let mut teams = world.factory::<Team>().listener();
	let mut members = world.factory::<SceneNode>().listener();
	let (soldier, payload) = prefab(SOLDIER);
	let loaded = Loaded::prefab("soldier.prefab", soldier, payload);
	let spawner = spawner();

	let quarter_turn = Orientation::try_from_array([0.0, std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2]).unwrap();
	let placement = Transform::new(Point::new(10.0, 0.0, 0.0), Scale::new(2.0, 2.0, 2.0), quarter_turn);
	let first = spawner.spawn(&world, &loaded, Placement::at(placement.clone())).unwrap();
	let second = spawner
		.spawn(
			&world,
			&loaded,
			Placement::default().with_parameters(&serde_json::json!({ "number": 3 })).unwrap(),
		)
		.unwrap();

	let transforms = drain(&mut transforms);
	let world_of = |root: Handle, entity: u32| {
		let handle = super::level_entity_handle_from_root(root, entity);
		transforms.iter().find(|(h, _)| *h == handle).unwrap().1.clone()
	};
	assert!(
		world_of(first, 1).get_position().distance_to(Point::new(10.0, 4.0, 0.0)) < 1e-5,
		"the placement scales and moves the entity"
	);
	assert_eq!(world_of(first, 0).get_orientation(), quarter_turn);
	assert_eq!(world_of(second, 0).get_position(), Point::new(0.0, 1.0, 0.0));
	assert_eq!(
		drain(&mut teams).into_iter().map(|(_, team)| team.number).collect::<Vec<_>>(),
		[7, 3],
		"an unset parameter takes its default"
	);
	let members = drain(&mut members);
	assert!(members.contains(&(super::level_entity_handle_from_root(first, 0), SceneNode::under(first))));
}

#[test]
fn an_instance_spawns_its_prefab_under_itself_at_its_transform() {
	let world = MessageScope::standalone("world");
	let mut transforms = world.factory::<Transform>().listener();
	let mut teams = world.factory::<Team>().listener();
	let mut members = world.factory::<SceneNode>().listener();
	let mut names = world.factory::<Name>().listener();
	let (level, payload) = bake_level(
		br#"{ entities: [
			{ components: { name: "post", transform: { position: [5, 0, 0] } }, prefab: { resource: "soldier.prefab", parameters: { number: 2 } } },
			{ components: { name: "gate" } },
		] }"#,
	)
	.unwrap();
	let (soldier, soldier_payload) = prefab(SOLDIER);
	let loaded = Loaded::level("test.level", level, payload).with_prefab("soldier.prefab", soldier, soldier_payload);

	let root = spawner().spawn(&world, &loaded, Placement::default()).unwrap();

	let names = drain(&mut names);
	let handle_of = |name: &str| names.iter().find(|(_, n)| n.as_str() == name).unwrap().0;
	let (post, body, helmet) = (handle_of("post"), handle_of("soldier"), handle_of("helmet"));
	assert_eq!(handle_of("gate").id(), post.id() + 1, "level entities come before the prefab's");
	assert_eq!(body.id(), post.id() + 2);
	assert_eq!(
		drain(&mut members),
		[
			(post, SceneNode::under(root)),
			(handle_of("gate"), SceneNode::under(root)),
			(body, SceneNode::under(post)),
			(helmet, SceneNode::under(body)),
		]
	);
	let transforms = drain(&mut transforms);
	let position_of = |handle: Handle| transforms.iter().find(|(h, _)| *h == handle).unwrap().1.get_position();
	assert_eq!(position_of(body), Point::new(5.0, 1.0, 0.0));
	assert_eq!(position_of(helmet), Point::new(5.0, 2.0, 0.0));
	assert_eq!(drain(&mut teams), [(body, Team { number: 2 })]);
}

#[test]
fn a_nested_prefab_takes_the_parameters_its_enclosing_prefab_passes_on() {
	let world = MessageScope::standalone("world");
	let mut teams = world.factory::<Team>().listener();
	let (squad, squad_payload) = prefab(
		r#"{
			parameters: { number: 1 },
			entities: [
				{ prefab: { resource: "soldier.prefab", parameters: { number: { "$parameter": "number" } } } },
				{ prefab: { resource: "soldier.prefab" } },
			],
		}"#,
	);
	let (soldier, soldier_payload) = prefab(SOLDIER);
	let loaded = Loaded::prefab("squad.prefab", squad, squad_payload).with_prefab("soldier.prefab", soldier, soldier_payload);

	spawner()
		.spawn(
			&world,
			&loaded,
			Placement::default().with_parameters(&serde_json::json!({ "number": 9 })).unwrap(),
		)
		.unwrap();

	assert_eq!(drain(&mut teams).into_iter().map(|(_, team)| team.number).collect::<Vec<_>>(), [9, 7]);
}

#[test]
fn a_prefab_spawn_that_cannot_complete_publishes_nothing() {
	let (soldier, soldier_payload) = prefab(SOLDIER);
	let (looping, looping_payload) = prefab(r#"{ entities: [{ prefab: { resource: "loop.prefab" } }] }"#);
	let (broken, broken_payload) = prefab(r#"{ entities: [{ id: "bad", components: { team: { number: "x" } } }] }"#);
	let level = |source: &str| {
		let (level, payload) = bake_level(source.as_bytes()).unwrap();
		Loaded::level("test.level", level, payload)
			.with_prefab("soldier.prefab", soldier.clone(), soldier_payload.clone())
			.with_prefab("loop.prefab", looping.clone(), looping_payload.clone())
			.with_prefab("broken.prefab", broken.clone(), broken_payload.clone())
	};
	for (problem, source, cause) in [
		("unknown parameter", r#"{ entities: [{ prefab: { resource: "soldier.prefab", parameters: { rank: 1 } } }] }"#, "doesn't declare"),
		("missing prefab", r#"{ entities: [{ prefab: { resource: "absent.prefab" } }] }"#, "isn't loaded"),
		("prefab that places itself", r#"{ entities: [{ prefab: { resource: "loop.prefab" } }] }"#, "nest more than"),
		("prefab that doesn't decode", r#"{ entities: [{ components: { name: "a" } }, { prefab: { resource: "broken.prefab" } }] }"#, "entity 'bad' of 'broken.prefab'"),
	] {
		let world = MessageScope::standalone("world");
		let mut scenes = world.factory::<Scene>().listener();
		let mut names = world.factory::<Name>().listener();

		let error = spawner().spawn(&world, &level(source), Placement::default()).expect_err(problem);

		assert!(error.contains(cause), "{problem}: {error}");
		assert!(drain(&mut scenes).is_empty() && drain(&mut names).is_empty(), "{problem} published something");
	}
}

#[test]
fn deleting_an_instance_deletes_its_prefab() {
	let world = MessageScope::standalone("world");
	let mut scenes = SceneGraph::new(&world);
	let deletes: DefaultChannel<DeleteMessage> = world.channel();
	let mut deleted = deletes.listener();
	let (level, payload) = bake_level(br#"{ entities: [{ id: "post", prefab: { resource: "soldier.prefab" } }] }"#).unwrap();
	let (soldier, soldier_payload) = prefab(SOLDIER);
	let loaded = Loaded::level("test.level", level, payload).with_prefab("soldier.prefab", soldier, soldier_payload);

	let root = spawner().spawn(&world, &loaded, Placement::default()).unwrap();
	let post = super::level_entity_handle_from_root(root, 0);
	deletes.send(DeleteMessage::new(post));
	scenes.cascade();

	let mut deleted = std::iter::from_fn(|| deleted.read()).map(|deletion| deletion.into_handle().id()).collect::<Vec<_>>();
	deleted.sort();
	assert_eq!(deleted, [post.id(), post.id() + 1, post.id() + 2]);
}
