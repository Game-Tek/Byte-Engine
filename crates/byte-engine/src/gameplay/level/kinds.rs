//! The engine's level kinds: the payload each one reads and the component it publishes.
//!
//! Each payload type mirrors a section of the `.level` reference. Converters check what the engine's constructors
//! would otherwise assert, so a bad level fails its spawn with a message instead of panicking.

use std::f32::consts::PI;

use math::{Point, Radians, Vector};
use serde::Deserialize;

use super::{EntityRef, Spawner};
use crate::{
	gameplay::{Name, Waypoint, pathfinding::NavigationArea},
	physics::{Body, BodyTypes, Constraint, LocalSpace, Shapes},
};

impl Spawner {
	/// Creates a spawner that knows the engine's kinds: `name`, `mesh`, `directional_light`,
	/// `point_light`, `cone_light`, `lightmap`, `terrain`, `body`, `navigation_area`, `hinge`, and `waypoint`.
	///
	/// A build without rendering ignores `mesh`, the lights, `lightmap`, and `terrain`, so a server spawns the same levels as its
	/// clients.
	/// Next, register your application's kinds with [`Self::register`] or [`Self::register_with`].
	pub fn with_engine_kinds() -> Self {
		let mut spawner = Self::new();
		spawner.register_with("name", name).expect(DISTINCT);
		spawner.register_with("body", body).expect(DISTINCT);
		spawner.register_with("navigation_area", navigation_area).expect(DISTINCT);
		spawner.register_with("hinge", hinge).expect(DISTINCT);
		spawner.register::<Waypoint>("waypoint").expect(DISTINCT);
		#[cfg(feature = "headed")]
		{
			spawner.register_with("mesh", rendering::mesh).expect(DISTINCT);
			// A light reads its own payload, the one the inspector's light updates take too.
			spawner
				.register::<crate::rendering::DirectionalLight>("directional_light")
				.expect(DISTINCT);
			spawner
				.register::<crate::rendering::PointLight>("point_light")
				.expect(DISTINCT);
			spawner.register::<crate::rendering::ConeLight>("cone_light").expect(DISTINCT);
			spawner.register_with("lightmap", rendering::lightmap).expect(DISTINCT);
			spawner.register_with("terrain", rendering::terrain).expect(DISTINCT);
		}
		#[cfg(not(feature = "headed"))]
		for kind in [
			"mesh",
			"directional_light",
			"point_light",
			"cone_light",
			"lightmap",
			"terrain",
		] {
			spawner.ignore(kind).expect(DISTINCT);
		}
		spawner
	}
}

const DISTINCT: &str = "The engine's level kinds have distinct names.";

fn name(name: String) -> Result<Name, String> {
	if name.is_empty() {
		return Err("the name is empty. Give the entity a name or remove the component".to_string());
	}
	Ok(Name::new(name))
}

/// The `BodyPayload` struct is the authored form of a physics body.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BodyPayload {
	#[serde(rename = "type")]
	body_type: BodyTypePayload,
	shape: ShapePayload,
	#[serde(default = "one")]
	mass: f32,
	#[serde(default)]
	velocity: [f32; 3],
	#[serde(default = "default_elasticity")]
	elasticity: f32,
	#[serde(default = "default_friction")]
	friction: f32,
	#[serde(default = "yes")]
	rotates: bool,
}

fn one() -> f32 {
	1.0
}

fn default_elasticity() -> f32 {
	0.1
}

fn default_friction() -> f32 {
	0.5
}

fn yes() -> bool {
	true
}

/// The `BodyTypePayload` enum is the authored form of [`BodyTypes`].
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum BodyTypePayload {
	Static,
	Kinematic,
	Dynamic,
}

/// The `ShapePayload` enum is the authored form of [`Shapes`], apart from convex hulls, which need mesh data.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum ShapePayload {
	Sphere {
		radius: f32,
	},
	Cube {
		half_extents: [f32; 3],
	},
	Capsule {
		radius: f32,
		height: f32,
		#[serde(default)]
		center: [f32; 3],
	},
}

fn body(body: BodyPayload) -> Result<Body, String> {
	let shape = match body.shape {
		ShapePayload::Sphere { radius } => Shapes::sphere(positive(radius, "radius")?),
		ShapePayload::Cube { half_extents: [x, y, z] } => Shapes::cube(Vector::<LocalSpace>::new(
			positive(x, "half extent")?,
			positive(y, "half extent")?,
			positive(z, "half extent")?,
		)),
		ShapePayload::Capsule {
			radius,
			height,
			center: [x, y, z],
		} => Shapes::capsule(
			positive(radius, "radius")?,
			positive(height, "height")?,
			Point::<LocalSpace>::new(x, y, z),
		),
	};
	let body_type = match body.body_type {
		BodyTypePayload::Static => BodyTypes::Static,
		BodyTypePayload::Kinematic => BodyTypes::Kinematic,
		BodyTypePayload::Dynamic => BodyTypes::Dynamic,
	};
	let [x, y, z] = body.velocity;
	Ok(Body::new(body_type, shape)
		.with_mass(positive(body.mass, "mass")?)
		.with_velocity(Vector::new(x, y, z))
		.with_elasticity(body.elasticity)
		.with_friction(body.friction)
		.with_rotation(body.rotates))
}

/// The `NavigationAreaPayload` enum is the authored form of [`NavigationArea`].
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum NavigationAreaPayload {
	Walkable { half_extents: [f32; 2] },
	Obstacle { half_extents: [f32; 2] },
}

fn navigation_area(area: NavigationAreaPayload) -> Result<NavigationArea, String> {
	Ok(match area {
		NavigationAreaPayload::Walkable { half_extents: [x, z] } => {
			NavigationArea::walkable(positive(x, "half extent")?, positive(z, "half extent")?)
		}
		NavigationAreaPayload::Obstacle { half_extents: [x, z] } => {
			NavigationArea::obstacle(positive(x, "half extent")?, positive(z, "half extent")?)
		}
	})
}

/// The `HingePayload` struct is the authored form of a hinge [`Constraint`] between two bodies of the level.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HingePayload {
	first: EntityRef,
	second: EntityRef,
	axis: [f32; 3],
	#[serde(default)]
	limit: Option<[f32; 2]>,
}

fn hinge(hinge: HingePayload) -> Result<Constraint, String> {
	let [x, y, z] = hinge.axis;
	let axis = Vector::<LocalSpace>::new(x, y, z)
		.normalized()
		.map_err(|_| "the axis has no length. Give it a direction such as [0, 1, 0]".to_string())?;
	let limit = match hinge.limit {
		Some([start, end]) if (-PI..=PI).contains(&start) && start <= end && end <= PI => {
			Some(Radians::new(start)..=Radians::new(end))
		}
		Some(_) => {
			return Err(
				"the limit isn't a range of radians between -π and π. Give it as [start, end] in radians, start first"
					.to_string(),
			);
		}
		None => None,
	};
	Ok(Constraint::hinge(hinge.first.handle(), hinge.second.handle(), axis, limit))
}

/// Returns `value` when it's finite and above zero, or an error naming the field.
fn positive(value: f32, field: &str) -> Result<f32, String> {
	if value.is_finite() && value > 0.0 {
		Ok(value)
	} else {
		Err(format!("a {field} is {value}. Give it a finite length above zero"))
	}
}

/// The kinds that publish rendering components, which only builds with rendering have.
#[cfg(feature = "headed")]
mod rendering {
	use maths_rs::Vec3f;
	use serde::Deserialize;

	use super::positive;
	use crate::gameplay::{Transform, transform::SerializableTransform};
	use crate::rendering::{
		Lightmap, RenderableMesh, RenderableTerrain,
		mesh::generator::{BoxMeshGenerator, CapsuleMeshGenerator, SphereMeshGenerator},
	};

	/// The `MeshPayload` enum is the authored form of a [`RenderableMesh`]: a resource or a generated shape.
	#[derive(Deserialize)]
	#[serde(rename_all = "snake_case", deny_unknown_fields)]
	pub(super) enum MeshPayload {
		Resource(String),
		/// Copies of one resource mesh, drawn as one. The level bake writes it for placements it clusters.
		Merged {
			resource: String,
			placements: Vec<SerializableTransform>,
		},
		Box {
			half_extents: [f32; 3],
			#[serde(default)]
			material: Option<String>,
			#[serde(default)]
			meter_uvs: bool,
		},
		Sphere {
			radius: f32,
		},
		Capsule {
			radius: f32,
			height: f32,
			#[serde(default)]
			material: Option<String>,
		},
	}

	pub(super) fn mesh(mesh: MeshPayload) -> Result<RenderableMesh, String> {
		Ok(match mesh {
			MeshPayload::Resource(id) => RenderableMesh::resource(id),
			MeshPayload::Merged { resource, placements } => RenderableMesh::merged(
				resource,
				placements
					.into_iter()
					.map(Transform::try_from)
					.collect::<Result<Vec<_>, _>>()?,
			),
			MeshPayload::Box {
				half_extents: [x, y, z],
				material,
				meter_uvs,
			} => {
				let size = Vec3f::new(
					positive(x, "half extent")?,
					positive(y, "half extent")?,
					positive(z, "half extent")?,
				);
				let mut generator = BoxMeshGenerator::from_size(size);
				if let Some(material) = material {
					generator = generator.with_material(material);
				}
				if meter_uvs {
					generator = generator.with_meter_uvs();
				}
				RenderableMesh::generated(Box::new(generator))
			}
			MeshPayload::Sphere { radius } => {
				RenderableMesh::generated(Box::new(SphereMeshGenerator::from_radius(positive(radius, "radius")?)))
			}
			MeshPayload::Capsule {
				radius,
				height,
				material,
			} => {
				let mut generator = CapsuleMeshGenerator::new(positive(radius, "radius")?, positive(height, "height")?);
				if let Some(material) = material {
					generator = generator.with_material(material);
				}
				RenderableMesh::generated(Box::new(generator))
			}
		})
	}

	/// Turns the ID a `lightmap` kind names into the component that loads it.
	pub(super) fn lightmap(resource_id: String) -> Result<Lightmap, String> {
		if resource_id.is_empty() {
			return Err("the lightmap ID is empty. Name the `.lightmap.bead` asset baked from this level".to_string());
		}
		Ok(Lightmap::new(resource_id))
	}

	/// Turns the ID a `terrain` kind names into the component that draws it.
	pub(super) fn terrain(resource_id: String) -> Result<RenderableTerrain, String> {
		if resource_id.is_empty() {
			return Err("the terrain ID is empty. Name the `.terrain.bead` asset to draw".to_string());
		}
		Ok(RenderableTerrain::resource(resource_id))
	}
}
