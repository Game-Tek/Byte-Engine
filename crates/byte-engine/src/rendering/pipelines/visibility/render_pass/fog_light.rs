//! Fog light: the light the scene's lights scatter toward the camera through the newest
//! [`crate::rendering::ExponentialHeightFog`], shadowed by their shadow maps, marched at half resolution into
//! [`FOG_LIGHT_TARGET`].
//!
//! The height fog pass blends the target over the scene with the fog's closed-form transmittance and the sky's light;
//! see [`crate::rendering::render_passes::height_fog`]. The light cluster grid bounds the surfaces each pixel shows,
//! not the fog in front of them, so a tile pass first buckets the light table into a fixed grid of screen tiles by the
//! fog each tile sees. Then the march visits only its tile's lights.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::pod::Mat4x3f;
use math::inverse;
use maths_rs::Vec3f;
use utils::Extent;

use super::super::shader_data::LightingData;
use super::shadows::ShadowMaps;
use super::{ComputeStage, Pipelines};
use crate::rendering::fog::{FOG_LIGHT_TARGET, FogLayerShaderData, pixel_to_camera_offset};
use crate::rendering::lookup_textures::create_blue_noise_texture;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::{ExponentialHeightFog, FogLayer, PipelineManagerClient, Sink};

/// The march runs at half the sink resolution.
const FOG_LIGHT_RESOLUTION_DIVISOR: u32 = 2;
/// The tile grid's columns and rows, and the mask words of each tile, as `fog_light.besl` declares them.
const FOG_LIGHT_TILE_COLUMNS: u32 = 32;
const FOG_LIGHT_TILE_ROWS: u32 = 18;
const FOG_LIGHT_TILE_MASK_WORDS: u32 = 32;
const FOG_LIGHT_TILE_MASK_WORD_COUNT: usize =
	(FOG_LIGHT_TILE_COLUMNS * FOG_LIGHT_TILE_ROWS * FOG_LIGHT_TILE_MASK_WORDS) as usize;
/// The density per meter below which a layer's fog is too thin for a point or cone light to light.
const FOG_DENSITY_CUTOFF: f32 = 0.0001;

/// The tile pass and the march, named as [`Pipelines::request`] takes them.
const FOG_LIGHT_PIPELINES: [&str; 2] = ["fog-light-tiles", "fog-light"];

/// Creates the fog light target for the sink that `render_pass_builder` sets up.
///
/// It is a render target so the renderer sizes it with the sink, clears it on frames the march doesn't run, and orders
/// the height fog pass's read after this frame's write. Next, pass it to the visibility render pass, which hands it to
/// [`FogLightPass::new`].
pub(crate) fn create_fog_light_target(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> ghi::BaseImageHandle {
	render_pass_builder
		.create_scaled_render_target(
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Storage | ghi::Uses::Image)
				.name(FOG_LIGHT_TARGET)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
			FOG_LIGHT_RESOLUTION_DIVISOR,
		)
		.into()
}

/// The `FogVolume` struct bounds where one fog layer is dense enough for a point or cone light to light, so the tile
/// pass leaves out lights that reach no fog.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FogVolume {
	pub(crate) min: [f32; 4],
	pub(crate) max: [f32; 4],
}

impl FogVolume {
	/// A box whose minimum lies above its maximum, which holds no fog.
	const EMPTY: Self = Self {
		min: [1.0, 1.0, 1.0, 0.0],
		max: [-1.0, -1.0, -1.0, 0.0],
	};
}

/// Returns the world-space box where `layer` is at least [`FOG_DENSITY_CUTOFF`] dense: its own box, if any, below the
/// height where its density falls to the cutoff.
fn fog_volume(layer: Option<FogLayer>) -> FogVolume {
	let Some(layer) = layer.filter(|layer| layer.density() > FOG_DENSITY_CUTOFF) else {
		return FogVolume::EMPTY;
	};
	// The density falls to the cutoff ln(density / cutoff) / falloff meters above the base height.
	let top = if layer.height_falloff() > 0.0 {
		layer.base_height() + (layer.density() / FOG_DENSITY_CUTOFF).ln() / layer.height_falloff()
	} else {
		f32::MAX
	};
	let (min, mut max) = match layer.bounds() {
		Some(bounds) => (bounds.min().into_maths(), bounds.max().into_maths()),
		None => (
			Vec3f::new(-f32::MAX, -f32::MAX, -f32::MAX),
			Vec3f::new(f32::MAX, f32::MAX, f32::MAX),
		),
	};
	max.y = max.y.min(top);
	if min.y > max.y {
		return FogVolume::EMPTY;
	}
	FogVolume {
		min: [min.x, min.y, min.z, 0.0],
		max: [max.x, max.y, max.z, 0.0],
	}
}

/// The `FogLightShaderParameters` struct carries one sink's fog and camera to both fog light stages, as
/// `FogLightParameters` in `fog_light.besl` declares them.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FogLightShaderParameters {
	pub(crate) pixel_to_camera_offset: ghi::pod::Mat4f,
	/// Maps world space to the view space the tiles are laid out in, where depth grows along +Z.
	pub(crate) view: Mat4x3f,
	pub(crate) layers: [FogLayerShaderData; 2],
	pub(crate) volumes: [FogVolume; 2],
	pub(crate) camera_position: [f32; 4],
	pub(crate) camera_forward: [f32; 4],
	/// 1 + g², 2g, and (1 − g²) / 4π for the fog's anisotropy g.
	pub(crate) phase: [f32; 4],
	pub(crate) albedo: [f32; 4],
	/// The view-space X/Z and Y/Z slopes at the right and top edges of the image.
	pub(crate) edge_slopes: [f32; 2],
	pub(crate) samples: u32,
	_padding: u32,
}

const _: () = assert!(std::mem::offset_of!(FogLightShaderParameters, view) == 64);
const _: () = assert!(std::mem::offset_of!(FogLightShaderParameters, layers) == 112);
const _: () = assert!(std::mem::offset_of!(FogLightShaderParameters, volumes) == 208);
const _: () = assert!(std::mem::offset_of!(FogLightShaderParameters, camera_position) == 272);
const _: () = assert!(std::mem::offset_of!(FogLightShaderParameters, edge_slopes) == 336);
const _: () = assert!(std::mem::size_of::<FogLightShaderParameters>() == 352);

/// Builds one sink's fog light constants from `fog`.
///
/// The march multiplies the light table's exposure into the light itself, so the constants don't depend on the
/// sink's exposure.
pub(crate) fn fog_light_shader_parameters(fog: &ExponentialHeightFog, sink: &Sink) -> FogLightShaderParameters {
	let view = sink.view();
	let inverse_view = inverse(view.view());
	// The translation column of the inverse view is the camera's world position, and its third column the view axis.
	let camera = Vec3f::from(inverse_view.get_column(3));
	let forward = Vec3f::from(inverse_view.get_column(2));
	let projection = view.projection();
	let anisotropy = fog.anisotropy();
	let squared = anisotropy * anisotropy;
	let albedo = fog.albedo();

	FogLightShaderParameters {
		pixel_to_camera_offset: pixel_to_camera_offset(view, sink.extent()).into(),
		view: view.view().into(),
		layers: [
			FogLayerShaderData::new(fog.layer(), camera),
			// A missing second layer packs no density, which adds no fog.
			fog.second_layer()
				.map_or_else(bytemuck::Zeroable::zeroed, |layer| FogLayerShaderData::new(layer, camera)),
		],
		volumes: [fog_volume(Some(fog.layer())), fog_volume(fog.second_layer())],
		camera_position: [camera.x, camera.y, camera.z, 0.0],
		camera_forward: [forward.x, forward.y, forward.z, 0.0],
		phase: [
			1.0 + squared,
			2.0 * anisotropy,
			(1.0 - squared) / (4.0 * std::f32::consts::PI),
			0.0,
		],
		albedo: [albedo.x, albedo.y, albedo.z, 0.0],
		edge_slopes: [1.0 / projection[0], 1.0 / projection[5]],
		samples: fog.light_samples(),
		_padding: 0,
	}
}

/// The `FogLightPass` struct lights one sink's height fog with the scene's lights and shadow maps, so the height fog
/// pass can show light shafts and the glow of lamps in the mist.
///
/// It runs after the shadow maps are drawn and the opaque depth exists. Create its target with
/// [`create_fog_light_target`], then call [`Self::prepare`] in each frame that has a fog.
pub(super) struct FogLightPass {
	/// The scene set, whose views hold the shadow maps' projections, then the fog light set.
	descriptor_sets: [ghi::DescriptorSetHandle; 2],
	pipelines: Pipelines<2>,
	parameters: ghi::DynamicBufferHandle<FogLightShaderParameters>,
}

impl FogLightPass {
	/// Wires the light table, the sink's depth, the shadow maps, and `target`, and requests both pipelines.
	///
	/// `base_descriptor_set` is the sink's base visibility set. `depth` is this frame's copy of the sink's reverse-Z
	/// depth.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
		depth: ghi::BaseImageHandle,
		shadow_maps: &ShadowMaps,
		target: ghi::BaseImageHandle,
	) -> Self {
		let descriptor_set = set_layouts::FOG_LIGHT.create_set(context, "Fog Light Descriptor Set");
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Fog Light Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let tile_masks: ghi::BufferHandle<[u32; FOG_LIGHT_TILE_MASK_WORD_COUNT]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Fog Light Tile Masks")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let blue_noise = create_blue_noise_texture(context);
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest),
		);
		// Gathers ignore filtering, and the border keeps a point light's cube lookups on their own texels.
		let shadow_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {}),
		);
		let fog_light = set_layouts::FOG_LIGHT.binder(descriptor_set);
		let mut writes = vec![
			fog_light.buffer("lighting_data", lighting_buffer),
			fog_light.buffer("parameters", parameters),
			fog_light.buffer("tile_masks", tile_masks),
			fog_light.sampled("depth", depth, point_sampler),
			fog_light.sampled("blue_noise", blue_noise, point_sampler),
			fog_light.sampled("directional_shadow_map", shadow_maps.directional, shadow_sampler),
			fog_light.image("scattered_light", target),
		];
		// One image of each local shadow-map size class, which a light picks by its class.
		for (size_class, (cone, point)) in shadow_maps.cone.iter().zip(&shadow_maps.point).enumerate() {
			writes.extend([
				fog_light.sampled_element("cone_shadow_maps", *cone, shadow_sampler, size_class as u32),
				fog_light.sampled_element("point_shadow_maps", *point, shadow_sampler, size_class as u32),
			]);
		}
		context.write(&writes);

		Self {
			descriptor_sets: [base_descriptor_set, descriptor_set],
			pipelines: Pipelines::request(pipeline_manager, FOG_LIGHT_PIPELINES),
			parameters,
		}
	}

	/// Uploads this frame's fog and camera constants for `sink`, and returns the tile and march dispatches, or `None`
	/// while either pipeline is still compiling. The height fog pass then lights the fog without shadows, so the frame
	/// doesn't wait.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		sink: &Sink,
		fog: &ExponentialHeightFog,
	) -> Option<[ComputeStage<2>; 2]> {
		let [tiles, march] = self.pipelines.resolve(pipeline_manager)?;
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = fog_light_shader_parameters(fog, sink);
		frame.sync_buffer(self.parameters);
		// One workgroup per tile, and one thread per mask word of 32 lights.
		let tile_workgroup = FOG_LIGHT_TILE_MASK_WORDS;
		Some([
			ComputeStage {
				label: "Fog Light Tiles",
				pipeline: tiles,
				descriptor_sets: self.descriptor_sets,
				extent: Extent::line(FOG_LIGHT_TILE_COLUMNS * FOG_LIGHT_TILE_ROWS * tile_workgroup),
				workgroup: Extent::line(tile_workgroup),
			},
			ComputeStage {
				label: "Fog Light",
				pipeline: march,
				descriptor_sets: self.descriptor_sets,
				extent: sink.extent().scaled_down(FOG_LIGHT_RESOLUTION_DIVISOR),
				workgroup: Extent::new(8, 8, 1),
			},
		])
	}
}
