//! Detail level selection: which of a primitive's baked meshlet ranges each instance draws this frame.
//!
//! A baked primitive stores its levels as consecutive meshlet ranges over the same vertices, so drawing another level
//! only changes the meshlet range an instance names. [`MeshLods`] carries those ranges from the loaded mesh to each
//! render entity, and [`LodView`] picks one per instance from the main camera. Shadow views draw the same level, which
//! keeps a caster and its lit surface in agreement.

/// Configuration namespace of the runtime detail level controls.
pub const LOD_CONFIGURATION_PREFIX: &str = "render.lod.";

/// Error, in pixels, a level may show on screen at the default bias before a finer one is drawn.
const LOD_PIXEL_ERROR: f32 = 1.0;

/// The `MeshLod` struct names the meshlet range an instance draws for one detail level.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MeshLod {
	/// First meshlet of the level, counted from the primitive's first meshlet.
	pub(crate) first_meshlet: u32,
	pub(crate) meshlet_count: u32,
	/// Estimated largest distance, in model-space units, between this level's surface and full detail.
	pub(crate) error: f32,
}

/// The `MeshLods` struct keeps a primitive's detail levels, finest first, in storage the frame loop can copy freely.
///
/// A fixed table avoids a heap allocation per render entity. Build one with [`Self::from_resource`] for a baked
/// primitive or [`Self::single`] for geometry without coarser levels, then let [`LodView::select`] choose from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MeshLods {
	levels: [MeshLod; MAX_PRIMITIVE_LODS],
	count: u8,
}

impl Default for MeshLods {
	fn default() -> Self {
		Self::single(0)
	}
}

impl MeshLods {
	/// Returns the table of a primitive that only has full detail.
	pub(crate) fn single(meshlet_count: u32) -> Self {
		Self::from_resource(&[PrimitiveLod {
			meshlet_count,
			error: 0.0,
		}])
	}

	/// Lays out a baked primitive's levels as the consecutive meshlet ranges they are stored in.
	pub(crate) fn from_resource(lods: &[PrimitiveLod]) -> Self {
		assert!(
			(1..=MAX_PRIMITIVE_LODS).contains(&lods.len()),
			"Mesh primitive has an unsupported number of detail levels. The most likely cause is a mesh resource that was not validated when it was solved."
		);
		let mut levels = [MeshLod::default(); MAX_PRIMITIVE_LODS];
		let mut first_meshlet = 0;
		for (level, lod) in levels.iter_mut().zip(lods) {
			*level = MeshLod {
				first_meshlet,
				meshlet_count: lod.meshlet_count,
				error: lod.error,
			};
			first_meshlet += lod.meshlet_count;
		}
		Self {
			levels,
			count: lods.len() as u8,
		}
	}

	pub(crate) fn levels(&self) -> &[MeshLod] {
		&self.levels[..self.count as usize]
	}

	/// Returns the coarsest level whose error is at most `allowed_error`, or full detail when none is.
	///
	/// Errors never fall as levels get coarser, so the first match from the coarse end is the answer. A NaN allowance
	/// matches nothing and gives full detail.
	fn select(&self, allowed_error: f32) -> MeshLod {
		let fitting = self.levels().iter().rev().find(|level| level.error <= allowed_error);
		*fitting.unwrap_or(&self.levels[0])
	}
}

/// The `LodSettings` struct holds the runtime controls for detail level selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LodSettings {
	/// Multiplies the on-screen error a level may show. Above one draws coarser levels sooner; zero allows no error.
	pub(crate) bias: f32,
	/// Draws this level for every instance instead of selecting by distance. Primitives with fewer levels draw their
	/// coarsest one.
	pub(crate) force: Option<u32>,
}

impl Default for LodSettings {
	fn default() -> Self {
		Self { bias: 1.0, force: None }
	}
}

impl LodSettings {
	/// Applies one runtime parameter, returning the updated settings and the effective value, or leaves them unchanged.
	pub(crate) fn with_parameter(
		self,
		parameter: &str,
		value: &ConfigurationValue,
	) -> Result<(Self, ConfigurationValue), String> {
		match parameter {
			"bias" => {
				let bias = value.to_f64().filter(|bias| *bias >= 0.0 && *bias <= f32::MAX as f64).ok_or(
					"Detail level bias was not set. The most likely cause is that the value is not a finite nonnegative number.",
				)? as f32;
				Ok((Self { bias, ..self }, ConfigurationValue::Float(f64::from(bias))))
			}
			"force" if value.as_text() == Some("off") => Ok((Self { force: None, ..self }, value.clone())),
			"force" => {
				let level = value.to_u32().ok_or(
					"Detail level force was not set. The most likely cause is that the value is neither a whole level number nor `off`.",
				)?;
				let settings = Self {
					force: Some(level),
					..self
				};
				Ok((settings, ConfigurationValue::Integer(i64::from(level))))
			}
			_ => Err(
				"Detail level parameter was not set. The most likely cause is that the parameter name is unsupported."
					.to_string(),
			),
		}
	}
}

/// The `LodView` struct holds what detail level selection needs for one frame: the camera terms, worked out once, and
/// the level to force when one is set.
///
/// Create one with [`Self::new`] from the frame's main sink, then call [`Self::select`] for each instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LodView {
	/// World-space camera position.
	eye: Vec3f,
	/// World-space error allowed per unit of distance from the camera. Zero for an orthographic view.
	error_per_distance: f32,
	/// World-space error allowed at any distance. Zero for a perspective view.
	constant_error: f32,
	force: Option<u32>,
}

impl LodView {
	/// Works out how much world-space error stays within the allowed pixels for `sink`'s camera.
	pub(crate) fn new(sink: &Sink, settings: LodSettings) -> Self {
		let view = sink.view();
		let (projection, extent) = (view.projection(), sink.extent());
		// Pixels that a unit length covers at unit depth, or at any depth for an orthographic view. The larger axis
		// is the stricter one.
		let pixels_per_unit =
			(projection[0].abs() * extent.width() as f32).max(projection[5].abs() * extent.height() as f32) * 0.5;
		let error = LOD_PIXEL_ERROR * settings.bias / pixels_per_unit;
		let perspective = view.y_fov() > math::Degrees::new(0.0);
		Self {
			// The translation column of the inverse view is the camera's world position.
			eye: Vec3f::from(math::inverse(view.view()).get_column(3)),
			error_per_distance: if perspective { error } else { 0.0 },
			constant_error: if perspective { 0.0 } else { error },
			force: settings.force,
		}
	}

	/// Returns the level an instance draws, as a range relative to its primitive's first meshlet, given its `model`
	/// matrix and its object-space bounding `sphere` as xyz center and w radius.
	pub(crate) fn select(&self, lods: &MeshLods, model: &Mat4x3f, sphere: [f32; 4]) -> MeshLod {
		match self.force {
			Some(level) => lods.levels()[(level as usize).min(lods.levels().len() - 1)],
			None => lods.select(self.allowed_error(model, sphere)),
		}
	}

	/// Returns the largest model-space error that stays within the allowed pixels for an instance.
	///
	/// The distance runs from the camera to the nearest point of the instance's bounding sphere, so the level suits
	/// the part of the instance that looks largest. It is a straight-line distance, not a view depth: turning the
	/// camera in place never switches a level. An instance that contains the camera, or whose model matrix has no
	/// usable scale, allows no error.
	fn allowed_error(&self, model: &Mat4x3f, sphere: [f32; 4]) -> f32 {
		let [x_axis, y_axis, z_axis, translation] = <[[f32; 3]; 4]>::from(*model).map(|[x, y, z]| Vec3f::new(x, y, z));
		// The largest axis scale turns model-space lengths into world-space ones without underestimating them.
		let scale = dot(x_axis, x_axis).max(dot(y_axis, y_axis)).max(dot(z_axis, z_axis)).sqrt();
		let center = x_axis * sphere[0] + y_axis * sphere[1] + z_axis * sphere[2] + translation;
		let distance = dist(center, self.eye) - sphere[3] * scale;
		// Written so that a NaN in any term also allows no error.
		if !(distance > 0.0 && scale > 0.0 && scale.is_finite()) {
			return 0.0;
		}
		(self.error_per_distance * distance + self.constant_error) / scale
	}
}

#[cfg(test)]
mod tests {
	use math::{Degrees, Point, UnitVector};
	use resource_management::resources::mesh::PrimitiveLod;
	use utils::Extent;

	use super::{LodSettings, LodView, MeshLods};
	use crate::configuration::ConfigurationValue;
	use crate::rendering::{Sink, view::View};

	const SPHERE: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

	/// Three levels of 8, 4 and 2 meshlets with errors 0, 0.1 and 1.
	fn three_levels() -> MeshLods {
		MeshLods::from_resource(
			&[(8, 0.0), (4, 0.1), (2, 1.0)].map(|(meshlet_count, error)| PrimitiveLod { meshlet_count, error }),
		)
	}

	/// A uniformly scaled instance at `z` along the camera's view direction.
	fn instance(x: f32, z: f32, scale: f32) -> ghi::pod::Mat4x3f {
		[[scale, 0.0, 0.0], [0.0, scale, 0.0], [0.0, 0.0, scale], [x, 0.0, z]].into()
	}

	/// Selection for a camera at the origin looking down +Z, 90 degrees tall, into a 1000 pixel square: one unit at
	/// unit depth covers 500 pixels.
	fn perspective(settings: LodSettings) -> LodView {
		let view = View::new_perspective(
			Degrees::new(90.0),
			1.0,
			0.1,
			100.0,
			Point::new(0.0, 0.0, 0.0),
			UnitVector::z_axis(),
		);
		LodView::new(&Sink::new(view, Extent::square(1000), 0), settings)
	}

	#[test]
	fn instances_draw_the_coarsest_level_whose_error_stays_within_a_pixel() {
		let (lods, view) = (three_levels(), perspective(LodSettings::default()));
		let meshlets = |model| view.select(&lods, &model, SPHERE).meshlet_count;

		// 51 units away with a unit radius leaves 50 units to the nearest point, where a pixel spans 0.1 units.
		assert_eq!(meshlets(instance(0.0, 50.0, 1.0)), 8);
		assert_eq!(meshlets(instance(0.0, 51.5, 1.0)), 4);
		assert_eq!(meshlets(instance(0.0, 600.0, 1.0)), 2);
		// The same distance to the side draws the same level, so turning the camera never switches levels.
		assert_eq!(meshlets(instance(51.5, 0.0, 1.0)), 4);
		// Doubling the instance doubles its error on screen.
		assert_eq!(meshlets(instance(0.0, 53.0, 2.0)), 8);
		// An instance around the camera, a collapsed one, and a broken matrix all stay at full detail.
		assert_eq!(meshlets(instance(0.0, 0.5, 1.0)), 8);
		assert_eq!(meshlets(instance(0.0, 600.0, 0.0)), 8);
		assert_eq!(meshlets(instance(f32::NAN, 600.0, 1.0)), 8);
	}

	#[test]
	fn the_selected_range_follows_the_finer_levels() {
		let level = perspective(LodSettings::default()).select(&three_levels(), &instance(0.0, 51.5, 1.0), SPHERE);

		assert_eq!((level.first_meshlet, level.meshlet_count), (8, 4));
	}

	#[test]
	fn bias_scales_the_allowed_error_and_force_overrides_distance() {
		let (lods, model) = (three_levels(), instance(0.0, 51.5, 1.0));
		let meshlets = |bias, force| {
			perspective(LodSettings { bias, force })
				.select(&lods, &model, SPHERE)
				.meshlet_count
		};

		assert_eq!(meshlets(1.0, None), 4);
		assert_eq!(meshlets(0.5, None), 8);
		assert_eq!(meshlets(10.0, None), 2);
		assert_eq!(meshlets(0.0, None), 8);

		assert_eq!(meshlets(1.0, Some(0)), 8);
		assert_eq!(meshlets(1.0, Some(2)), 2);
		// A forced level past the coarsest one draws the coarsest one.
		assert_eq!(meshlets(1.0, Some(200)), 2);
	}

	#[test]
	fn orthographic_views_draw_the_same_level_at_every_distance() {
		// A 10 unit wide view into 1000 pixels: a pixel spans 0.01 units at any depth, less than the middle level's
		// error, until a bias of 20 allows it.
		let view = View::new_orthographic(
			-5.0,
			5.0,
			-5.0,
			5.0,
			0.1,
			100.0,
			Point::new(0.0, 0.0, 0.0),
			UnitVector::z_axis(),
		);
		let sink = Sink::new(view, Extent::square(1000), 0);
		let lods = three_levels();
		let meshlets = |bias, z| {
			LodView::new(&sink, LodSettings { bias, force: None })
				.select(&lods, &instance(0.0, z, 1.0), SPHERE)
				.meshlet_count
		};

		assert_eq!((meshlets(1.0, 5.0), meshlets(1.0, 80.0)), (8, 8));
		assert_eq!((meshlets(20.0, 5.0), meshlets(20.0, 80.0)), (4, 4));
	}

	#[test]
	fn runtime_parameters_set_the_bias_and_the_forced_level() {
		let settings = LodSettings::default();
		let text = |text: &str| ConfigurationValue::Text(text.to_string());

		let (biased, effective) = settings.with_parameter("bias", &text("2.5")).unwrap();
		assert_eq!((biased.bias, effective), (2.5, ConfigurationValue::Float(2.5)));
		assert!(settings.with_parameter("bias", &ConfigurationValue::Float(-1.0)).is_err());

		let (forced, effective) = settings.with_parameter("force", &ConfigurationValue::Integer(2)).unwrap();
		assert_eq!((forced.force, effective), (Some(2), ConfigurationValue::Integer(2)));
		let (released, effective) = forced.with_parameter("force", &text("off")).unwrap();
		assert_eq!((released.force, effective), (None, text("off")));
		assert!(settings.with_parameter("force", &text("coarse")).is_err());

		assert!(settings.with_parameter("distance", &ConfigurationValue::Float(1.0)).is_err());
	}
}

use ghi::pod::Mat4x3f;
use maths_rs::{Vec3f, dist, dot};
use resource_management::resources::mesh::{MAX_PRIMITIVE_LODS, PrimitiveLod};

use crate::configuration::ConfigurationValue;
use crate::rendering::Sink;
