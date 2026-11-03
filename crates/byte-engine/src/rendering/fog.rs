use math::AABB;
use maths_rs::Vec3f;

/// The `FogLayer` struct shapes how much fog there is at each height, so one [`ExponentialHeightFog`] can stack a
/// wide haze with a thin, dense sheet such as ground mist.
///
/// Pass one to [`ExponentialHeightFog::new`], and optionally a second to [`ExponentialHeightFog::with_second_layer`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogLayer {
	density: f32,
	height_falloff: f32,
	base_height: f32,
	bounds: Option<AABB>,
}

impl FogLayer {
	/// Creates a layer that blocks `density` of the light per meter at its base height.
	///
	/// A density of `0.02` hides about 1 % of a surface per half meter and about 86 % of one 100 m away. Light
	/// rain mist is about `0.01` to `0.06`, and thick fog is `0.1` or more. Next, set how quickly the layer thins
	/// with [`Self::with_height_falloff`].
	pub fn new(density: f32) -> Self {
		debug_assert!(
			density.is_finite() && density >= 0.0,
			"Fog density is invalid. The most likely cause is a negative or non-finite extinction value."
		);
		Self {
			density,
			height_falloff: 0.2,
			base_height: 0.0,
			bounds: None,
		}
	}

	/// Sets how quickly the layer thins with altitude, per meter.
	///
	/// The density halves every `ln(2) / falloff` meters above the base height and doubles as often below it, so the
	/// default `0.2` halves it every 3.5 m. Use `0.0` for fog of the same density at every height, or a large value
	/// such as `3.0` for a sheet that hugs the ground.
	pub fn with_height_falloff(mut self, falloff: f32) -> Self {
		debug_assert!(
			falloff.is_finite() && falloff >= 0.0,
			"Fog height falloff is invalid. The most likely cause is a negative or non-finite falloff."
		);
		self.height_falloff = falloff;
		self
	}

	/// Sets the world-space height, in meters, at which the layer has the density given to [`Self::new`].
	pub fn with_base_height(mut self, height: f32) -> Self {
		debug_assert!(
			height.is_finite(),
			"Fog base height is invalid. The most likely cause is a non-finite height."
		);
		self.base_height = height;
		self
	}

	/// Confines the layer to a world-space box, so it fills one area, such as a courtyard where rain splashes, instead
	/// of the whole scene.
	///
	/// The density drops to zero at the box's faces, so a face seen edge-on shows as a sharp line. Line the faces up
	/// with walls, pillars, or the ground where geometry hides them, and keep the top where the layer has already
	/// thinned out.
	pub fn with_bounds(mut self, bounds: AABB) -> Self {
		self.bounds = Some(bounds);
		self
	}

	/// Returns the fraction of light the layer blocks per meter at its base height. See [`Self::new`].
	pub fn density(&self) -> f32 {
		self.density
	}

	/// Returns how quickly the layer thins with altitude, per meter. See [`Self::with_height_falloff`].
	pub fn height_falloff(&self) -> f32 {
		self.height_falloff
	}

	/// Returns the height at which the layer has its base density. See [`Self::with_base_height`].
	pub fn base_height(&self) -> f32 {
		self.base_height
	}

	/// Returns the box the layer is confined to, if any. See [`Self::with_bounds`].
	pub fn bounds(&self) -> Option<AABB> {
		self.bounds
	}
}

/// The `ExponentialHeightFog` struct describes the mist or haze that fills a scene and thins out with altitude.
///
/// Use it for weather such as rain mist, an overcast day, or valley haze. Install the fog pass with
/// [`crate::application::graphics::setup_exponential_height_fog_render_pass`], then create one fog through
/// [`crate::core::message_bus::MessageScope::factory`]. The newest fog wins, and publishing a fog again on the same
/// handle with [`crate::core::factory::Factory::derive`] changes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExponentialHeightFog {
	layer: FogLayer,
	second_layer: Option<FogLayer>,
	albedo: Vec3f,
	max_opacity: f32,
	anisotropy: f32,
	ambient_illuminance: f32,
	light_samples: u32,
}

impl ExponentialHeightFog {
	/// Creates a fog whose density follows `layer`.
	///
	/// Next, light it with [`Self::with_ambient_illuminance`], and add ground mist or a high haze with
	/// [`Self::with_second_layer`].
	pub fn new(layer: FogLayer) -> Self {
		Self {
			layer,
			second_layer: None,
			albedo: Vec3f::new(1.0, 1.0, 1.0),
			max_opacity: 1.0,
			anisotropy: 0.7,
			ambient_illuminance: 0.0,
			light_samples: 16,
		}
	}

	/// Adds a second layer whose fog adds to the first one's.
	///
	/// Use a dense layer with a steep falloff for mist that pools near the ground, such as spray from rain splashes,
	/// under a thinner haze that fills the whole scene. Both layers share this fog's lighting.
	pub fn with_second_layer(mut self, layer: FogLayer) -> Self {
		self.second_layer = Some(layer);
		self
	}

	/// Sets the fraction of each color the fog scatters rather than absorbs.
	///
	/// Water droplets absorb almost nothing, so the default is white. Use a darker or tinted value for smog or dust.
	pub fn with_albedo(mut self, albedo: Vec3f) -> Self {
		debug_assert!(
			[albedo.x, albedo.y, albedo.z]
				.iter()
				.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
			"Fog albedo is invalid. The most likely cause is a channel outside 0 to 1."
		);
		self.albedo = albedo;
		self
	}

	/// Caps how much the fog can hide, from `0.0` to `1.0`.
	///
	/// Values below `1.0` keep a trace of distant surfaces and the sky visible through the thickest fog.
	pub fn with_max_opacity(mut self, opacity: f32) -> Self {
		debug_assert!(
			(0.0..=1.0).contains(&opacity),
			"Fog maximum opacity is invalid. The most likely cause is a value outside 0 to 1."
		);
		self.max_opacity = opacity;
		self
	}

	/// Sets how strongly the fog scatters sunlight forward, from `0.0` for evenly in every direction to just below
	/// `1.0` for a tight glow around the sun.
	///
	/// Water droplets scatter forward strongly, so the default is `0.7`. The sun is the newest
	/// [`crate::rendering::DirectionalLight`].
	pub fn with_anisotropy(mut self, anisotropy: f32) -> Self {
		debug_assert!(
			(0.0..1.0).contains(&anisotropy),
			"Fog anisotropy is invalid. The most likely cause is a value outside 0 to just below 1."
		);
		self.anisotropy = anisotropy;
		self
	}

	/// Lights the fog evenly from every direction with a sky that delivers `lux` to an upward-facing surface.
	///
	/// Pass the same value as [`crate::rendering::Environment::with_illuminance`] so the fog matches the sky that
	/// lights the scene. A clear daytime sky delivers about 10,000 to 25,000 lux and an overcast one about 1,000 to
	/// 10,000. Without it, only the sun lights the fog.
	pub fn with_ambient_illuminance(mut self, lux: f32) -> Self {
		debug_assert!(
			lux.is_finite() && lux >= 0.0,
			"Fog ambient illuminance is invalid. The most likely cause is a negative or non-finite lux value."
		);
		self.ambient_illuminance = lux;
		self
	}

	/// Sets how many points along each view ray sample each light's shadow, from `1` to `64`.
	///
	/// More samples draw sharper light shafts with finer grain, and cost more GPU time per light that lights the fog.
	/// The default is `16`. The samples sit at the same places every frame, so the grain doesn't flicker. Only the
	/// visibility pipeline samples lights and their shadows in fog; see
	/// [`crate::application::graphics::setup_exponential_height_fog_render_pass`].
	pub fn with_light_samples(mut self, samples: u32) -> Self {
		debug_assert!(
			(1..=64).contains(&samples),
			"Fog light samples are invalid. The most likely cause is a sample count outside 1 to 64."
		);
		self.light_samples = samples;
		self
	}

	/// Returns the layer given to [`Self::new`].
	pub fn layer(&self) -> FogLayer {
		self.layer
	}

	/// Returns the layer added by [`Self::with_second_layer`], if any.
	pub fn second_layer(&self) -> Option<FogLayer> {
		self.second_layer
	}

	/// Returns the fraction of each color the fog scatters. See [`Self::with_albedo`].
	pub fn albedo(&self) -> Vec3f {
		self.albedo
	}

	/// Returns the most the fog can hide. See [`Self::with_max_opacity`].
	pub fn max_opacity(&self) -> f32 {
		self.max_opacity
	}

	/// Returns how strongly the fog scatters sunlight forward. See [`Self::with_anisotropy`].
	pub fn anisotropy(&self) -> f32 {
		self.anisotropy
	}

	/// Returns the sky illuminance that lights the fog, in lux. See [`Self::with_ambient_illuminance`].
	pub fn ambient_illuminance(&self) -> f32 {
		self.ambient_illuminance
	}

	/// Returns how many points along each view ray sample each light's shadow. See [`Self::with_light_samples`].
	pub fn light_samples(&self) -> u32 {
		self.light_samples
	}
}

/// The render-graph name of the light a scene pipeline marches through the fog at half the sink's resolution: the
/// exposed light the fog scatters toward the camera in rgb, and log2(1 + the marched distance) in alpha, zero where
/// nothing was marched. The visibility pipeline writes it and the height fog pass upsamples it.
pub(crate) const FOG_LIGHT_TARGET: &str = "Fog Light";

/// The `FogLayerShaderData` struct packs one [`FogLayer`] relative to the camera, as `FogLayer` in
/// `layouts/fog.besl` declares it, for the height fog pass and the visibility pipeline's fog light march.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FogLayerShaderData {
	/// Density, height falloff, the density exponent at the camera, and `1` when the layer has bounds.
	pub(crate) shape: [f32; 4],
	pub(crate) bounds_min: [f32; 4],
	pub(crate) bounds_max: [f32; 4],
}

impl FogLayerShaderData {
	/// Packs `layer` for a camera at `camera`.
	pub(crate) fn new(layer: FogLayer, camera: Vec3f) -> Self {
		// The density at the camera is density · e^exponent. The shader clamps the exponent once it adds the ray's part.
		let exponent = layer.height_falloff() * (layer.base_height() - camera.y);
		let (bounded, bounds_min, bounds_max) = match layer.bounds() {
			Some(bounds) => (1.0, bounds.min().into_maths() - camera, bounds.max().into_maths() - camera),
			None => (0.0, Vec3f::new(0.0, 0.0, 0.0), Vec3f::new(0.0, 0.0, 0.0)),
		};
		Self {
			shape: [layer.density(), layer.height_falloff(), exponent, bounded],
			bounds_min: [bounds_min.x, bounds_min.y, bounds_min.z, 0.0],
			bounds_max: [bounds_max.x, bounds_max.y, bounds_max.z, 0.0],
		}
	}
}

/// Returns the matrix that takes a pixel's integer coordinate and reverse-Z depth to its offset from the camera, in
/// world axes, before the division by w.
pub(crate) fn pixel_to_camera_offset(view: crate::rendering::View, extent: utils::Extent) -> math::Matrix {
	// Without the view's translation, the inverse lands relative to the camera instead of the world origin.
	let mut rotation = view.view();
	rotation.set_column(3, maths_rs::Vec4f::new(0.0, 0.0, 0.0, 1.0));
	math::inverse(view.projection() * rotation) * super::render_passes::pixel_to_ndc(extent)
}
