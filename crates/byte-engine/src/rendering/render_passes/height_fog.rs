use bytemuck::Zeroable as _;
use ghi::{context::ContextCreate as _, frame::Frame as _};
use math::inverse;
use maths_rs::Vec3f;
use utils::Extent;

use crate::{
	core::{
		Entity,
		factory::CreateMessage,
		listener::{DefaultListener, Listener},
	},
	gameplay::transform::TransformationUpdate,
	rendering::{
		DirectionalLight, ExponentialHeightFog, Sink,
		fog::{FOG_LIGHT_TARGET, FogLayerShaderData, pixel_to_camera_offset},
		render_pass::{RenderPassBuilder, pixel_steps::PixelStep, simple_tile},
		render_passes::sun::Sun,
	},
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FogShaderData {
	pixel_to_camera_offset: ghi::pod::Mat4f,
	layers: [FogLayerShaderData; 2],
	scattering: [f32; 4],
	sun_direction: [f32; 4],
	ambient_inscattering: [f32; 4],
	sun_inscattering: [f32; 4],
}

/// The `ExponentialHeightFogRenderPass` struct hides the scene behind the newest [`ExponentialHeightFog`], so
/// distant surfaces and the horizon fade into mist lit by the sky and the scene's lights.
///
/// Under the visibility pipeline, the scene's lights and their shadows reach the fog through the half-resolution
/// light the pipeline marches into its `Fog Light` target, which this pass upsamples. Without that target, the newest
/// [`DirectionalLight`] lights the fog unshadowed.
///
/// It is a pixel step: install it with [`crate::application::graphics::setup_exponential_height_fog_render_pass`]
/// after the scene pipeline and particles, and before lens flare, bloom, and tone mapping, so light glows through the
/// fog instead of over it. The pass reads the opaque scene depth, so transparent surfaces and particles take the fog
/// of the surface behind them.
pub struct ExponentialHeightFogRenderPass {
	resources: [simple_tile::Resource; 3],
	parameters: ghi::DynamicBufferHandle<FogShaderData>,
	fogs: DefaultListener<CreateMessage<ExponentialHeightFog>>,
	fog: Option<ExponentialHeightFog>,
	sun: Sun,
}

impl Entity for ExponentialHeightFogRenderPass {}

impl ExponentialHeightFogRenderPass {
	/// The step's own pipeline, which fusions name it by.
	pub const PIPELINE: &'static str = "byte_engine/rendering/height-fog.pipeline";

	/// Creates the fog step, which reads the scene's depth and the marched fog light.
	///
	/// The step applies nothing until a fog arrives through `fogs`. The newest directional light from
	/// `directional_lights` is the sun that lights the fog, and its transform from `transform_listener` sets the sun's
	/// direction.
	pub fn new(
		render_pass_builder: &mut RenderPassBuilder,
		fogs: DefaultListener<CreateMessage<ExponentialHeightFog>>,
		directional_lights: DefaultListener<CreateMessage<DirectionalLight>>,
		transform_listener: DefaultListener<TransformationUpdate>,
	) -> Self {
		let depth = render_pass_builder.read_from("depth");
		// The scene pipeline's marched fog light, if it marches one. A black texel with no distance stands in otherwise,
		// which the shader reads as no march.
		let fog_light = render_pass_builder
			.try_read_from(FOG_LIGHT_TARGET)
			.map(ghi::BaseImageHandle::from);

		let context = render_pass_builder.context();
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Exponential Height Fog Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// The shader fetches texels by coordinate, so the sampler only completes the texture bindings.
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let fog_light = fog_light.unwrap_or_else(|| {
			crate::rendering::lookup_textures::create_lookup_texture(
				context,
				"No Fog Light",
				ghi::Formats::RGBA16F,
				Extent::square(1),
				&[0; 8],
			)
			.into()
		});

		Self {
			resources: [
				simple_tile::Resource::combined_image_sampler("depth_texture", depth, sampler, ghi::Layouts::Read),
				simple_tile::Resource::combined_image_sampler("fog_light", fog_light, sampler, ghi::Layouts::Read),
				simple_tile::Resource::buffer("fog_parameters", parameters),
			],
			parameters,
			fogs,
			fog: None,
			sun: Sun::new(directional_lights, transform_listener),
		}
	}

	/// Adopts the newest fog and sun. Returns whether the fog changed.
	fn update(&mut self) -> bool {
		self.sun.update();
		let mut changed = false;
		while let Some(message) = self.fogs.read() {
			self.fog = Some(*message.data());
			changed = true;
		}
		changed
	}
}

/// Builds one sink's fog constants from the fog and the sun's RGB illuminance in lux and direction.
///
/// Everything that is the same for every pixel is computed here so the shader doesn't repeat it. The fog is written
/// pre-exposed like the scene, so both light terms include the sink's camera exposure.
fn fog_shader_data(
	fog: &ExponentialHeightFog,
	sun_illuminance: Vec3f,
	sun_direction: Option<math::UnitVector>,
	sink: &Sink,
) -> FogShaderData {
	let view = sink.view();
	// The translation column of the inverse view is the camera's world position.
	let camera = Vec3f::from(inverse(view.view()).get_column(3));
	let exposure = sink.exposure_scale();
	let albedo = fog.albedo();
	// A sky that delivers E lux to the ground evenly from every direction has a radiance of E / π, and fog lit evenly
	// from every direction scatters that radiance unchanged, whatever its phase function.
	let ambient = albedo * (fog.ambient_illuminance() / std::f32::consts::PI * exposure);
	// The Henyey-Greenstein phase function is (1 − g²) / 4π · (1 + g² − 2g cos θ)^-3/2. The constant factor rides on the
	// sunlight, and the shader evaluates the angular part.
	let anisotropy = fog.anisotropy();
	let squared = anisotropy * anisotropy;
	let phase_scale = (1.0 - squared) / (4.0 * std::f32::consts::PI);
	// Without a sun direction there is no sun to scatter.
	let (sun_direction, sun) = match sun_direction {
		Some(direction) => (
			[direction.x(), direction.y(), direction.z(), 0.0],
			albedo * sun_illuminance * (exposure * phase_scale),
		),
		None => ([0.0, 1.0, 0.0, 0.0], Vec3f::new(0.0, 0.0, 0.0)),
	};

	FogShaderData {
		pixel_to_camera_offset: pixel_to_camera_offset(view, sink.extent()).into(),
		layers: [
			FogLayerShaderData::new(fog.layer(), camera),
			// A missing second layer packs no density, which adds no fog.
			fog.second_layer()
				.map_or_else(FogLayerShaderData::zeroed, |layer| FogLayerShaderData::new(layer, camera)),
		],
		scattering: [1.0 + squared, 2.0 * anisotropy, fog.max_opacity(), 0.0],
		sun_direction,
		ambient_inscattering: [ambient.x, ambient.y, ambient.z, 0.0],
		sun_inscattering: [sun.x, sun.y, sun.z, 0.0],
	}
}

impl PixelStep for ExponentialHeightFogRenderPass {
	fn name(&self) -> &'static str {
		"exponential-height-fog"
	}

	fn output(&self) -> &'static str {
		"Exponential Height Fog"
	}

	fn resources(&self) -> &[simple_tile::Resource] {
		&self.resources
	}

	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sink: &Sink) -> bool {
		self.update();
		let Some(fog) = self.fog else {
			return false;
		};
		*frame.get_mut_dynamic_buffer_slice(self.parameters) =
			fog_shader_data(&fog, self.sun.illuminance, self.sun.direction, sink);
		true
	}

	fn needs_frame(&mut self) -> bool {
		self.update()
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, Value, output_slot};
	use math::{Point, UnitVector};
	use maths_rs::Vec3f;

	use crate::rendering::{
		ExponentialHeightFog, FogLayer,
		shader_vm_test::{
			PIXEL_TILE, assert_rgba_close, buffer, empty_image, link_pixel_chain, rgba, run_pixel, slot, texture_2d,
		},
	};

	/// The step shader, as a path under `assets/rendering`.
	const HEIGHT_FOG_BESL: &str = "height-fog.besl";

	/// Scene color behind the fog in every test.
	const SCENE: [f32; 4] = [0.2, 0.4, 0.6, 1.0];
	/// Reverse-Z depth that means no surface.
	const BACKGROUND: f32 = 0.0;

	/// The one-pixel camera every test looks through: at `camera`, looking along `direction`, 10 m to the far plane.
	fn sink(camera: Point, direction: UnitVector) -> crate::rendering::Sink {
		let view = crate::rendering::View::new_perspective(math::Degrees::new(60.0), 1.0, 0.1, 10.0, camera, direction);
		crate::rendering::Sink::new(view, utils::Extent::square(1), 0)
	}

	/// Fogs one pixel of [`SCENE`] at reverse-Z `depth` with the production constants for `fog`, a sun of
	/// `sun_illuminance` lux toward `sun_direction`, and the camera from [`sink`].
	fn fog_pixel(
		fog: ExponentialHeightFog,
		sun_illuminance: [f32; 3],
		sun_direction: Option<UnitVector>,
		camera: Point,
		direction: UnitVector,
		depth: f32,
	) -> [f32; 4] {
		fog_pixel_through(fog, sun_illuminance, sun_direction, &sink(camera, direction), [0, 0], depth)
	}

	/// Fogs `pixel` of a `sink`-sized image of [`SCENE`] at reverse-Z `depth`, with nothing marched. See [`fog_pixel`].
	fn fog_pixel_through(
		fog: ExponentialHeightFog,
		sun_illuminance: [f32; 3],
		sun_direction: Option<UnitVector>,
		sink: &crate::rendering::Sink,
		pixel: [u32; 2],
		depth: f32,
	) -> [f32; 4] {
		fog_pixel_marched(fog, sun_illuminance, sun_direction, sink, pixel, depth, [0.0; 4])
	}

	/// Fogs `pixel` of a `sink`-sized image of [`SCENE`] at reverse-Z `depth`, with `marched` in every texel of the
	/// half-resolution fog light. See [`fog_pixel`].
	fn fog_pixel_marched(
		fog: ExponentialHeightFog,
		sun_illuminance: [f32; 3],
		sun_direction: Option<UnitVector>,
		sink: &crate::rendering::Sink,
		pixel: [u32; 2],
		depth: f32,
		marched: [f32; 4],
	) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(link_pixel_chain(&[HEIGHT_FOG_BESL]));
		let parameter_slot = slot(&program, "fog_parameters");
		let data = super::fog_shader_data(
			&fog,
			Vec3f::new(sun_illuminance[0], sun_illuminance[1], sun_illuminance[2]),
			sun_direction,
			sink,
		);
		let mut parameters = buffer(&program, parameter_slot);
		for (name, value) in [
			(
				"pixel_to_camera_offset",
				Value::Mat4F(bytemuck::cast(data.pixel_to_camera_offset)),
			),
			("scattering", Value::Vec4F(data.scattering)),
			("sun_direction", Value::Vec4F(data.sun_direction)),
			("ambient_inscattering", Value::Vec4F(data.ambient_inscattering)),
			("sun_inscattering", Value::Vec4F(data.sun_inscattering)),
		] {
			parameters
				.write(name, value)
				.expect("Failed to initialize fog parameters. The most likely cause is a changed production buffer layout.");
		}
		for (index, layer) in data.layers.iter().enumerate() {
			for (field, value) in [
				("shape", layer.shape),
				("bounds_min", layer.bounds_min),
				("bounds_max", layer.bounds_max),
			] {
				parameters
					.write_indexed_field("layers", index, field, Value::Vec4F(value))
					.expect("Failed to initialize fog layers. The most likely cause is a changed production buffer layout.");
			}
		}
		// Every pixel holds the same depth and color, so only the view ray changes from pixel to pixel.
		let (width, height) = (sink.extent().width(), sink.extent().height());
		let texels = (width * height) as usize;
		let mut depth_texture = texture_2d(width, height, &vec![[depth, 0.0, 0.0, 1.0]; texels]);
		let mut scene = texture_2d(width, height, &vec![SCENE; texels]);
		let march_extent = sink.extent().scaled_down(2);
		let mut fog_light = texture_2d(
			march_extent.width(),
			march_extent.height(),
			&vec![marched; (march_extent.width() * march_extent.height()) as usize],
		);
		let mut result = empty_image(width, height);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(slot(&program, "depth_texture"), &mut depth_texture);
		descriptors.bind_texture(slot(&program, "source"), &mut scene);
		descriptors.bind_texture(slot(&program, "fog_light"), &mut fog_light);
		descriptors.bind_image(output_slot(0), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_pixel(&program, &mut descriptors, pixel, PIXEL_TILE, [width, height]);
		drop(descriptors);
		rgba(&result, pixel)
	}

	/// Returns the reverse-Z depth of a surface `distance` meters along the view axis of [`sink`].
	fn depth_at(distance: f32) -> f32 {
		let sink = sink(Point::origin(), UnitVector::z_axis());
		let clip = sink.view().projection() * maths_rs::Vec4f::new(0.0, 0.0, distance, 1.0);
		clip.z / clip.w
	}

	fn level() -> UnitVector {
		UnitVector::z_axis()
	}

	/// Verifies that a surface in uniform fog fades by Beer–Lambert's law toward the ambient sky's radiance.
	#[test]
	fn surface_fades_into_ambient_light_by_distance() {
		let density = 0.1;
		let sky_lux = std::f32::consts::PI;
		let fog = ExponentialHeightFog::new(FogLayer::new(density).with_height_falloff(0.0)).with_ambient_illuminance(sky_lux);
		let fogged = fog_pixel(fog, [0.0; 3], None, Point::origin(), level(), depth_at(5.0));

		// A sky of π lux has a radiance of 1 nit, which the fog scatters unchanged.
		let transmittance = (-density * 5.0f32).exp();
		let expected = SCENE.map(|channel| channel * transmittance + (1.0 - transmittance));
		assert_rgba_close(fogged, [expected[0], expected[1], expected[2], SCENE[3]], 1e-3);
	}

	/// Verifies that fog without density leaves the scene untouched, sky included.
	#[test]
	fn clear_air_preserves_the_scene() {
		let fog = ExponentialHeightFog::new(FogLayer::new(0.0)).with_ambient_illuminance(10_000.0);

		assert_rgba_close(
			fog_pixel(
				fog,
				[100_000.0; 3],
				Some(UnitVector::y_axis()),
				Point::origin(),
				level(),
				BACKGROUND,
			),
			SCENE,
			1e-6,
		);
	}

	/// Verifies that fog thins with altitude: the same surface hides less from a camera above the fog's base.
	#[test]
	fn fog_thins_with_altitude() {
		let fog = ExponentialHeightFog::new(FogLayer::new(0.2).with_height_falloff(0.5));
		let low = fog_pixel(fog, [0.0; 3], None, Point::origin(), level(), depth_at(5.0));
		let high = fog_pixel(fog, [0.0; 3], None, Point::new(0.0, 4.0, 0.0), level(), depth_at(5.0));

		// Unlit fog darkens what it hides, so the brighter result saw less fog.
		assert!(
			high[2] > low[2],
			"Fog didn't thin with height. The most likely cause is an inverted height falloff: low {low:?}, high {high:?}"
		);
	}

	/// Verifies that the horizon disappears into fog, while the sky straight up keeps most of its color.
	#[test]
	fn horizon_hides_while_zenith_stays_clear() {
		let fog = ExponentialHeightFog::new(FogLayer::new(0.02)).with_ambient_illuminance(std::f32::consts::PI);
		let horizon = fog_pixel(fog, [0.0; 3], None, Point::origin(), level(), BACKGROUND);
		let zenith = fog_pixel(
			fog,
			[0.0; 3],
			None,
			Point::origin(),
			// Straight up, nudged so the view's up axis stays usable.
			math::Vector::new(0.0, 1.0, 0.001).normalized().unwrap(),
			BACKGROUND,
		);

		assert_rgba_close(horizon, [1.0, 1.0, 1.0, SCENE[3]], 1e-3);
		// Straight up, the fog above the camera holds density / falloff of optical depth.
		let transmittance = (-0.02f32 / 0.2).exp();
		assert_rgba_close(
			zenith,
			SCENE.map(|channel| channel * transmittance + (1.0 - transmittance)),
			1e-3,
		);
	}

	/// Verifies that each pixel looks along its own view ray, top rows up and bottom rows down: below a fog's base,
	/// the top of the frame sees up into thinning fog and the bottom down into thicker fog.
	#[test]
	fn pixels_look_along_their_own_rays() {
		let view = crate::rendering::View::new_perspective(math::Degrees::new(90.0), 1.0, 0.1, 10.0, Point::origin(), level());
		let tall = crate::rendering::Sink::new(view, utils::Extent::rectangle(1, 2), 0);
		let fog = ExponentialHeightFog::new(FogLayer::new(0.1).with_height_falloff(1.0));
		let top = fog_pixel_through(fog, [0.0; 3], None, &tall, [0, 0], depth_at(5.0));
		let bottom = fog_pixel_through(fog, [0.0; 3], None, &tall, [0, 1], depth_at(5.0));

		// Unlit fog darkens what it hides, so the brighter pixel saw less fog.
		assert!(
			top[2] > bottom[2],
			"The fog is upside down. The most likely cause is a flipped pixel-to-view mapping: top {top:?}, bottom {bottom:?}"
		);
	}

	/// Verifies that the maximum opacity keeps a trace of the scene visible through the thickest fog.
	#[test]
	fn max_opacity_caps_the_fog() {
		let fog = ExponentialHeightFog::new(FogLayer::new(1.0)).with_max_opacity(0.75);
		let fogged = fog_pixel(fog, [0.0; 3], None, Point::origin(), level(), BACKGROUND);

		assert_rgba_close(fogged, [SCENE[0] * 0.25, SCENE[1] * 0.25, SCENE[2] * 0.25, SCENE[3]], 1e-4);
	}

	/// Verifies that a second layer adds its fog to the first: two uniform layers hide as much as one layer of their
	/// combined density.
	#[test]
	fn second_layer_adds_its_fog() {
		let uniform = |density| FogLayer::new(density).with_height_falloff(0.0);
		let stacked = ExponentialHeightFog::new(uniform(0.05)).with_second_layer(uniform(0.15));
		let combined = ExponentialHeightFog::new(uniform(0.2));

		assert_rgba_close(
			fog_pixel(stacked, [0.0; 3], None, Point::origin(), level(), depth_at(5.0)),
			fog_pixel(combined, [0.0; 3], None, Point::origin(), level(), depth_at(5.0)),
			1e-5,
		);
	}

	/// Verifies that a thin ground layer veils the floor below the camera but not a wall at eye height.
	#[test]
	fn ground_layer_hugs_the_floor() {
		let ground_mist =
			ExponentialHeightFog::new(FogLayer::new(0.0)).with_second_layer(FogLayer::new(0.5).with_height_falloff(4.0));
		let camera = Point::new(0.0, 1.5, 0.0);
		let eye_level = fog_pixel(ground_mist, [0.0; 3], None, camera, level(), depth_at(5.0));
		// Looking 30 degrees down, the view axis meets the floor 3 m away.
		let downward = math::Vector::new(0.0, -0.5, 0.866).normalized().unwrap();
		let floor = fog_pixel(ground_mist, [0.0; 3], None, camera, downward, depth_at(3.0));

		assert!(
			eye_level[2] > 0.95 * SCENE[2] && floor[2] < 0.9 * SCENE[2],
			"Ground mist doesn't hug the floor. The most likely cause is that the second layer ignores its height: eye level {eye_level:?}, floor {floor:?}"
		);
	}

	/// Fogs one pixel of a surface 5 m ahead through a uniform, unlit layer of density `0.2` confined to `bounds`.
	fn bounded_fog_pixel(bounds: math::AABB) -> [f32; 4] {
		let layer = FogLayer::new(0.2).with_height_falloff(0.0).with_bounds(bounds);
		fog_pixel(
			ExponentialHeightFog::new(layer),
			[0.0; 3],
			None,
			Point::origin(),
			level(),
			depth_at(5.0),
		)
	}

	/// Returns [`SCENE`] seen through `optical_depth` of unlit fog.
	fn behind_unlit_fog(optical_depth: f32) -> [f32; 4] {
		let transmittance = (-optical_depth).exp();
		let [r, g, b, a] = SCENE;
		[r * transmittance, g * transmittance, b * transmittance, a]
	}

	/// Verifies that a bounded layer fogs only the part of the view ray inside its box, whether the camera starts
	/// outside or inside it.
	#[test]
	fn bounded_layer_fogs_only_inside_its_box() {
		let ahead = math::AABB::new(Point::new(-1.0, -1.0, 2.0), Point::new(1.0, 1.0, 4.0));
		let around_camera = math::AABB::new(Point::new(-1.0, -1.0, -1.0), Point::new(1.0, 1.0, 3.0));

		// The ray crosses 2 m of the box ahead, and 3 m of the box around the camera before leaving it.
		assert_rgba_close(bounded_fog_pixel(ahead), behind_unlit_fog(0.2 * 2.0), 1e-4);
		assert_rgba_close(bounded_fog_pixel(around_camera), behind_unlit_fog(0.2 * 3.0), 1e-4);
	}

	/// Verifies that a bounded layer leaves views that miss its box untouched, such as a courtyard's mist seen from
	/// a gallery beside it, and boxes beyond the surface.
	#[test]
	fn bounded_layer_spares_rays_that_miss_its_box() {
		let beside = math::AABB::new(Point::new(2.0, -1.0, 0.0), Point::new(4.0, 1.0, 10.0));
		let behind_surface = math::AABB::new(Point::new(-1.0, -1.0, 6.0), Point::new(1.0, 1.0, 9.0));

		assert_rgba_close(bounded_fog_pixel(beside), SCENE, 1e-6);
		assert_rgba_close(bounded_fog_pixel(behind_surface), SCENE, 1e-6);
	}

	/// Verifies that the fog glows brighter looking toward the sun than away from it.
	#[test]
	fn fog_scatters_sunlight_forward() {
		let fog = ExponentialHeightFog::new(FogLayer::new(1.0).with_height_falloff(0.0));
		let toward = fog_pixel(fog, [10.0; 3], Some(level()), Point::origin(), level(), BACKGROUND);
		let away = fog_pixel(fog, [10.0; 3], Some(-level()), Point::origin(), level(), BACKGROUND);

		assert!(
			toward[0] > away[0] && away[0] > 0.0,
			"Fog doesn't scatter sunlight forward. The most likely cause is an inverted phase function: toward {toward:?}, away {away:?}"
		);
	}

	/// Verifies that the fog's light is written pre-exposed, like the scene it covers.
	#[test]
	fn fog_light_is_pre_exposed() {
		let exposure = 0.25;
		let exposed = |fog, sun_illuminance| {
			let sink = sink(Point::origin(), level()).with_exposure_scale(exposure);
			fog_pixel_through(fog, sun_illuminance, Some(-level()), &sink, [0, 0], BACKGROUND)
		};
		let opaque = FogLayer::new(1.0).with_height_falloff(0.0);
		// A sky of π lux has a radiance of 1 nit.
		let sky_lit = ExponentialHeightFog::new(opaque).with_ambient_illuminance(std::f32::consts::PI);
		// Fog that scatters evenly sends 1 / 4π of the sunlight per steradian, so a sun of 4π lux gives 1 nit.
		let sun_lit = ExponentialHeightFog::new(opaque).with_anisotropy(0.0);

		assert_rgba_close(exposed(sky_lit, [0.0; 3]), [exposure, exposure, exposure, SCENE[3]], 1e-5);
		assert_rgba_close(
			exposed(sun_lit, [4.0 * std::f32::consts::PI; 3]),
			[exposure, exposure, exposure, SCENE[3]],
			1e-5,
		);
	}

	/// Verifies that marched light replaces the unshadowed sun: the fog shows the sky's light by its opacity and adds
	/// the march's light, while the sun's own glow drops out.
	#[test]
	fn marched_light_replaces_the_unshadowed_sun() {
		let density = 0.1;
		let fog = ExponentialHeightFog::new(FogLayer::new(density).with_height_falloff(0.0))
			.with_ambient_illuminance(std::f32::consts::PI);
		let sink = sink(Point::origin(), level());
		let marched = [0.5, 0.25, 0.125, (1.0f32 + 5.0).log2()];
		let fogged = fog_pixel_marched(fog, [10_000.0; 3], Some(level()), &sink, [0, 0], depth_at(5.0), marched);

		// A sky of π lux has a radiance of 1 nit.
		let transmittance = (-density * 5.0f32).exp();
		let expected = [0, 1, 2].map(|channel| SCENE[channel] * transmittance + (1.0 - transmittance) + marched[channel]);
		assert_rgba_close(fogged, [expected[0], expected[1], expected[2], SCENE[3]], 1e-3);
	}

	/// Verifies that the maximum opacity scales the marched light with the rest of the fog.
	#[test]
	fn max_opacity_caps_the_marched_light() {
		let fog = ExponentialHeightFog::new(FogLayer::new(1.0)).with_max_opacity(0.5);
		let sink = sink(Point::origin(), level());
		// The fog is opaque by the background, so its opacity of one is held to a half.
		let marched = [1.0, 1.0, 1.0, (1.0f32 + 100_000.0).log2()];
		let fogged = fog_pixel_marched(fog, [0.0; 3], None, &sink, [0, 0], BACKGROUND, marched);

		assert_rgba_close(
			fogged,
			[SCENE[0] * 0.5 + 0.5, SCENE[1] * 0.5 + 0.5, SCENE[2] * 0.5 + 0.5, SCENE[3]],
			1e-4,
		);
	}
}
