//! The engine's level kinds: the payload each one reads and the component it publishes.
//!
//! Each payload type mirrors a section of the `.level` reference. Converters check what the engine's constructors
//! would otherwise assert, so a bad level fails its spawn with a message instead of panicking.

use std::f32::consts::PI;

use math::{Point, Radians, Vector};
use serde::Deserialize;

use super::{EntityRef, Spawner};
use crate::{
	gameplay::{Name, Transform, pathfinding::NavigationArea, transform::SerializableTransform},
	physics::{Body, BodyTypes, Constraint, LocalSpace, Shapes},
};

impl Spawner {
	/// Creates a spawner that knows the engine's kinds: `transform`, `name`, `mesh`, `directional_light`,
	/// `point_light`, `cone_light`, `body`, `navigation_area`, and `hinge`.
	///
	/// A build without rendering ignores `mesh` and the lights, so a server spawns the same levels as its clients.
	/// Next, register your application's kinds with [`Self::register`] or [`Self::register_with`].
	pub fn with_engine_kinds() -> Self {
		let mut spawner = Self::new();
		spawner
			.register_with("transform", |transform: SerializableTransform| Transform::try_from(transform))
			.expect(DISTINCT);
		spawner.register_with("name", name).expect(DISTINCT);
		spawner.register_with("body", body).expect(DISTINCT);
		spawner.register_with("navigation_area", navigation_area).expect(DISTINCT);
		spawner.register_with("hinge", hinge).expect(DISTINCT);
		#[cfg(feature = "headed")]
		{
			spawner.register_with("mesh", rendering::mesh).expect(DISTINCT);
			spawner
				.register_with("directional_light", rendering::directional_light)
				.expect(DISTINCT);
			spawner.register_with("point_light", rendering::point_light).expect(DISTINCT);
			spawner.register_with("cone_light", rendering::cone_light).expect(DISTINCT);
		}
		#[cfg(not(feature = "headed"))]
		for kind in ["mesh", "directional_light", "point_light", "cone_light"] {
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

	use super::{Radians, positive};
	use crate::rendering::{
		ConeLight, DirectionalLight, LightColor, PhotometricIntensity, PointLight, RenderableMesh,
		mesh::generator::{BoxMeshGenerator, CapsuleMeshGenerator, SphereMeshGenerator},
	};

	/// The `MeshPayload` enum is the authored form of a [`RenderableMesh`]: a resource or a generated shape.
	#[derive(Deserialize)]
	#[serde(rename_all = "snake_case", deny_unknown_fields)]
	pub(super) enum MeshPayload {
		Resource(String),
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

	/// The `ColorPayload` enum is the authored form of [`LightColor`].
	#[derive(Deserialize)]
	#[serde(rename_all = "snake_case", deny_unknown_fields)]
	pub(super) enum ColorPayload {
		Kelvin(f32),
		LinearSrgb([f32; 3]),
	}

	/// The `IntensityPayload` enum is the authored form of [`PhotometricIntensity`], with each distance and area 1 by
	/// default, or of an IES profile for a local light.
	#[derive(Deserialize)]
	#[serde(rename_all = "snake_case", deny_unknown_fields)]
	pub(super) enum IntensityPayload {
		Illuminance {
			lux: f32,
			#[serde(default = "super::one")]
			measurement_distance_m: f32,
		},
		LuminousIntensity {
			candela: f32,
			#[serde(default = "super::one")]
			reference_distance_m: f32,
		},
		LuminousFlux {
			lumens: f32,
			#[serde(default = "super::one")]
			directional_beam_area_m2: f32,
		},
		Luminance {
			nits: f32,
			#[serde(default = "super::one")]
			projected_area_m2: f32,
			#[serde(default = "super::one")]
			reference_distance_m: f32,
		},
		Ies {
			profile: String,
			dimmer: f32,
		},
	}

	impl From<ColorPayload> for LightColor {
		fn from(color: ColorPayload) -> Self {
			match color {
				ColorPayload::Kelvin(kelvin) => LightColor::kelvin(kelvin),
				ColorPayload::LinearSrgb([r, g, b]) => LightColor::linear_srgb(r, g, b),
			}
		}
	}

	/// The `Intensity` enum is what a light's `intensity` names: a photometric quantity, or a baked IES profile.
	enum Intensity {
		Photometric(PhotometricIntensity),
		Ies { profile: String, dimmer: f32 },
	}

	impl From<IntensityPayload> for Intensity {
		fn from(intensity: IntensityPayload) -> Self {
			Intensity::Photometric(match intensity {
				IntensityPayload::Illuminance {
					lux,
					measurement_distance_m,
				} => PhotometricIntensity::Illuminance {
					lux,
					measurement_distance_m,
				},
				IntensityPayload::LuminousIntensity {
					candela,
					reference_distance_m,
				} => PhotometricIntensity::LuminousIntensity {
					candela,
					reference_distance_m,
				},
				IntensityPayload::LuminousFlux {
					lumens,
					directional_beam_area_m2,
				} => PhotometricIntensity::LuminousFlux {
					lumens,
					directional_beam_area_m2,
				},
				IntensityPayload::Luminance {
					nits,
					projected_area_m2,
					reference_distance_m,
				} => PhotometricIntensity::Luminance {
					nits,
					projected_area_m2,
					reference_distance_m,
				},
				IntensityPayload::Ies { profile, dimmer } => return Intensity::Ies { profile, dimmer },
			})
		}
	}

	/// The `DirectionalLightPayload` struct is the authored form of a [`DirectionalLight`].
	#[derive(Deserialize)]
	#[serde(deny_unknown_fields)]
	pub(super) struct DirectionalLightPayload {
		color: ColorPayload,
		intensity: IntensityPayload,
		#[serde(default)]
		angular_radius: Option<f32>,
	}

	pub(super) fn directional_light(light: DirectionalLightPayload) -> Result<DirectionalLight, String> {
		let Intensity::Photometric(intensity) = light.intensity.into() else {
			return Err(
				"it names an IES profile, which only point and cone lights take. Give it a photometric intensity such as `illuminance`"
					.to_string(),
			);
		};
		let built = DirectionalLight::new(light.color.into(), intensity).map_err(|error| error.to_string())?;
		match light.angular_radius {
			Some(radius) => built
				.with_angular_radius(Radians::new(radius))
				.map_err(|error| error.to_string()),
			None => Ok(built),
		}
	}

	/// The `PointLightPayload` struct is the authored form of a [`PointLight`].
	#[derive(Deserialize)]
	#[serde(deny_unknown_fields)]
	pub(super) struct PointLightPayload {
		color: ColorPayload,
		intensity: IntensityPayload,
		#[serde(default)]
		shadow_near: Option<f32>,
		#[serde(default)]
		shadow_far: Option<f32>,
	}

	pub(super) fn point_light(light: PointLightPayload) -> Result<PointLight, String> {
		let (near, far) = shadow_range(light.shadow_near, light.shadow_far)?;
		let mut built = match light.intensity.into() {
			Intensity::Photometric(intensity) => PointLight::new(light.color.into(), intensity),
			Intensity::Ies { profile, dimmer } => PointLight::new_ies(light.color.into(), dimmer, profile),
		}
		.map_err(|error| error.to_string())?;
		if let Some(near) = near {
			built = built.with_shadow_near(near);
		}
		if let Some(far) = far {
			built = built.with_shadow_far(far);
		}
		Ok(built)
	}

	/// The `ConeLightPayload` struct is the authored form of a [`ConeLight`], with half angles in radians.
	#[derive(Deserialize)]
	#[serde(deny_unknown_fields)]
	pub(super) struct ConeLightPayload {
		color: ColorPayload,
		intensity: IntensityPayload,
		inner_angle: f32,
		outer_angle: f32,
		#[serde(default)]
		shadow_near: Option<f32>,
		#[serde(default)]
		shadow_far: Option<f32>,
	}

	pub(super) fn cone_light(light: ConeLightPayload) -> Result<ConeLight, String> {
		let (near, far) = shadow_range(light.shadow_near, light.shadow_far)?;
		let (inner, outer) = (Radians::new(light.inner_angle), Radians::new(light.outer_angle));
		let mut built = match light.intensity.into() {
			Intensity::Photometric(intensity) => ConeLight::new(light.color.into(), intensity, inner, outer),
			Intensity::Ies { profile, dimmer } => ConeLight::new_ies(light.color.into(), dimmer, profile, inner, outer),
		}
		.map_err(|error| error.to_string())?;
		if let Some(near) = near {
			built = built.with_shadow_near(near);
		}
		if let Some(far) = far {
			built = built.with_shadow_far(far);
		}
		Ok(built)
	}

	/// Checks a local light's shadow overrides: each above zero, and the near one below the far one when both are
	/// given.
	fn shadow_range(near: Option<f32>, far: Option<f32>) -> Result<(Option<f32>, Option<f32>), String> {
		let near = near.map(|near| positive(near, "shadow near distance")).transpose()?;
		let far = far.map(|far| positive(far, "shadow far distance")).transpose()?;
		if let (Some(near), Some(far)) = (near, far)
			&& near >= far
		{
			return Err(format!(
				"its shadow range runs from {near} m to {far} m. Give a `shadow_near` below `shadow_far`"
			));
		}
		Ok((near, far))
	}
}
