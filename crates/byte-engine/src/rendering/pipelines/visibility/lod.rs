//! Detail level selection: which of a primitive's baked levels each instance wants this frame, and which resident one
//! it draws.
//!
//! A mesh keeps only some of its levels resident. [`MeshLods`] carries where each resident level lives from the loaded
//! mesh to each render entity, and [`LodView`] picks the level each instance wants from the main camera. An instance
//! draws that level when it is resident, or the finest resident level coarser than it otherwise. Shadow views draw the
//! same level, which keeps a caster and its lit surface in agreement.

/// Error, in pixels, a level may show on screen at the default bias before a finer one is drawn.
const LOD_PIXEL_ERROR: f32 = 1.0;

/// The `MeshLod` struct names the record an instance reads one detail level of its primitive from.
///
/// A prepared mesh counts [`Self::level`] from the start of its own level records;
/// [`super::geometry::GeometryBuffers::append_mesh`] turns it into a buffer element once it places the mesh. Levels that
/// are not resident keep only their meshlet count and error.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MeshLod {
	/// The level's [`super::layout::ShaderMeshLevel`] record, which names its meshlets and vertices.
	pub(crate) level: u32,
	pub(crate) meshlet_count: u32,
	/// The level's first vertex, which a posed skinned instance copies its bind-pose UVs from.
	pub(crate) vertex_base: u32,
	/// Estimated largest distance, in model-space units, between this level's surface and full detail.
	pub(crate) error: f32,
	/// Whether the level's geometry is in the buffers, so an instance can draw it.
	pub(crate) resident: bool,
}

/// The `MeshLods` struct keeps a primitive's detail levels, finest first, in storage the frame loop can copy freely.
///
/// A fixed table avoids a heap allocation per render entity. Build one with [`Self::new`] for a baked primitive or
/// [`Self::single`] for geometry without coarser levels, then let [`LodView::select`] choose from it.
///
/// A primitive isn't always drawn. One that other primitives of its mesh replace past an error, set with
/// [`Self::replaced_at`], is hidden once that error is allowed. One whose finest level has an error, such as a
/// billboard, [takes over](Self::takes_over) from others and is hidden until that error is allowed. The two errors are
/// equal, so exactly one side shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MeshLods {
	levels: [MeshLod; MAX_PRIMITIVE_LODS],
	count: u8,
	/// The error from which other primitives of the mesh take this one's place, or infinity when none does.
	replaced_at: f32,
}

impl Default for MeshLods {
	fn default() -> Self {
		Self::single(0)
	}
}

impl MeshLods {
	/// Returns the table of a primitive that only has full detail.
	pub(crate) fn single(meshlet_count: u32) -> Self {
		Self::new(&[MeshLod {
			meshlet_count,
			resident: true,
			..MeshLod::default()
		}])
	}

	/// Collects a primitive's levels, finest first. Each level names its own record, so levels need not follow one
	/// another.
	pub(crate) fn new(lods: &[MeshLod]) -> Self {
		assert!(
			(1..=MAX_PRIMITIVE_LODS).contains(&lods.len()),
			"Mesh primitive has an unsupported number of detail levels. The most likely cause is a mesh resource that was not validated when it was solved."
		);
		let mut levels = [MeshLod::default(); MAX_PRIMITIVE_LODS];
		levels[..lods.len()].copy_from_slice(lods);
		Self {
			levels,
			count: lods.len() as u8,
			replaced_at: f32::INFINITY,
		}
	}

	/// Returns the table of a primitive that other primitives of its mesh replace once `error` is allowed, or never
	/// for `None`.
	pub(crate) fn replaced_at(mut self, error: Option<f32>) -> Self {
		self.replaced_at = error.unwrap_or(f32::INFINITY);
		self
	}

	/// Returns whether the primitive takes the place of other primitives of its mesh: its finest level has an error,
	/// so it's drawn only once that error is allowed.
	pub(crate) fn takes_over(&self) -> bool {
		self.levels[0].error > 0.0
	}

	pub(crate) fn levels(&self) -> &[MeshLod] {
		&self.levels[..self.count as usize]
	}

	pub(crate) fn levels_mut(&mut self) -> &mut [MeshLod] {
		&mut self.levels[..self.count as usize]
	}

	/// Returns the level an instance that wants `wanted` draws: that level when it is resident, or else the finest
	/// resident level coarser than it, or else the coarsest resident level finer than it.
	///
	/// A resource mesh always holds its coarsest levels, so its instances always draw. A merged mesh holds only the
	/// level it was last drawn at, and none until that level is built: `None` then, and the instance waits.
	pub(crate) fn drawn(&self, wanted: usize) -> Option<MeshLod> {
		let levels = self.levels();
		let wanted = wanted.min(levels.len() - 1);
		levels[wanted..]
			.iter()
			.find(|level| level.resident)
			.or_else(|| levels[..wanted].iter().rev().find(|level| level.resident))
			.copied()
	}

	/// Overlays every resident level of `other`, a block holding other levels of the same primitive, onto this table.
	pub(crate) fn overlay(&mut self, other: &MeshLods) {
		for (level, other) in self.levels_mut().iter_mut().zip(other.levels()) {
			if other.resident {
				*level = *other;
			}
		}
	}

	/// Returns the coarsest level whose error is at most `allowed_error`, or full detail when none is. Returns `None`
	/// when the primitive isn't drawn at that allowance: others have replaced it, or it hasn't taken over yet.
	///
	/// Errors never fall as levels get coarser, so the first match from the coarse end is the answer. A NaN allowance
	/// matches nothing: it gives full detail, and hides a primitive that takes over.
	fn select(&self, allowed_error: f32) -> Option<usize> {
		if self.replaced_at <= allowed_error {
			return None;
		}
		match self.levels().iter().rposition(|level| level.error <= allowed_error) {
			None if self.takes_over() => None,
			level => Some(level.unwrap_or(0)),
		}
	}

	/// Returns the allowed errors between which [`Self::select`] gives what it gives for `allowed_error`: at least the
	/// first, less than the second.
	fn held_errors(&self, allowed_error: f32) -> [f32; 2] {
		if self.replaced_at <= allowed_error {
			return [self.replaced_at, f32::INFINITY];
		}
		let levels = self.levels();
		let next = |level: usize| {
			levels
				.get(level + 1)
				.map_or(f32::INFINITY, |level| level.error)
				.min(self.replaced_at)
		};
		match levels.iter().rposition(|level| level.error <= allowed_error) {
			// Not taken over yet.
			None if self.takes_over() => [f32::NEG_INFINITY, levels[0].error],
			None => [f32::NEG_INFINITY, next(0)],
			Some(0) if !self.takes_over() => [f32::NEG_INFINITY, next(0)],
			Some(level) => [levels[level].error, next(level)],
		}
	}
}

/// The `Held` struct is the level an instance wants and how far the camera can move before it wants another.
///
/// [`LodView::held`] makes one when it picks an instance's level. While [`Self::holds`] for a later view that
/// [`LodView::scales_alike`], the pick stands, so following a camera that moves or zooms costs one distance for each
/// instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Held {
	/// What [`LodView::select`] returned.
	pub(crate) wanted: Option<usize>,
	/// The world-space center of the instance's bounding sphere.
	center: [f32; 3],
	/// The world-space radius of the bounding sphere, which the allowed error's distance starts from.
	reach: f32,
	/// The world-space errors, allowed per unit of distance from the sphere's surface times that distance, between
	/// which `wanted` holds: more than the first, less than the second. Storing errors instead of distances keeps the
	/// pick valid when the field of view or the sink's size changes the error allowed per unit of distance.
	range: [f32; 2],
}

impl Held {
	/// Returns a pick of `wanted` that never holds, so its instance is picked again every time it's followed.
	pub(crate) const fn never(wanted: Option<usize>) -> Self {
		Self {
			wanted,
			center: [0.0; 3],
			reach: 0.0,
			range: [f32::INFINITY, 0.0],
		}
	}

	/// Returns a pick of `wanted` that holds from anywhere.
	pub(crate) const fn always(wanted: Option<usize>) -> Self {
		Self {
			wanted,
			center: [0.0; 3],
			reach: 0.0,
			range: [f32::NEG_INFINITY, f32::INFINITY],
		}
	}

	/// Returns whether the instance still wants the same level from `view`, which
	/// [`LodView::scales_alike`] the view that made this pick.
	pub(crate) fn holds(&self, view: &LodView) -> bool {
		let [x, y, z] = self.center;
		let offset = Vec3f::new(x, y, z) - view.eye;
		let squared = dot(offset, offset);
		// The distances at which this view's allowed error crosses each end of the range. Infinite ends stay infinite.
		let [near, far] = self.range.map(|error| error / view.error_per_distance + self.reach);
		let near = near.max(0.0);
		squared >= near * near && squared < far * far
	}
}

/// The `LodSettings` struct holds the runtime controls for detail level selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LodSettings {
	/// Multiplies the on-screen error a level may show. Above one draws coarser levels sooner; zero allows no error.
	pub(crate) bias: f32,
	/// Draws this level for every instance instead of selecting by distance. Primitives with fewer levels draw their
	/// coarsest one, and primitives that take over from others aren't drawn.
	pub(crate) force: Option<u32>,
	/// The radius, in pixels, under which an instance's bounding sphere makes it a small instance. Zero makes none
	/// small.
	pub(crate) small_radius: f32,
	/// Multiplies the error a small instance may show, on top of `bias`. Never below one, so an instance never gets
	/// finer as it shrinks on screen.
	pub(crate) small_bias: f32,
}

impl Default for LodSettings {
	/// One pixel of error, and two for instances under 6 pixels in radius. Scenes scatter grass and shrubs by the
	/// tens of thousands, and each covers a handful of pixels from most of the scene: on the slate quarry they were
	/// most of the camera's meshlets, and twice the error for them cut the early visibility pass by 22 % to 42 %
	/// while changing under 1 % of the pixels.
	fn default() -> Self {
		Self {
			bias: 1.0,
			force: None,
			small_radius: 6.0,
			small_bias: 2.0,
		}
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
			"small-radius" => {
				let small_radius = value.to_f64().filter(|radius| *radius >= 0.0 && *radius <= f32::MAX as f64).ok_or(
					"Small instance radius was not set. The most likely cause is that the value is not a finite nonnegative number of pixels.",
				)? as f32;
				Ok((
					Self { small_radius, ..self },
					ConfigurationValue::Float(f64::from(small_radius)),
				))
			}
			"small-bias" => {
				let small_bias = value.to_f64().filter(|bias| *bias >= 1.0 && *bias <= f32::MAX as f64).ok_or(
					"Small instance detail bias was not set. The most likely cause is that the value is not a finite number of at least 1.",
				)? as f32;
				Ok((Self { small_bias, ..self }, ConfigurationValue::Float(f64::from(small_bias))))
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
	/// The world-space error, per unit of an instance's world-space radius, past which the instance is small: its
	/// radius then covers fewer pixels than the small radius. Infinite when no instance is small.
	small_error_per_reach: f32,
	/// Multiplies the error a small instance is allowed. At least one.
	small_bias: f32,
	force: Option<u32>,
	/// The camera frustum's planes in world space, each as its xyz normal, which points inside, and w distance.
	frustum: [[f32; 4]; 6],
	/// The direction each sun's light travels, which is the way shadows fall. Only the first `sun_count` are set.
	sun_travel: [Vec3f; MAX_FOLLOWED_SUNS],
	sun_count: u8,
}

/// The most suns whose shadows [`LodView::sees`] follows. Further suns' shadows of instances outside the camera's
/// view may go missing.
const MAX_FOLLOWED_SUNS: usize = 4;

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
			// A pixel spans `error / bias` here, so the radius covers fewer than `small_radius` pixels once the
			// allowed error passes `bias / small_radius` of it. A bias of zero allows no error, and nothing is small.
			small_error_per_reach: if settings.small_radius > 0.0 && settings.bias > 0.0 {
				LOD_PIXEL_ERROR * settings.bias / settings.small_radius
			} else {
				f32::INFINITY
			},
			small_bias: settings.small_bias.max(1.0),
			force: settings.force,
			frustum: view.get_frustum_planes().map(|plane| {
				let normal = plane.normal().into_vector().to_array();
				[normal[0], normal[1], normal[2], plane.distance()]
			}),
			sun_travel: [Vec3f::new(0.0, 0.0, 0.0); MAX_FOLLOWED_SUNS],
			sun_count: 0,
		}
	}

	/// Follows the shadows of suns whose light travels along `directions`, so [`Self::sees`] counts an instance whose
	/// shadow reaches the camera's view.
	pub(crate) fn with_suns(mut self, directions: impl IntoIterator<Item = Vec3f>) -> Self {
		for direction in directions.into_iter().take(MAX_FOLLOWED_SUNS) {
			self.sun_travel[self.sun_count as usize] = direction;
			self.sun_count += 1;
		}
		self
	}

	/// Returns whether the camera can see an instance or a shadow a followed sun casts from it, given its `model`
	/// matrix and its object-space bounding `sphere` as xyz center and w radius.
	///
	/// The test never misses what is in view: it keeps an instance unless its sphere, and the sphere carried any
	/// distance along a sun's light, lies wholly behind one plane of the camera's frustum. A merged mesh builds its
	/// geometry only while this holds.
	pub(crate) fn sees(&self, model: &Mat4x3f, sphere: [f32; 4]) -> bool {
		let [x_axis, y_axis, z_axis, translation] = <[[f32; 3]; 4]>::from(*model).map(|[x, y, z]| Vec3f::new(x, y, z));
		let scale = dot(x_axis, x_axis).max(dot(y_axis, y_axis)).max(dot(z_axis, z_axis)).sqrt();
		let center = x_axis * sphere[0] + y_axis * sphere[1] + z_axis * sphere[2] + translation;
		let radius = sphere[3] * scale;
		// A sphere that isn't finite can't be ruled out.
		if !(radius.is_finite() && center.x.is_finite() && center.y.is_finite() && center.z.is_finite()) {
			return true;
		}
		let behind = |[x, y, z, distance]: [f32; 4], direction: Option<Vec3f>| {
			let normal = Vec3f::new(x, y, z);
			// Carried along `direction`, the sphere stays behind the plane only if the direction leads no closer.
			dot(normal, center) + distance < -radius && direction.is_none_or(|direction| dot(normal, direction) <= 0.0)
		};
		let outside = |direction: Option<Vec3f>| self.frustum.iter().any(|&plane| behind(plane, direction));
		!outside(None)
			|| self.sun_travel[..self.sun_count as usize]
				.iter()
				.any(|&sun| !outside(Some(sun)))
	}

	/// Returns the level an instance wants, given its `model` matrix and its object-space bounding `sphere` as xyz
	/// center and w radius, or `None` when the instance isn't drawn from here: other primitives of its mesh have
	/// replaced it, or it hasn't taken over from them yet. Pass the level to [`MeshLods::drawn`] for the level the
	/// instance can draw now.
	pub(crate) fn select(&self, lods: &MeshLods, model: &Mat4x3f, sphere: [f32; 4]) -> Option<usize> {
		match self.force {
			Some(level) => (!lods.takes_over()).then_some((level as usize).min(lods.levels().len() - 1)),
			None => lods.select(self.allowed_error(model, sphere)),
		}
	}

	/// Returns this view from a camera at `eye`, for tests that move the camera.
	#[cfg(test)]
	pub(crate) fn moved_to(self, eye: Vec3f) -> Self {
		Self { eye, ..self }
	}

	/// Returns a perspective view, for tests, from a camera at the origin whose pixel covers `error_per_distance`
	/// units at each unit of distance.
	#[cfg(test)]
	pub(crate) fn for_test(error_per_distance: f32) -> Self {
		Self {
			eye: Vec3f::new(0.0, 0.0, 0.0),
			error_per_distance,
			constant_error: 0.0,
			small_error_per_reach: f32::INFINITY,
			small_bias: 1.0,
			force: None,
			frustum: [[0.0; 4]; 6],
			sun_travel: [Vec3f::new(0.0, 0.0, 0.0); MAX_FOLLOWED_SUNS],
			sun_count: 0,
		}
	}

	/// Returns whether a [`Held`] pick made for `other` can be tested against this view. Perspective views that
	/// differ in where the camera is, where it faces, its field of view, or the sink's size scale alike: a pick
	/// stores errors, which [`Held::holds`] turns into distances for the view it tests. A forced level, a switch
	/// between perspective and orthographic, or another orthographic error picks every level anew.
	pub(crate) fn scales_alike(&self, other: &Self) -> bool {
		(self.error_per_distance > 0.0) == (other.error_per_distance > 0.0)
			&& self.constant_error == other.constant_error
			// A pick's range is worked out with these, so it means nothing under others.
			&& self.small_error_per_reach == other.small_error_per_reach
			&& self.small_bias == other.small_bias
			&& self.force == other.force
	}

	/// Returns the level an instance wants, as [`Self::select`] does, with the camera distances over which it keeps
	/// wanting it.
	///
	/// The allowed error grows with the distance from the camera to the instance's bounding sphere, so each pick holds
	/// between two distances. A forced level and an orthographic view pick the same level from anywhere.
	pub(crate) fn held(&self, lods: &MeshLods, model: &Mat4x3f, sphere: [f32; 4]) -> Held {
		let wanted = self.select(lods, model, sphere);
		let [x_axis, y_axis, z_axis, translation] = <[[f32; 3]; 4]>::from(*model).map(|[x, y, z]| Vec3f::new(x, y, z));
		let scale = dot(x_axis, x_axis).max(dot(y_axis, y_axis)).max(dot(z_axis, z_axis)).sqrt();
		let center = x_axis * sphere[0] + y_axis * sphere[1] + z_axis * sphere[2] + translation;
		let reach = sphere[3] * scale;
		let center = [center.x, center.y, center.z];
		// The pick doesn't follow distance: it's forced, the view is orthographic, or the instance has no usable
		// scale or place and allows no error wherever the camera is.
		let follows_distance = self.force.is_none()
			&& self.error_per_distance > 0.0
			&& scale > 0.0
			&& scale.is_finite()
			&& reach.is_finite()
			&& center.iter().all(|component| component.is_finite());
		if !follows_distance {
			return Held::always(wanted);
		}
		let [low, high] = lods.held_errors(self.allowed_error(model, sphere));
		// `allowed_error` is the error per unit of distance times the distance from the sphere's surface, raised for
		// a small instance and divided by the scale, so the scaled errors bound that product through
		// `error_reaching`. The range stops a little short of each switch: there
		// the inverse and `allowed_error` can round to different sides, and an instance that close is picked again,
		// so following always agrees with picking.
		const MARGIN: f32 = 1e-4;
		let near = if low * scale <= 0.0 {
			f32::NEG_INFINITY
		} else {
			self.error_reaching(low * scale, reach) * (1.0 + MARGIN)
		};
		let far = if high == f32::INFINITY {
			f32::INFINITY
		} else {
			self.error_reaching(high * scale, reach) * (1.0 - MARGIN)
		};
		Held {
			wanted,
			center,
			reach,
			range: [near, far],
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
		self.allowed_world_error(self.error_per_distance * distance + self.constant_error, sphere[3] * scale) / scale
	}

	/// Returns the world-space error allowed for an instance of world-space radius `reach` where a level may show
	/// `error` at the default allowance: `error` itself, or the small instance bias times it once the instance is
	/// small. It never falls as `error` grows, and jumps where the instance becomes small.
	fn allowed_world_error(&self, error: f32, reach: f32) -> f32 {
		// Written so that a NaN, as a zero radius gives when nothing is small, counts as not small.
		if error > reach * self.small_error_per_reach {
			error * self.small_bias
		} else {
			error
		}
	}

	/// Returns the smallest default allowance at which [`Self::allowed_world_error`] reaches `allowed` for an instance
	/// of world-space radius `reach`. [`Self::held`] turns the errors a pick holds between into the allowances, and
	/// so the distances, it holds between.
	fn error_reaching(&self, allowed: f32, reach: f32) -> f32 {
		let small_from = reach * self.small_error_per_reach;
		if !(allowed > small_from) {
			// Reached before the instance is small.
			allowed
		} else if allowed <= small_from * self.small_bias {
			// Reached by the jump where the instance becomes small.
			small_from
		} else {
			allowed / self.small_bias
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{Degrees, Point, UnitVector};
	use maths_rs::Vec3f;
	use utils::Extent;

	use super::{Held, LodSettings, LodView, MeshLod, MeshLods};
	use crate::configuration::ConfigurationValue;
	use crate::rendering::{Sink, view::View};

	const SPHERE: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

	/// Three levels of 8, 4 and 2 meshlets in records 0, 1 and 2, with errors 0, 0.1 and 1.
	fn three_levels() -> MeshLods {
		MeshLods::new(
			&[(0, 8, 0.0), (1, 4, 0.1), (2, 2, 1.0)].map(|(level, meshlet_count, error)| MeshLod {
				level,
				meshlet_count,
				error,
				resident: true,
				..MeshLod::default()
			}),
		)
	}

	/// Settings under which no instance is small, so every instance is allowed the same error on screen.
	const NO_SMALL_INSTANCES: LodSettings = LodSettings {
		bias: 1.0,
		force: None,
		small_radius: 0.0,
		small_bias: 1.0,
	};

	/// A uniformly scaled instance at `z` along the camera's view direction.
	fn instance(x: f32, z: f32, scale: f32) -> ghi::pod::Mat4x3f {
		[[scale, 0.0, 0.0], [0.0, scale, 0.0], [0.0, 0.0, scale], [x, 0.0, z]].into()
	}

	#[test]
	fn an_instance_is_seen_in_the_frustum_or_when_a_sun_carries_its_shadow_there() {
		use maths_rs::Vec3f;

		let view = perspective(LodSettings::default());
		// Ahead of the camera, and far to its side but still touching the frustum's edge.
		assert!(view.sees(&instance(0.0, 10.0, 1.0), SPHERE));
		assert!(view.sees(&instance(10.5, 10.0, 1.0), SPHERE));
		// Behind the camera, and beside it past the frustum's edge.
		assert!(!view.sees(&instance(0.0, -10.0, 1.0), SPHERE));
		assert!(!view.sees(&instance(20.0, 10.0, 1.0), SPHERE));
		// A larger instance reaches into view from the same place.
		assert!(view.sees(&instance(20.0, 10.0, 10.0), SPHERE));

		// A sun that shines forward carries the shadow of what is behind the camera into view; one that shines
		// backward carries it away.
		let forward = view.with_suns([Vec3f::new(0.0, 0.0, 1.0)]);
		assert!(forward.sees(&instance(0.0, -10.0, 1.0), SPHERE));
		let backward = view.with_suns([Vec3f::new(0.0, 0.0, -1.0)]);
		assert!(!backward.sees(&instance(0.0, -10.0, 1.0), SPHERE));
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
		let meshlets = |model| lods.drawn(view.select(&lods, &model, SPHERE).unwrap()).unwrap().meshlet_count;

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
	fn a_primitive_gives_way_to_the_one_that_replaces_it_at_one_distance() {
		let view = perspective(NO_SMALL_INSTANCES);
		// A plant that a billboard replaces once an error of half a unit is allowed.
		let plant = three_levels().replaced_at(Some(0.5));
		let billboard = MeshLods::new(&[MeshLod {
			error: 0.5,
			meshlet_count: 1,
			resident: true,
			..MeshLod::default()
		}]);
		let shown = |z| {
			let model = instance(0.0, z, 1.0);
			[&plant, &billboard].map(|lods| view.select(lods, &model, SPHERE))
		};

		// A pixel spans half a unit 250 units past the sphere. Up to there the plant draws, on its own levels.
		assert_eq!(shown(51.5), [Some(1), None]);
		assert_eq!(shown(250.0), [Some(1), None]);
		// From there on the billboard draws alone, so the plant never reaches its own coarsest level.
		assert_eq!(shown(252.0), [None, Some(0)]);
		assert_eq!(shown(2000.0), [None, Some(0)]);
		// Around the camera nothing is allowed, which is the plant's full detail.
		assert_eq!(shown(0.5), [Some(0), None]);

		// A forced level draws the plant and never the billboard.
		let forced = perspective(LodSettings {
			force: Some(7),
			..LodSettings::default()
		});
		let model = instance(0.0, 2000.0, 1.0);
		assert_eq!(
			[&plant, &billboard].map(|lods| forced.select(lods, &model, SPHERE)),
			[Some(2), None]
		);
	}

	#[test]
	fn small_instances_draw_coarser_levels_sooner() {
		let lods = three_levels();
		let meshlets = |settings, z| {
			let view = perspective(settings);
			lods.drawn(view.select(&lods, &instance(0.0, z, 1.0), SPHERE).unwrap())
				.unwrap()
				.meshlet_count
		};
		// A unit sphere 250 units away covers 2 pixels of radius. Its pixel spans half a unit there, short of the
		// coarsest level's error of one unit, which twice the error allows.
		assert_eq!(meshlets(NO_SMALL_INSTANCES, 251.0), 4);
		assert_eq!(meshlets(LodSettings::default(), 251.0), 2);
		// At 70 units it covers 7 pixels of radius, which is not small.
		assert_eq!(meshlets(NO_SMALL_INSTANCES, 71.0), 4);
		assert_eq!(meshlets(LodSettings::default(), 71.0), 4);
		// A wider small radius takes it in, but its allowance of 0.28 units still falls short of the coarsest level.
		let wide = LodSettings {
			small_radius: 10.0,
			..LodSettings::default()
		};
		assert_eq!(meshlets(wide, 71.0), 4);
		// A stronger bias for small instances reaches it.
		let strong = LodSettings { small_bias: 8.0, ..wide };
		assert_eq!(meshlets(strong, 71.0), 2);
	}

	#[test]
	fn a_held_pick_stands_exactly_while_it_holds() {
		let view = perspective(LodSettings::default());
		let plant = three_levels().replaced_at(Some(0.5));
		let billboard = MeshLods::new(&[MeshLod {
			error: 0.5,
			meshlet_count: 1,
			resident: true,
			..MeshLod::default()
		}]);
		// Distances on both sides of every switch: the plant's levels at errors 0.1 and 1, and the takeover at 0.5.
		let distances = (1..400).map(|step| step as f32 * 1.5);

		for lods in [three_levels(), plant, billboard] {
			let mut picks = Vec::new();
			for z in distances.clone() {
				let held = view.held(&lods, &instance(0.0, z, 1.0), SPHERE);
				assert_eq!(held.wanted, view.select(&lods, &instance(0.0, z, 1.0), SPHERE));
				if picks.last() != Some(&held.wanted) {
					picks.push(held.wanted);
				}
				// The camera is at the origin, so an instance at another depth is the camera at another distance. A
				// zoomed view allows another error per unit of distance.
				for zoom in [0.25, 1.0, 3.0] {
					let zoomed = LodView {
						error_per_distance: view.error_per_distance * zoom,
						..view
					};
					assert!(zoomed.scales_alike(&view));
					for other in distances.clone() {
						let moved = LodView {
							eye: Vec3f::new(0.0, 0.0, z - other),
							..zoomed
						};
						if held.holds(&moved) {
							assert_eq!(
								zoomed.select(&lods, &instance(0.0, other, 1.0), SPHERE),
								held.wanted,
								"picked at {z}, tested at {other} zoomed {zoom}"
							);
						}
					}
				}
			}
			assert!(picks.len() > 1, "the distances should cross a switch: {picks:?}");
		}

		// A forced level holds from anywhere, and a pick that never holds is always made again.
		let forced = perspective(LodSettings {
			force: Some(1),
			..LodSettings::default()
		});
		let held = forced.held(&three_levels(), &instance(0.0, 5.0, 1.0), SPHERE);
		assert!(held.holds(&LodView {
			eye: Vec3f::new(0.0, 0.0, -9000.0),
			..forced
		}));
		assert!(!Held::never(Some(0)).holds(&view));
		assert!(Held::always(None).holds(&view));
		assert!(view.scales_alike(&LodView {
			eye: Vec3f::new(3.0, 0.0, 0.0),
			..view
		}));
		assert!(!view.scales_alike(&forced));
	}

	#[test]
	fn the_selected_record_follows_the_finer_levels() {
		let lods = three_levels();
		let level = lods
			.drawn(
				perspective(LodSettings::default())
					.select(&lods, &instance(0.0, 51.5, 1.0), SPHERE)
					.unwrap(),
			)
			.unwrap();

		assert_eq!((level.level, level.meshlet_count), (1, 4));
	}

	#[test]
	fn instances_draw_the_finest_resident_level_no_finer_than_they_want() {
		let mut lods = three_levels();
		lods.levels_mut()[0].resident = false;
		lods.levels_mut()[1].resident = false;

		// Wanting a level that isn't resident draws the next coarser resident one.
		assert_eq!(lods.drawn(0).unwrap().meshlet_count, 2);
		assert_eq!(lods.drawn(1).unwrap().meshlet_count, 2);

		let mut refined = three_levels();
		refined.levels_mut()[2].resident = false;
		refined.levels_mut()[1].level = 40;
		lods.overlay(&refined);

		assert_eq!(lods.drawn(0).unwrap().meshlet_count, 8);
		assert_eq!((lods.drawn(1).unwrap().level, lods.drawn(2).unwrap().level), (40, 2));
	}

	#[test]
	fn bias_scales_the_allowed_error_and_force_overrides_distance() {
		let (lods, model) = (three_levels(), instance(0.0, 51.5, 1.0));
		let meshlets = |bias, force| {
			lods.drawn(
				perspective(LodSettings {
					bias,
					force,
					..NO_SMALL_INSTANCES
				})
				.select(&lods, &model, SPHERE)
				.unwrap(),
			)
			.unwrap()
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
			lods.drawn(
				LodView::new(
					&sink,
					LodSettings {
						bias,
						..NO_SMALL_INSTANCES
					},
				)
				.select(&lods, &instance(0.0, z, 1.0), SPHERE)
				.unwrap(),
			)
			.unwrap()
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

		let (small, effective) = settings.with_parameter("small-radius", &text("12")).unwrap();
		assert_eq!((small.small_radius, effective), (12.0, ConfigurationValue::Float(12.0)));
		assert!(
			settings
				.with_parameter("small-radius", &ConfigurationValue::Float(-1.0))
				.is_err()
		);
		let (small, effective) = settings.with_parameter("small-bias", &text("3")).unwrap();
		assert_eq!((small.small_bias, effective), (3.0, ConfigurationValue::Float(3.0)));
		// A small instance never gets finer than a large one.
		assert!(
			settings
				.with_parameter("small-bias", &ConfigurationValue::Float(0.5))
				.is_err()
		);

		assert!(settings.with_parameter("distance", &ConfigurationValue::Float(1.0)).is_err());
	}
}

use ghi::pod::Mat4x3f;
use maths_rs::{Vec3f, dist, dot};
use resource_management::resources::mesh::MAX_PRIMITIVE_LODS;

use crate::configuration::ConfigurationValue;
use crate::rendering::Sink;
