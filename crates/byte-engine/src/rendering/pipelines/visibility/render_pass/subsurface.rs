//! Subsurface scattering: the diffuse light of skin, wax, and other subsurface materials, spread beneath their surface.
//!
//! Opaque material evaluation leaves each subsurface pixel's diffuse light out of the lit image and writes it to the
//! [`SubsurfaceTargets`] instead. [`SubsurfacePass`] then spreads it along Christensen and Burley's diffusion profile
//! and adds it to the lit image, before the background and the transparent phase. Build it in with
//! `render.subsurface.enabled`; see the subsurface scattering design note.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::min_depth_pyramid::{ScreenViewData, screen_view_data};
use super::{ComputeStage, Pipelines};
use crate::rendering::lookup_textures::blue_noise_slice;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::{PipelineManagerClient, Sink};

/// The spreading pipeline, named as [`Pipelines::request`] takes it.
pub(in super::super) const SUBSURFACE_PIPELINES: [&str; 1] = ["subsurface"];

/// The `SubsurfaceTargets` struct holds the full-size images opaque material evaluation leaves for the subsurface pass.
///
/// They are render targets so the renderer sizes them with the sink and the inspector can capture them by name. Create
/// them with [`create_subsurface_targets`], then hand them to material evaluation and to [`SubsurfacePass::new`].
#[derive(Clone, Copy)]
pub(crate) struct SubsurfaceTargets {
	/// The diffuse light before scattering, weighted by the square root of the albedo and pre-exposed, named
	/// `Subsurface Diffuse`.
	pub(crate) diffuse: ghi::BaseImageHandle,
	/// The profile scale `d` per channel, in meters, named `Subsurface Shape`.
	pub(crate) shape: ghi::BaseImageHandle,
	/// The square root of the albedo, with alpha 1 at subsurface pixels, named `Subsurface Albedo`.
	pub(crate) albedo: ghi::BaseImageHandle,
}

/// Creates the subsurface targets for the sink that `render_pass_builder` sets up.
pub(crate) fn create_subsurface_targets(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> SubsurfaceTargets {
	let mut target = |name, format| {
		render_pass_builder
			.create_scaled_render_target(
				ghi::image::Builder::new(format, ghi::Uses::Storage | ghi::Uses::Image).name(name),
				1,
			)
			.into()
	};
	SubsurfaceTargets {
		diffuse: target("Subsurface Diffuse", ghi::Formats::RGBA16F),
		shape: target("Subsurface Shape", ghi::Formats::RGBA16F),
		albedo: target("Subsurface Albedo", ghi::Formats::RGBA8UNORM),
	}
}

/// The `SubsurfaceShaderParameters` struct carries the blue noise slice and whether the disk turns, as
/// `SubsurfaceParameters` in `layouts/subsurface.besl` declares them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct SubsurfaceShaderParameters {
	noise_slice: u32,
	rotates: u32,
}

/// The `SubsurfacePass` struct spreads the diffuse light of a sink's subsurface pixels after opaque material
/// evaluation.
pub(super) struct SubsurfacePass {
	descriptor_set: ghi::DescriptorSetHandle,
	/// The full-resolution camera constants the pass rebuilds view-space distances with.
	view_data: ghi::BufferHandle<ScreenViewData>,
	parameters: ghi::BufferHandle<SubsurfaceShaderParameters>,
	pub(super) pipelines: Pipelines<1>,
}

impl SubsurfacePass {
	/// Binds this frame's `depth`, the `targets` material evaluation writes, the `lit` image the pass adds to, and the
	/// engine's `blue_noise`, which turns each pixel's disk while the sink is jittered.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		lit: ghi::ImageHandle,
		targets: SubsurfaceTargets,
		blue_noise: ghi::ImageHandle,
	) -> Self {
		let subsurface_layout = set_layouts::layout(set_layouts::SUBSURFACE);
		let descriptor_set = subsurface_layout.create_set(context, "Subsurface Descriptor Set");
		let view_data = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Subsurface View Data")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		let parameters = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Subsurface Parameters")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		let binder = subsurface_layout.binder(descriptor_set);
		context.write(&[
			binder.buffer("view", view_data),
			binder.texture("depth", depth),
			binder.texture("diffuse_light", targets.diffuse),
			binder.texture("profile_scale", targets.shape),
			binder.texture("albedo_root", targets.albedo),
			binder.image("lit_map", lit),
			binder.buffer("parameters", parameters),
			binder.texture("blue_noise", blue_noise),
		]);

		Self {
			descriptor_set,
			view_data,
			parameters,
			pipelines: Pipelines::request(pipeline_manager, SUBSURFACE_PIPELINES),
		}
	}

	/// Uploads this frame's full-resolution camera constants and returns the dispatch, which records after opaque
	/// material evaluation.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		[pipeline]: [ghi::PipelineHandle; 1],
	) -> ComputeStage<1> {
		let extent = sink.extent();
		*frame.map(self.view_data) = screen_view_data(sink, extent);
		*frame.map(self.parameters) = SubsurfaceShaderParameters {
			noise_slice: blue_noise_slice(sink, frame.key().frame_index()),
			rotates: sink.is_jittered() as u32,
		};

		ComputeStage {
			label: "Subsurface",
			pipeline,
			descriptor_sets: [self.descriptor_set],
			extent,
			workgroup: Extent::new(8, 8, 1),
		}
	}
}
