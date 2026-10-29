//! Run with `cargo bench -p byte-engine --bench level`.
//!
//! Spawns a baked level of many props and compares it with creating the same props in code, so a change to the
//! level format or the spawner shows its cost per entity. Large levels decode on the fast cores, so compare runs
//! on the same machine.

use byte_engine::{
	core::{Creator, message_bus::MessageScope},
	gameplay::{Spawner, Transform},
	math::{Point, Vector},
	physics::{Body, BodyTypes, LocalSpace, Shapes},
	rendering::RenderableMesh,
};
use divan::{Bencher, counter::ItemsCount};
use resource_management::asset::handler::implementations::level::bake_level;

/// Entities in the benchmark level, each a prop with four components.
const PROPS: usize = 65_536;

/// The `Tag` struct stands in for an application kind.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, reason = "The benchmark decodes and publishes tags; nothing reads them.")]
struct Tag {
	value: u32,
}

fn main() {
	divan::main();
}

/// Writes a level of `PROPS` props on a grid, each with a transform, a body, a mesh, and a tag.
fn level_source() -> String {
	let mut source = String::from("{ entities: [\n");
	for prop in 0..PROPS {
		let (x, z) = ((prop % 256) as f32 * 2.5, (prop / 256) as f32 * 2.5);
		source.push_str(&format!(
			"{{ components: {{ transform: {{ position: [{x}, 0.5, {z}] }}, body: {{ type: \"static\", shape: {{ cube: {{ half_extents: [0.5, 0.5, 0.5] }} }} }}, mesh: {{ resource: \"props/crate.glb\" }}, tag: {{ value: {prop} }} }} }},\n"
		));
	}
	source.push_str("] }");
	source
}

#[divan::bench(sample_count = 20)]
fn spawn_baked_level(bencher: Bencher) {
	let (level, payload) = bake_level(level_source().as_bytes()).expect("the benchmark level should bake");
	let mut spawner = Spawner::with_engine_kinds();
	spawner.register::<Tag>("tag").unwrap();

	bencher
		.counter(ItemsCount::new(PROPS))
		// A fresh scope per sample, so earlier samples' messages never fill its pool.
		.with_inputs(|| MessageScope::standalone("world"))
		.bench_local_values(|world| {
			spawner.spawn(&world, &level, &payload).unwrap();
			world
		});
}

#[divan::bench(sample_count = 20)]
fn create_props_in_code(bencher: Bencher) {
	bencher
		.counter(ItemsCount::new(PROPS))
		.with_inputs(|| MessageScope::standalone("world"))
		.bench_local_values(|world| {
			for prop in 0..PROPS {
				let (x, z) = ((prop % 256) as f32 * 2.5, (prop / 256) as f32 * 2.5);
				world
					.create(Transform::from_position(Point::new(x, 0.5, z)))
					.with(Body::new(
						BodyTypes::Static,
						Shapes::cube(Vector::<LocalSpace>::new(0.5, 0.5, 0.5)),
					))
					.with(RenderableMesh::resource("props/crate.glb"))
					.with(Tag { value: prop as u32 });
			}
			world
		});
}

/// Writes a level of `PROPS` entities that each hold only `component`, or nothing for `none`.
fn single_kind_source(component: &str) -> String {
	let mut source = String::from("{ entities: [\n");
	for prop in 0..PROPS {
		let (x, z) = ((prop % 256) as f32 * 2.5, (prop / 256) as f32 * 2.5);
		let payload = match component {
			"transform" => format!("transform: {{ position: [{x}, 0.5, {z}] }}"),
			"body" => "body: { type: \"static\", shape: { cube: { half_extents: [0.5, 0.5, 0.5] } } }".to_string(),
			"mesh" => "mesh: { resource: \"props/crate.glb\" }".to_string(),
			"tag" => format!("tag: {{ value: {prop} }}"),
			_ => String::new(),
		};
		source.push_str(&format!("{{ components: {{ {payload} }} }},\n"));
	}
	source.push_str("] }");
	source
}

/// Splits the level's cost by kind; `none` is the cost of the entities and their scene membership alone.
#[divan::bench(sample_count = 20, args = ["none", "transform", "body", "mesh", "tag"])]
fn spawn_one_kind(bencher: Bencher, component: &str) {
	let (level, payload) = bake_level(single_kind_source(component).as_bytes()).unwrap();
	let mut spawner = Spawner::with_engine_kinds();
	spawner.register::<Tag>("tag").unwrap();
	bencher
		.counter(ItemsCount::new(PROPS))
		.with_inputs(|| MessageScope::standalone("world"))
		.bench_local_values(|world| {
			spawner.spawn(&world, &level, &payload).unwrap();
			world
		});
}
