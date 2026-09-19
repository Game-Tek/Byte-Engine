use std::{collections::HashMap, sync::Arc};

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
	types::Size as _,
};
use utils::{Box, Extent, RGBA};

use super::{
	element::ElementHandle as _,
	layout::{ClipMask, Geometry, engine},
	style::{Color, EdgeFeather, LayerKind},
	transform::Rotation,
};
use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
	},
	ui::{
		components::curve::{CurvePoint, CurveSegment},
		font::TextSystem,
	},
};

// Group draw preparation and geometry generation by responsibility.
mod cache;
mod data;
mod geometry;
mod slug;
mod text;

#[cfg(test)]
mod cpu_tests;
#[cfg(test)]
mod damage_tests;
#[cfg(test)]
mod subtree_tests;

#[cfg(all(test, feature = "ui-render-bench"))]
mod benchmarks;

use cache::*;
use data::*;
use geometry::*;
use slug::*;
use text::*;

/// The text renderer every [`UiRenderPass`] uses. Set it to [`UiTextMode::Atlas`] to draw CPU-rasterized glyphs instead.
const UI_TEXT_MODE: UiTextMode = UiTextMode::Slug;

/// The `UiTextMode` enum names the UI's two interchangeable text renderers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiTextMode {
	/// The GPU computes glyph coverage per pixel from outline curves. See [`slug`].
	Slug,
	/// The CPU rasterizes glyphs per pixel size into one coverage atlas that quads sample. See [`text`].
	Atlas,
}

/// The `UiText` enum owns the glyph data and GPU resources of the active text renderer.
///
/// Both renderers emit glyph primitives for the same ubershader, so the rest of the pass only
/// differs in which variant prepares the frame.
enum UiText {
	Slug {
		glyphs: UiGlyphCurves,
		curve_buffer: ghi::BufferHandle<[[f32; 4]; UI_GLYPH_CURVE_CAPACITY]>,
		band_buffer: ghi::BufferHandle<[u32; UI_GLYPH_BAND_CAPACITY]>,
	},
	Atlas {
		atlas: UiGlyphAtlas,
		image: ghi::BaseImageHandle,
	},
}

impl UiText {
	/// Identifies the current glyph packing; a prepared frame built against an older one is stale.
	fn generation(&self) -> u64 {
		match self {
			Self::Slug { glyphs, .. } => glyphs.generation(),
			Self::Atlas { atlas, .. } => atlas.generation(),
		}
	}
}

/// The `UiRenderPass` struct draws the UI into a persistent layer and composites the scene under it.
///
/// Every UI primitive is one record in a storage buffer that a single ubershader draws without
/// vertex or index buffers. A frame is one draw per damaged region, plus one more for each
/// backdrop blur, because a blur reads the layer drawn below it.
pub struct UiRenderPass {
	surface_revision: Option<engine::RenderRevision>,
	caches: UiGeometryCaches,
	masks: UiMaskTable,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	pipeline: crate::rendering::PipelineRef,
	/// The ubershader with blending disabled: a scissored quad is the hardware clear for one damaged region.
	clear_pipeline: crate::rendering::PipelineRef,
	primitive_buffer: ghi::BufferHandle<[UiPrimitive; MAX_UI_PRIMITIVES]>,
	mask_buffer: ghi::BufferHandle<[UiClipMaskEntry; MAX_UI_MASKS]>,
	descriptor_set: ghi::DescriptorSetHandle,
	image_sampler: ghi::SamplerHandle,
	image_textures: HashMap<u64, UiImageTexture>,
	text: UiText,
	blur_downsample_pipeline: crate::rendering::PipelineRef,
	blur_filter_pipeline: crate::rendering::PipelineRef,
	blur_downsample_workgroup: Extent,
	blur_filter_workgroup: Extent,
	blur_sampler: ghi::SamplerHandle,
	blur_half_downsample_descriptor_set: ghi::DescriptorSetHandle,
	blur_full_x_descriptor_set: ghi::DescriptorSetHandle,
	blur_full_y_descriptor_set: ghi::DescriptorSetHandle,
	blur_half_x_descriptor_set: ghi::DescriptorSetHandle,
	blur_half_y_descriptor_set: ghi::DescriptorSetHandle,
	blur_full_scratch: ghi::BaseImageHandle,
	blur_full_output: ghi::BaseImageHandle,
	blur_half_source: ghi::BaseImageHandle,
	blur_half_scratch: ghi::BaseImageHandle,
	blur_half_output: ghi::BaseImageHandle,
	blur_backdrop: ghi::BaseImageHandle,
	/// Whether the ubershader's blur bindings hold the blur outputs, which exist once a blur sizes them.
	blur_textures_bound: bool,
	/// Persistent premultiplied UI layer; only damaged regions are cleared and redrawn.
	layer: ghi::BaseImageHandle,
	/// Scene under layer for one region: the frame output at full extent, or a blur's backdrop.
	composite_pipeline: crate::rendering::PipelineRef,
	composite_descriptor_set: ghi::DescriptorSetHandle,
	blur_resolve_descriptor_set: ghi::DescriptorSetHandle,
	region_workgroup: Extent,
	bypass_pass: crate::rendering::render_passes::blit::ImageBypassPass,
	data: UiDrawList,
	render_revision: Option<engine::RenderRevision>,
	/// Layout-unit damage of the adopted render and the revision it is relative to.
	damage_base: Option<engine::RenderRevision>,
	damage_rects: Vec<Geometry>,
	/// Revision and extent whose pixels the layer currently holds.
	layer_revision: Option<engine::RenderRevision>,
	layer_extent: Option<Extent>,
	damage: Vec<UiPixelRegion>,
	prepared: Option<UiPreparedFrame>,
	reported_capacity_limit: bool,
	reported_dropped_glyphs: bool,
	reported_image_limit: bool,
	text_system: TextSystem,
}

impl Entity for UiRenderPass {}

impl UiRenderPass {
	/// Creates a UI pass and all GPU resources used to draw layout primitives.
	// Keep the UI pipeline and fixed buffer setup together because every handle is required by frame preparation.
	#[allow(clippy::too_many_lines)]
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let source = render_pass_builder.read_from("main");
		// The layer outlives frames: damaged regions are redrawn into it and the scene is composited under it every frame.
		let main_attachment = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				MAIN_ATTACHMENT_FORMAT,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage,
			)
			.name("UI Layer"),
		);
		let output = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Storage | ghi::Uses::Image).name("UI"),
		);

		let pipeline_manager = render_pass_builder.pipeline_manager().clone();
		let pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/ui.pipeline");
		let clear_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/layer-clear.pipeline");
		let blur_downsample_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/ui/backdrop-blur-downsample.pipeline");
		let blur_filter_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/backdrop-blur-filter.pipeline");
		let composite_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/ui/composite.pipeline");
		let blur_downsample_workgroup = Extent::square(16);
		let blur_filter_workgroup = Extent::square(16);
		let region_workgroup = Extent::square(UI_REGION_WORKGROUP);
		let output: ghi::ImageOrSwapchain = output.into();

		let context = render_pass_builder.context();

		let primitive_buffer: ghi::BufferHandle<[UiPrimitive; MAX_UI_PRIMITIVES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("UI Primitives")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let mask_buffer: ghi::BufferHandle<[UiClipMaskEntry; MAX_UI_MASKS]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("UI Clip Masks")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let image_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		// Every texture binding of the ubershader needs a texture, so unused ones share this one.
		let unused_texture: ghi::BaseImageHandle = context
			.build_image(
				ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image | ghi::Uses::TransferDestination)
					.name("UI Unused Texture")
					.extent(Extent::square(1))
					.device_accesses(ghi::DeviceAccesses::HostToDevice),
			)
			.into();
		let descriptor_set = context.create_descriptor_set(Some("UI"));
		context.write(&[
			ghi::DescriptorWrite::buffer(descriptor_set, UI_PRIMITIVES_SLOT, primitive_buffer.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, UI_MASKS_SLOT, mask_buffer.into()),
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				UI_BLUR_FULL_SLOT,
				unused_texture,
				image_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				UI_BLUR_HALF_SLOT,
				unused_texture,
				image_sampler,
				ghi::Layouts::Read,
			),
		]);
		for slot in 0..UI_TEXTURE_SLOTS {
			context.write(&[ghi::DescriptorWrite::combined_image_sampler_array(
				descriptor_set,
				UI_TEXTURES_SLOT,
				unused_texture,
				image_sampler,
				ghi::Layouts::Read,
				slot,
			)]);
		}
		let text = match UI_TEXT_MODE {
			UiTextMode::Slug => {
				let curve_buffer = context.build_buffer(
					ghi::buffer::Builder::new(ghi::Uses::Storage)
						.name("UI Glyph Curves")
						.device_accesses(ghi::DeviceAccesses::HostToDevice),
				);
				let band_buffer = context.build_buffer(
					ghi::buffer::Builder::new(ghi::Uses::Storage)
						.name("UI Glyph Bands")
						.device_accesses(ghi::DeviceAccesses::HostToDevice),
				);
				context.write(&[
					ghi::DescriptorWrite::buffer(descriptor_set, UI_GLYPH_CURVES_SLOT, curve_buffer.into()),
					ghi::DescriptorWrite::buffer(descriptor_set, UI_GLYPH_BANDS_SLOT, band_buffer.into()),
				]);
				UiText::Slug {
					glyphs: UiGlyphCurves::new(UI_GLYPH_CURVE_CAPACITY, UI_GLYPH_BAND_CAPACITY),
					curve_buffer,
					band_buffer,
				}
			}
			UiTextMode::Atlas => {
				let atlas = UiGlyphAtlas::new(UI_GLYPH_ATLAS_INITIAL_SIZE);
				// Linear coverage sampling preserves fractional translation and residual bucket scaling.
				let image: ghi::BaseImageHandle = context
					.build_image(
						ghi::image::Builder::new(UI_GLYPH_ATLAS_FORMAT, ghi::Uses::Image | ghi::Uses::TransferDestination)
							.name("UI Glyph Atlas")
							.extent(atlas.extent())
							.device_accesses(ghi::DeviceAccesses::HostToDevice),
					)
					.into();
				let sampler = context.build_sampler(
					ghi::sampler::Builder::new()
						.filtering_mode(ghi::FilteringModes::Linear)
						.mip_map_mode(ghi::FilteringModes::Closest)
						.addressing_mode(ghi::SamplerAddressingModes::Clamp),
				);
				context.write(&[
					ghi::DescriptorWrite::combined_image_sampler_array(
						descriptor_set,
						UI_TEXTURES_SLOT,
						image,
						sampler,
						ghi::Layouts::Read,
						UI_ATLAS_TEXTURE_SLOT,
					),
					// No atlas glyph reads outline data, but the shader's bindings still need a buffer.
					ghi::DescriptorWrite::buffer(descriptor_set, UI_GLYPH_CURVES_SLOT, mask_buffer.into()),
					ghi::DescriptorWrite::buffer(descriptor_set, UI_GLYPH_BANDS_SLOT, mask_buffer.into()),
				]);
				UiText::Atlas { atlas, image }
			}
		};
		let blur_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let blur_full_scratch = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Full Scratch"),
		);
		let blur_full_scratch_image: ghi::BaseImageHandle = blur_full_scratch.into();
		let blur_full_output = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Full Output"),
		);
		let blur_full_output_image: ghi::BaseImageHandle = blur_full_output.into();
		let blur_half_source = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Half Source"),
		);
		let blur_half_source_image: ghi::BaseImageHandle = blur_half_source.into();
		let blur_half_scratch = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Half Scratch"),
		);
		let blur_half_scratch_image: ghi::BaseImageHandle = blur_half_scratch.into();
		let blur_half_output = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Half Output"),
		);
		let blur_half_output_image: ghi::BaseImageHandle = blur_half_output.into();
		let blur_backdrop = context.build_dynamic_image(
			ghi::image::Builder::new(MAIN_ATTACHMENT_FORMAT, ghi::Uses::Image | ghi::Uses::Storage)
				.name("UI Backdrop Blur Source"),
		);
		let blur_backdrop_image: ghi::BaseImageHandle = blur_backdrop.into();
		let main_attachment_image: ghi::BaseImageHandle = main_attachment.into();
		let source_image: ghi::BaseImageHandle = source.into();
		let blur_resolve_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Resolve"));
		let composite_descriptor_set = context.create_descriptor_set(Some("UI Composite"));
		context.write(&[
			ghi::DescriptorWrite::image(
				composite_descriptor_set,
				ghi::ResourceSlot::new(0),
				source_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::image(
				composite_descriptor_set,
				ghi::ResourceSlot::new(1),
				main_attachment_image,
				ghi::Layouts::General,
			),
			match output {
				ghi::ImageOrSwapchain::Image(image) => ghi::DescriptorWrite::image(
					composite_descriptor_set,
					ghi::ResourceSlot::new(2),
					image,
					ghi::Layouts::General,
				),
				ghi::ImageOrSwapchain::Swapchain(swapchain) => {
					ghi::DescriptorWrite::swapchain(composite_descriptor_set, ghi::ResourceSlot::new(2), swapchain)
				}
			},
			ghi::DescriptorWrite::image(
				blur_resolve_descriptor_set,
				ghi::ResourceSlot::new(0),
				source_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::image(
				blur_resolve_descriptor_set,
				ghi::ResourceSlot::new(1),
				main_attachment_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::image(
				blur_resolve_descriptor_set,
				ghi::ResourceSlot::new(2),
				blur_backdrop_image,
				ghi::Layouts::General,
			),
		]);
		let blur_half_downsample_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Half Downsample"));
		let blur_full_x_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Full X"));
		let blur_full_y_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Full Y"));
		let blur_half_x_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Half X"));
		let blur_half_y_descriptor_set = context.create_descriptor_set(Some("UI Backdrop Blur Half Y"));
		context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(
				blur_half_downsample_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				blur_backdrop_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_half_downsample_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_half_source_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_full_x_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				blur_backdrop_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_full_x_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_full_scratch_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_full_y_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				blur_full_scratch_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_full_y_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_full_output_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_half_x_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				blur_half_source_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_half_x_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_half_scratch_image,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_half_y_descriptor_set,
				UI_BLUR_SOURCE_BINDING.slot(),
				blur_half_scratch_image,
				blur_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_half_y_descriptor_set,
				UI_BLUR_OUTPUT_BINDING.slot(),
				blur_half_output_image,
				ghi::Layouts::General,
			),
		]);
		let bypass_pass = crate::rendering::render_passes::blit::ImageBypassPass::new(render_pass_builder, source, output);

		Self {
			surface_revision: None,
			caches: UiGeometryCaches::default(),
			masks: UiMaskTable::default(),
			pipeline_manager,
			pipeline,
			clear_pipeline,
			primitive_buffer,
			mask_buffer,
			descriptor_set,
			image_sampler,
			image_textures: HashMap::new(),
			text,
			blur_downsample_pipeline,
			blur_filter_pipeline,
			blur_downsample_workgroup,
			blur_filter_workgroup,
			blur_sampler,
			blur_half_downsample_descriptor_set,
			blur_full_x_descriptor_set,
			blur_full_y_descriptor_set,
			blur_half_x_descriptor_set,
			blur_half_y_descriptor_set,
			blur_full_scratch: blur_full_scratch_image,
			blur_full_output: blur_full_output_image,
			blur_half_source: blur_half_source_image,
			blur_half_scratch: blur_half_scratch_image,
			blur_half_output: blur_half_output_image,
			blur_backdrop: blur_backdrop_image,
			blur_textures_bound: false,
			layer: main_attachment_image,
			composite_pipeline,
			composite_descriptor_set,
			blur_resolve_descriptor_set,
			region_workgroup,
			bypass_pass,
			data: UiDrawList::default(),
			render_revision: None,
			damage_base: None,
			damage_rects: Vec::new(),
			layer_revision: None,
			layer_extent: None,
			damage: Vec::new(),
			prepared: None,
			reported_capacity_limit: false,
			reported_dropped_glyphs: false,
			reported_image_limit: false,
			text_system: TextSystem::new(),
		}
	}

	/// Makes an image resident and returns the element of the ubershader's texture array it is bound to.
	///
	/// Returns `None` when every element holds an image the UI still shows.
	fn ensure_image_texture(&mut self, frame: &mut ghi::implementation::Frame, source: usize) -> Option<u32> {
		let image = &self.data.images[source];
		if !self.image_textures.contains_key(&image.image_id) {
			let free = (UI_FIRST_IMAGE_TEXTURE_SLOT..UI_TEXTURE_SLOTS)
				.find(|slot| self.image_textures.values().all(|texture| texture.slot != *slot));
			let texture = if let Some(slot) = free {
				let texture: ghi::BaseImageHandle = frame
					.build_image(
						ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image | ghi::Uses::TransferDestination)
							.name("UI Image")
							.extent(Extent::rectangle(image.source_width, image.source_height))
							.device_accesses(ghi::DeviceAccesses::HostToDevice),
					)
					.into();
				frame.write(&[ghi::DescriptorWrite::combined_image_sampler_array(
					self.descriptor_set,
					UI_TEXTURES_SLOT,
					texture,
					self.image_sampler,
					ghi::Layouts::Read,
					slot,
				)]);
				UiImageTexture {
					version: u64::MAX,
					extent: (0, 0),
					image: texture,
					slot,
				}
			} else {
				// Every element is taken. An image the UI no longer shows hands over its texture and its element.
				let shown = &self.data.images;
				let stale = self
					.image_textures
					.keys()
					.copied()
					.find(|id| shown.iter().all(|image| image.image_id != *id))?;
				let mut texture = self.image_textures.remove(&stale)?;
				texture.version = u64::MAX;
				texture
			};
			self.image_textures.insert(image.image_id, texture);
		}

		let texture = self.image_textures.get_mut(&image.image_id)?;
		if texture.version != image.version || texture.extent != (image.source_width, image.source_height) {
			frame.resize_image(texture.image, Extent::rectangle(image.source_width, image.source_height));
			let texture_slice = frame.get_texture_slice_mut(texture.image);
			texture_slice[..image.pixels.len()].copy_from_slice(&image.pixels);
			frame.sync_texture(texture.image);
			texture.version = image.version;
			texture.extent = (image.source_width, image.source_height);
		}

		Some(texture.slot)
	}

	/// Adopts submitted UI data; the caller can share one render across sinks.
	///
	/// Renders carry a revision; adopting the same revision again leaves the
	/// prepared frame valid, so unchanged UI never rebuilds geometry.
	pub fn update(&mut self, render: &engine::Render) {
		if self.render_revision == Some(render.revision()) {
			return;
		}
		if self.surface_revision != Some(render.surface_revision) {
			self.caches.retain_surfaces(&render.surface_ids);
			if let UiText::Atlas { atlas, .. } = &mut self.text {
				atlas.retain_surfaces(&render.surface_ids);
			}
			self.surface_revision = Some(render.surface_revision);
		}
		update_from_render(render, &mut self.data);
		self.render_revision = Some(render.revision());
		self.damage_rects.clear();
		self.damage_base = render.damage().map(|(base, rects)| {
			self.damage_rects.extend_from_slice(rects);
			base
		});
	}

	/// Computes this frame's pixel damage: engine damage when the layer holds its base revision, else everything.
	///
	/// Visible backdrop blurs are always damaged because they sample the scene rendered this frame.
	fn frame_damage(&mut self, extent: Extent) {
		self.damage.clear();
		let relative =
			self.layer_revision.is_some() && self.layer_revision == self.damage_base && self.layer_extent == Some(extent);
		if relative {
			pixel_damage(&self.damage_rects, self.data.layout_size, extent, &mut self.damage);
		} else if self.layer_revision != self.render_revision || self.layer_extent != Some(extent) {
			self.damage.push(UiPixelRegion::full(extent));
		}
		blur_footprints(&self.data, extent, &mut self.damage);
		merge_damage(&mut self.damage, extent);
	}

	/// Rebuilds primitives, uploads, and glyph residency for the adopted draw list at `extent`.
	// Keep every upload in one rebuild so the prepared frame always describes what the GPU buffers hold.
	#[allow(clippy::too_many_lines)]
	fn rebuild_prepared_frame(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		extent: Extent,
		frame_allocator: &bumpalo::Bump,
	) {
		let damage = Some(self.damage.as_slice());
		assert!(
			self.data.texts.is_empty() || (extent.width() > 0 && extent.height() > 0),
			"UI text geometry requires a non-zero viewport extent. The most likely cause is that text rendering ran before swapchain extent validation."
		);
		self.masks.clear();
		// Each renderer packs its own glyph data; both produce primitives for the same shader.
		let text = match &mut self.text {
			_ if self.data.texts.is_empty() => None,
			UiText::Slug {
				glyphs,
				curve_buffer,
				band_buffer,
			} => {
				let geometry = build_ui_slug_geometry_damaged(
					&self.data,
					extent,
					&mut self.text_system,
					glyphs,
					&mut self.masks,
					frame_allocator,
					damage,
				);
				glyphs.upload(frame, *curve_buffer, *band_buffer);
				Some(geometry)
			}
			UiText::Atlas { atlas, image } => {
				let geometry = build_ui_text_geometry_damaged(
					&self.data,
					extent,
					&mut self.text_system,
					atlas,
					&mut self.masks,
					frame_allocator,
					damage,
				);
				atlas.upload(frame, *image);
				Some(geometry)
			}
		};
		let mut primitives = build_ui_primitives(
			&self.data,
			extent,
			frame_allocator,
			Some(&mut self.caches),
			&mut self.masks,
			text.as_ref(),
			damage,
		);

		let mut images_without_texture = 0;
		for &(primitive, source) in &primitives.images {
			match self.ensure_image_texture(frame, source as usize) {
				Some(slot) => primitives.primitives[primitive as usize].data0 = slot,
				None => {
					// An empty quad rasterizes nothing, which keeps the records after it in place.
					primitives.primitives[primitive as usize].bounds = [0.0; 4];
					images_without_texture += 1;
				}
			}
		}

		warn_once(&mut self.reported_capacity_limit, primitives.truncated, || {
			format!(
				"UI primitive capacity exceeded. The most likely cause is that the UI needs more than {MAX_UI_PRIMITIVES} rectangles, glyphs, images, and curve pieces, or more than {MAX_UI_MASKS} clip masks, in a single frame."
			)
		});
		warn_once(&mut self.reported_dropped_glyphs, primitives.dropped_glyphs > 0, || {
			format!(
				"UI glyph capacity exceeded; {} glyphs were not drawn. The most likely cause is that one frame uses more distinct glyphs than the glyph atlas or the glyph curve buffers can hold.",
				primitives.dropped_glyphs
			)
		});
		warn_once(&mut self.reported_image_limit, images_without_texture > 0, || {
			format!(
				"UI image capacity exceeded; {images_without_texture} images were not drawn. The most likely cause is that the UI shows more than {} distinct images at once.",
				UI_TEXTURE_SLOTS - UI_FIRST_IMAGE_TEXTURE_SLOT
			)
		});

		frame.get_mut_buffer_slice(self.primitive_buffer)[..primitives.primitives.len()]
			.copy_from_slice(&primitives.primitives);
		frame.sync_buffer(self.primitive_buffer);
		let masks = self.masks.entries();
		frame.get_mut_buffer_slice(self.mask_buffer)[..masks.len()].copy_from_slice(masks);
		frame.sync_buffer(self.mask_buffer);

		if primitives.steps.iter().any(|step| matches!(step, UiStep::Blur(_))) {
			let half_extent = blur_half_extent(extent);
			frame.resize_image(self.blur_backdrop, extent);
			frame.resize_image(self.blur_full_scratch, extent);
			frame.resize_image(self.blur_full_output, extent);
			frame.resize_image(self.blur_half_source, half_extent);
			frame.resize_image(self.blur_half_scratch, half_extent);
			frame.resize_image(self.blur_half_output, half_extent);
			if !self.blur_textures_bound {
				frame.write(&[
					ghi::DescriptorWrite::combined_image_sampler(
						self.descriptor_set,
						UI_BLUR_FULL_SLOT,
						self.blur_full_output,
						self.blur_sampler,
						ghi::Layouts::Read,
					),
					ghi::DescriptorWrite::combined_image_sampler(
						self.descriptor_set,
						UI_BLUR_HALF_SLOT,
						self.blur_half_output,
						self.blur_sampler,
						ghi::Layouts::Read,
					),
				]);
				self.blur_textures_bound = true;
			}
		}

		// The previous frame is replaced below; retain its step allocation across rebuilds.
		let mut steps = self.prepared.take().map(|prepared| prepared.steps).unwrap_or_default();
		steps.clear();
		steps.extend_from_slice(&primitives.steps);

		self.prepared = Some(UiPreparedFrame {
			revision: self.render_revision,
			extent,
			glyph_generation: self.text.generation(),
			damage: self.damage.clone(),
			steps,
		});
	}
}

/// Warns when a limit is first exceeded, and again only after a frame that stayed within it.
fn warn_once(reported: &mut bool, exceeded: bool, message: impl FnOnce() -> String) {
	if exceeded && !*reported {
		log::warn!("{}", message());
	}
	*reported = exceeded;
}

impl RenderPass for UiRenderPass {
	fn name(&self) -> &'static str {
		"ui"
	}

	fn needs_frame(&mut self) -> bool {
		// The layer lags the adopted render until a recorded frame brings it up to date. An empty UI that never
		// drew anything has nothing to show.
		self.layer_revision != self.render_revision && !(self.data.is_empty() && self.layer_revision.is_none())
	}

	// Keep ordered UI step recording in one function so clears, blur barriers, and painter order cannot diverge.
	#[allow(clippy::too_many_lines)]
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pipeline = self.pipeline_manager.pipeline(self.pipeline)?;
		let clear_pipeline = self.pipeline_manager.pipeline(self.clear_pipeline)?;
		let blur_downsample_pipeline = self.pipeline_manager.pipeline(self.blur_downsample_pipeline)?;
		let blur_filter_pipeline = self.pipeline_manager.pipeline(self.blur_filter_pipeline)?;
		let composite_pipeline = self.pipeline_manager.pipeline(self.composite_pipeline)?;
		let extent = sink.extent();
		if self.data.is_empty() && self.layer_revision.is_none() {
			// Nothing was ever drawn into the layer, so the scene is the complete output.
			return self.bypass_pass.prepare(frame, sink, frame_allocator);
		}
		self.frame_damage(extent);
		let glyph_generation = self.text.generation();
		if !self.damage.is_empty()
			&& !self
				.prepared
				.as_ref()
				.is_some_and(|prepared| prepared.matches(self.render_revision, extent, glyph_generation, &self.damage))
		{
			self.rebuild_prepared_frame(frame, extent, frame_allocator);
		}
		let composite_descriptor_set = self.composite_descriptor_set;
		let region_workgroup = self.region_workgroup;
		let composite = move |command_buffer: &mut ghi::implementation::CommandBufferRecording| {
			let compute = command_buffer.bind_compute_pipeline(composite_pipeline);
			compute.bind_descriptor_sets(&[composite_descriptor_set]);
			compute.write_push_constant(0, UiRegionPush::from(UiPixelRegion::full(extent)));
			compute.dispatch(ghi::DispatchExtent::new(extent, region_workgroup));
		};
		if self.damage.is_empty() {
			// The layer already shows this revision; only the scene underneath may have changed.
			return Some(crate::rendering::render_pass::allocate_render_command(
				frame_allocator,
				move |command_buffer, _| composite(command_buffer),
			));
		}
		let prepared_steps = &self
			.prepared
			.as_ref()
			.expect("UI prepared frame must exist after a rebuild. The most likely cause is a rebuild that returned early.")
			.steps;

		let descriptor_set = self.descriptor_set;
		let blur_downsample_workgroup = self.blur_downsample_workgroup;
		let blur_filter_workgroup = self.blur_filter_workgroup;
		let blur_half_downsample_descriptor_set = self.blur_half_downsample_descriptor_set;
		let blur_full_x_descriptor_set = self.blur_full_x_descriptor_set;
		let blur_full_y_descriptor_set = self.blur_full_y_descriptor_set;
		let blur_half_x_descriptor_set = self.blur_half_x_descriptor_set;
		let blur_half_y_descriptor_set = self.blur_half_y_descriptor_set;
		let blur_resolve_descriptor_set = self.blur_resolve_descriptor_set;
		let layer = self.layer;
		let steps: &'a [UiStep] = frame_allocator.alloc_slice_copy(prepared_steps);
		let damage: &'a [UiPixelRegion] = frame_allocator.alloc_slice_copy(&self.damage);
		// The recorded command brings the layer up to the adopted revision at this extent.
		self.layer_revision = self.render_revision;
		self.layer_extent = Some(extent);

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("UI"),
					|command_buffer| {
						let push = |first: u32| UiDrawPush {
							viewport: [extent.width().max(1) as f32, extent.height().max(1) as f32],
							first,
							padding: 0,
						};
						// Damaged pixels start transparent; everything intersecting them is redrawn in painter order.
						// Damage that covers the viewport is the attachment's own clear, which also skips loading the layer.
						let redraws_everything = damage == [UiPixelRegion::full(extent)];

						for (index, step) in steps.iter().enumerate() {
							let (first, count) = match step {
								UiStep::Draw { first, count } => (*first, *count),
								UiStep::Blur(blur) => {
									command_buffer.region(
										|label| label.write_str("UI Backdrop Blur"),
										|command_buffer| {
											// The blur samples the scene under the UI drawn so far, resolved for its footprint only.
											let compute = command_buffer.bind_compute_pipeline(composite_pipeline);
											compute.bind_descriptor_sets(&[blur_resolve_descriptor_set]);
											compute.write_push_constant(0, UiRegionPush::from(blur.backdrop));
											compute.dispatch(ghi::DispatchExtent::new(blur.backdrop.extent, region_workgroup));

											if blur_uses_full_resolution(blur.resolution_mix) {
												let compute = command_buffer.bind_compute_pipeline(blur_filter_pipeline);
												compute.bind_descriptor_sets(&[blur_full_x_descriptor_set]);
												compute.write_push_constant(
													0,
													blur.full_kernel.push([1.0, 0.0], blur.full_regions.horizontal),
												);
												compute.dispatch(ghi::DispatchExtent::new(
													blur.full_regions.horizontal.extent,
													blur_filter_workgroup,
												));

												let compute = command_buffer.bind_compute_pipeline(blur_filter_pipeline);
												compute.bind_descriptor_sets(&[blur_full_y_descriptor_set]);
												compute.write_push_constant(
													0,
													blur.full_kernel.push([0.0, 1.0], blur.full_regions.vertical),
												);
												compute.dispatch(ghi::DispatchExtent::new(
													blur.full_regions.vertical.extent,
													blur_filter_workgroup,
												));
											}

											if blur_uses_half_resolution(blur.resolution_mix) {
												let compute = command_buffer.bind_compute_pipeline(blur_downsample_pipeline);
												compute.bind_descriptor_sets(&[blur_half_downsample_descriptor_set]);
												compute
													.write_push_constant(0, UiRegionPush::from(blur.half_regions.downsample));
												compute.dispatch(ghi::DispatchExtent::new(
													blur.half_regions.downsample.extent,
													blur_downsample_workgroup,
												));

												let compute = command_buffer.bind_compute_pipeline(blur_filter_pipeline);
												compute.bind_descriptor_sets(&[blur_half_x_descriptor_set]);
												compute.write_push_constant(
													0,
													blur.half_kernel.push([1.0, 0.0], blur.half_regions.filter.horizontal),
												);
												compute.dispatch(ghi::DispatchExtent::new(
													blur.half_regions.filter.horizontal.extent,
													blur_filter_workgroup,
												));

												let compute = command_buffer.bind_compute_pipeline(blur_filter_pipeline);
												compute.bind_descriptor_sets(&[blur_half_y_descriptor_set]);
												compute.write_push_constant(
													0,
													blur.half_kernel.push([0.0, 1.0], blur.half_regions.filter.vertical),
												);
												compute.dispatch(ghi::DispatchExtent::new(
													blur.half_regions.filter.vertical.extent,
													blur_filter_workgroup,
												));
											}
										},
									);
									continue;
								}
							};

							// Each draw is its own pass because the blur before it reads the layer from compute.
							// The first one always runs, since it clears the damage.
							let clears = index == 0;
							let attachments = [ghi::AttachmentInformation::new(
								layer,
								ghi::Layouts::RenderTarget,
								if clears && redraws_everything {
									ghi::ClearValue::Color(RGBA::transparent())
								} else {
									ghi::ClearValue::None
								},
								!(clears && redraws_everything),
								true,
							)];
							let render_pass = command_buffer.start_render_pass(extent, &attachments);
							if clears && !redraws_everything {
								let clear = render_pass.bind_raster_pipeline(clear_pipeline);
								clear.bind_descriptor_sets(&[descriptor_set]);
								clear.write_push_constant(0, push(0));
								for region in damage {
									clear.set_scissor(region.origin, region.extent);
									clear.draw(UI_VERTICES_PER_PRIMITIVE, 1, 0, 0);
								}
							}
							if count > 0 {
								let draw = render_pass.bind_raster_pipeline(pipeline);
								draw.bind_descriptor_sets(&[descriptor_set]);
								draw.write_push_constant(0, push(first));
								for region in damage {
									draw.set_scissor(region.origin, region.extent);
									draw.draw(count * UI_VERTICES_PER_PRIMITIVE, 1, 0, 0);
								}
							}
							render_pass.end_render_pass();
						}
					},
				);
				// The scene is composited under the layer only after every damaged region has been redrawn.
				composite(command_buffer);
			},
		))
	}

	crate::rendering::render_pass::forward_to_inner_pass!(bypass = bypass_pass);
}

#[cfg(test)]
mod tests {
	use std::mem::{align_of, offset_of, size_of};

	use besl::vm::{
		Buffer, DescriptorBindings, ExecutableProgram, Texture, Value, builtin_position_slot, builtin_vertex_index_slot,
		input_slot, output_slot,
	};
	use utils::{Extent, RGBA};

	use super::Rotation;
	use super::{
		CURVE_QUADRATIC_TOLERANCE_PIXELS, DrawClip, DrawClipMask, MAX_CURVE_PIECES, MAX_UI_PRIMITIVES, UI_ATLAS_TEXTURE_SLOT,
		UI_BLUR_FULL_SLOT, UI_BLUR_GAUSSIAN_PAIR_COUNT, UI_BLUR_GAUSSIAN_SUPPORT, UI_BLUR_HALF_DOWNSCALE, UI_BLUR_HALF_SLOT,
		UI_CURVE_CAP_END, UI_CURVE_CAP_START, UI_GLYPH_BAND_CAPACITY, UI_GLYPH_CURVE_CAPACITY, UI_KIND_ATLAS_GLYPH,
		UI_KIND_BLUR, UI_KIND_CURVE, UI_KIND_IMAGE, UI_KIND_RECT, UI_TEXTURES_SLOT, UiBlurDrawElement, UiBlurFilterPush,
		UiBlurKernel, UiClipMaskEntry, UiCurveDrawElement, UiDrawElement, UiDrawList, UiGlyphCurves, UiImageDrawElement,
		UiMaskTable, UiPixelRegion, UiPreparedFrame, UiPrimitive, UiPrimitives, UiStep, UiTextDrawElement,
		blur_composite_region, blur_full_dispatch_regions, blur_half_dispatch_regions, blur_half_extent, blur_half_sigma,
		blur_resolution_mix, blur_sigma, blur_uses_full_resolution, blur_uses_half_resolution, build_ui_primitives_uncached,
		build_ui_slug_geometry, clear_primitive, curve_piece_count, should_draw_image, should_rasterize_text,
		update_from_render,
	};
	use crate::rendering::{
		render_pass::simple_compute,
		shader_vm_test::{assert_rgba_close, compile as compile_shader_vm, empty_image, rgba, run_at, texture_2d},
	};
	use crate::ui::{
		Container, Text,
		components::{
			curve::{CurvePoint, CurveSegment},
			image::Image,
		},
		flow::Size,
		font::TextSystem,
		layout::{
			context::{Context, ElementContext},
			engine::Engine,
		},
		style::{ConcreteLayer, ConcreteStyle, LayerKind},
	};

	const UI_BLUR_DOWNSAMPLE_BESL: &str = include_str!("../../assets/rendering/ui/backdrop-blur-downsample.besl");
	const UI_BLUR_FILTER_BESL: &str = include_str!("../../assets/rendering/ui/backdrop-blur-filter.besl");
	const UI_VERTEX_BESL: &str = include_str!("../../assets/rendering/ui/ui-vertex.besl");
	const UI_FRAGMENT_BESL: &str = include_str!("../../assets/rendering/ui/ui-fragment.besl");

	fn assert_vec2_close(actual: [f32; 2], expected: [f32; 2]) {
		assert!((actual[0] - expected[0]).abs() < 0.0001);
		assert!((actual[1] - expected[1]).abs() < 0.0001);
	}

	fn assert_vec4_close(actual: [f32; 4], expected: [f32; 4]) {
		for (actual, expected) in actual.into_iter().zip(expected) {
			assert!((actual - expected).abs() < 0.0001, "Expected {expected}, found {actual}");
		}
	}

	// Compiles one checked-in UI blur shader through the same shared scope used
	// by production standalone compute shaders.
	fn compile_ui_blur_shader(source: &str) -> ExecutableProgram {
		compile_shader_vm(simple_compute::compile_test_program(source))
	}

	// Links one checked-in raster shader through the production BESL frontend.
	fn ui_raster_program(source: &str, shader_name: &str) -> besl::NodeReference {
		let program = besl::compile_to_besl(source, None).unwrap_or_else(|error| {
			panic!(
				"Failed to link {shader_name}: {error:?}. The most likely cause is invalid syntax in the checked-in BESL asset."
			)
		});
		program.get_main().unwrap_or_else(|| {
			panic!(
				"Missing {shader_name} entry point. The most likely cause is that the checked-in BESL asset has no `main` function."
			)
		})
	}

	/// The `UiVaryings` struct is what the vertex stage hands one fragment.
	#[derive(Debug, Clone, Copy, Default)]
	struct UiVaryings {
		primitive: u32,
		pixel_position: [f32; 2],
		uv: [f32; 2],
		curve_from: [f32; 4],
		curve_to: [f32; 2],
		/// Where the quad's turn drew the pixel. `None` is an unrotated quad, drawn at `pixel_position`.
		screen_position: Option<[f32; 2]>,
	}

	fn vm_masks(executable: &ExecutableProgram, masks: &[UiClipMaskEntry]) -> Buffer {
		let mut mask_buffer = vm_array(executable, 1, masks.len());
		for (index, mask) in masks.iter().enumerate() {
			for (name, value) in [
				("clip", mask.clip),
				("rect", mask.rect),
				("edges", mask.edges),
				("corner", mask.corner),
				("rotation", mask.rotation),
			] {
				mask_buffer
					.write_array_member(index, name, Value::Vec4F(value))
					.expect("Failed to write a clip mask. The most likely cause is a changed clip mask layout.");
			}
		}
		mask_buffer
	}

	fn vm_array(executable: &ExecutableProgram, slot: u32, count: usize) -> Buffer {
		Buffer::new_array(
			executable
				.buffer_layout(besl::vm::ResourceSlot::new(slot))
				.expect("Missing UI shader buffer layout. The most likely cause is a changed shader binding.")
				.clone(),
			count.max(1),
		)
		.expect("Failed to create a UI shader VM buffer. The most likely cause is an invalid element count.")
	}

	// Mirrors primitive records into a VM buffer the way the render pass uploads them.
	fn vm_primitives(executable: &ExecutableProgram, primitives: &[UiPrimitive]) -> Buffer {
		let mut buffer = vm_array(executable, 0, primitives.len());
		for (index, primitive) in primitives.iter().enumerate() {
			for (name, value) in [
				("bounds", Value::Vec4F(primitive.bounds)),
				("color", Value::Vec4F(primitive.color)),
				("a", Value::Vec4F(primitive.a)),
				("b", Value::Vec4F(primitive.b)),
				("kind", Value::U32(primitive.kind)),
				("mask", Value::U32(primitive.mask)),
				("data0", Value::U32(primitive.data0)),
				("data1", Value::U32(primitive.data1)),
			] {
				buffer
					.write_array_member(index, name, value)
					.expect("Failed to write a primitive. The most likely cause is a changed primitive record layout.");
			}
		}
		buffer
	}

	/// The `UiFragmentVm` struct runs the production UI fragment shader on the records of one frame.
	struct UiFragmentVm {
		executable: ExecutableProgram,
		primitives: Buffer,
		masks: Buffer,
		glyph_curves: Buffer,
		glyph_bands: Buffer,
	}

	impl UiFragmentVm {
		fn new(primitives: &[UiPrimitive], masks: &[UiClipMaskEntry], glyphs: Option<&UiGlyphCurves>) -> Self {
			let executable = compile_shader_vm(ui_raster_program(UI_FRAGMENT_BESL, "UI fragment shader"));
			let primitives = vm_primitives(&executable, primitives);
			let mask_buffer = vm_masks(&executable, masks);
			let (curves, bands) = glyphs.map_or((&[][..], &[][..]), |glyphs| (glyphs.curves(), glyphs.bands()));
			let mut glyph_curves = vm_array(&executable, 2, curves.len());
			for (index, points) in curves.iter().enumerate() {
				glyph_curves
					.write_array_member(index, "points", Value::Vec4F(*points))
					.expect("Failed to write a glyph curve. The most likely cause is a changed curve element layout.");
			}
			let mut glyph_bands = vm_array(&executable, 3, bands.len());
			for (index, value) in bands.iter().enumerate() {
				glyph_bands
					.write_array_member(index, "value", Value::U32(*value))
					.expect("Failed to write a glyph band. The most likely cause is a changed band element layout.");
			}
			Self {
				executable,
				primitives,
				masks: mask_buffer,
				glyph_curves,
				glyph_bands,
			}
		}

		/// Shades one fragment. `textures` pairs shader bindings with the textures bound to them.
		fn run(&mut self, varyings: UiVaryings, textures: &mut [(u32, &mut Texture)]) -> [f32; 4] {
			// Interface fields take their locations in name order.
			let mut inputs = [
				("_besl_interface_curve_from", Value::Vec4F(varyings.curve_from)),
				("_besl_interface_curve_to", Value::Vec2F(varyings.curve_to)),
				("_besl_interface_pixel_position", Value::Vec2F(varyings.pixel_position)),
				("_besl_interface_primitive", Value::U32(varyings.primitive)),
				(
					"_besl_interface_screen_position",
					Value::Vec2F(varyings.screen_position.unwrap_or(varyings.pixel_position)),
				),
				("_besl_interface_uv", Value::Vec2F(varyings.uv)),
			]
			.into_iter()
			.enumerate()
			.map(|(location, (name, value))| {
				let mut input = Buffer::new(
					self.executable
						.input_layout(location as u8)
						.expect("Missing UI fragment input layout. The most likely cause is a changed shader interface.")
						.clone(),
				);
				input
					.write(name, value)
					.expect("Failed to seed a UI fragment VM input. The most likely cause is an interface type mismatch.");
				input
			})
			.collect::<Vec<_>>();
			let mut output = Buffer::new(
				self.executable
					.output_layout(0)
					.expect("Missing UI fragment output layout. The most likely cause is an unresolved shader output.")
					.clone(),
			);
			{
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_buffer(besl::vm::ResourceSlot::new(0), &mut self.primitives);
				descriptors.bind_buffer(besl::vm::ResourceSlot::new(1), &mut self.masks);
				descriptors.bind_buffer(besl::vm::ResourceSlot::new(2), &mut self.glyph_curves);
				descriptors.bind_buffer(besl::vm::ResourceSlot::new(3), &mut self.glyph_bands);
				for (slot, texture) in textures.iter_mut() {
					descriptors.bind_texture(besl::vm::ResourceSlot::new(*slot), texture);
				}
				for (location, input) in inputs.iter_mut().enumerate() {
					descriptors.bind_buffer(input_slot(location as u8), input);
				}
				descriptors.bind_buffer(output_slot(0), &mut output);
				self.executable
					.run_main(&mut descriptors)
					.expect("Failed to execute the UI fragment shader. The most likely cause is incomplete BESL VM support.");
			}
			match output
				.read("_besl_output_color_attachment")
				.expect("Failed to read UI fragment output. The most likely cause is an interface layout mismatch.")
			{
				Value::Vec4F(color) => color,
				value => panic!(
					"Invalid UI fragment output type `{value:?}`. The most likely cause is a BESL VM interface type mismatch."
				),
			}
		}
	}

	/// The `UiVertexVm` struct runs the production UI vertex shader, which pulls its quads from the primitive records.
	struct UiVertexVm {
		executable: ExecutableProgram,
		primitives: Buffer,
		masks: Buffer,
		viewport: [f32; 2],
	}

	impl UiVertexVm {
		fn new(primitives: &[UiPrimitive], masks: &[UiClipMaskEntry], viewport: [f32; 2]) -> Self {
			let executable = compile_shader_vm(ui_raster_program(UI_VERTEX_BESL, "UI vertex shader"));
			let primitives = vm_primitives(&executable, primitives);
			let masks = vm_masks(&executable, masks);
			Self {
				executable,
				primitives,
				masks,
				viewport,
			}
		}

		/// Returns the clip space position and the varyings of one vertex of a draw that starts at record `first`.
		fn run(&mut self, first: u32, vertex_index: u32) -> ([f32; 4], UiVaryings) {
			let mut push_constant = Buffer::new(
				self.executable
					.push_constant_layout()
					.expect("Missing UI draw push constants. The most likely cause is a changed vertex shader interface.")
					.clone(),
			);
			push_constant.write("viewport", Value::Vec2F(self.viewport)).unwrap();
			push_constant.write("first", Value::U32(first)).unwrap();
			let mut vertex = Buffer::new(self.executable.builtin_vertex_index_layout().unwrap().clone());
			vertex.write("vertex_index", Value::U32(vertex_index)).unwrap();
			let mut position = Buffer::new(self.executable.builtin_position_layout().unwrap().clone());
			let mut outputs: Vec<_> = (0..6)
				.map(|index| Buffer::new(self.executable.output_layout(index).unwrap().clone()))
				.collect();
			{
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_buffer(besl::vm::ResourceSlot::new(0), &mut self.primitives);
				descriptors.bind_buffer(besl::vm::ResourceSlot::new(1), &mut self.masks);
				descriptors.bind_push_constant(&mut push_constant);
				descriptors.bind_buffer(builtin_vertex_index_slot(), &mut vertex);
				descriptors.bind_buffer(builtin_position_slot(), &mut position);
				for (index, output) in outputs.iter_mut().enumerate() {
					descriptors.bind_buffer(output_slot(index as u8), output);
				}
				self.executable
					.run_main(&mut descriptors)
					.expect("Failed to execute the UI vertex shader. The most likely cause is incomplete BESL VM support.");
			}
			let vec2 = |buffer: &Buffer, name: &str| match buffer.read(name) {
				Ok(Value::Vec2F(value)) => value,
				value => panic!("Invalid UI vertex output `{name}`: {value:?}."),
			};
			let vec4 = |buffer: &Buffer, name: &str| match buffer.read(name) {
				Ok(Value::Vec4F(value)) => value,
				value => panic!("Invalid UI vertex output `{name}`: {value:?}."),
			};
			let Ok(Value::U32(primitive)) = outputs[3].read("_besl_interface_primitive") else {
				panic!("Invalid UI vertex primitive output. The most likely cause is a changed vertex shader interface.");
			};
			(
				vec4(&position, "_besl_interface_position"),
				// Interface fields take their locations in name order.
				UiVaryings {
					primitive,
					pixel_position: vec2(&outputs[2], "_besl_interface_pixel_position"),
					uv: vec2(&outputs[5], "_besl_interface_uv"),
					screen_position: Some(vec2(&outputs[4], "_besl_interface_screen_position")),
					curve_from: vec4(&outputs[0], "_besl_interface_curve_from"),
					curve_to: vec2(&outputs[1], "_besl_interface_curve_to"),
				},
			)
		}

		/// Returns the corners of one primitive's quad, clockwise from the top left, and its flat varyings.
		fn quad(&mut self, primitive: u32) -> ([[f32; 2]; 4], UiVaryings) {
			let corners = [0, 1, 2, 4].map(|vertex| self.run(primitive, vertex).1.pixel_position);
			(corners, self.run(primitive, 0).1)
		}
	}

	/// Shades every pixel whose center is inside a primitive's quad, as the rasterizer would.
	fn rasterize_primitive(
		vertex: &mut UiVertexVm,
		fragment: &mut UiFragmentVm,
		primitive: u32,
		mut shade: impl FnMut([i32; 2], [f32; 4]),
	) {
		let (corners, varyings) = vertex.quad(primitive);
		let min = |axis: usize| corners.iter().map(|corner| corner[axis]).fold(f32::INFINITY, f32::min);
		let max = |axis: usize| corners.iter().map(|corner| corner[axis]).fold(f32::NEG_INFINITY, f32::max);
		for y in min(1).floor() as i32..max(1).ceil() as i32 {
			for x in min(0).floor() as i32..max(0).ceil() as i32 {
				let center = [x as f32 + 0.5, y as f32 + 0.5];
				// The quad is convex and clockwise, so its inside is to the right of every edge.
				let inside = (0..4).all(|edge| {
					let (from, to) = (corners[edge], corners[(edge + 1) % 4]);
					(to[0] - from[0]) * (center[1] - from[1]) - (to[1] - from[1]) * (center[0] - from[0]) >= 0.0
				});
				if inside {
					let varyings = UiVaryings {
						pixel_position: center,
						screen_position: None,
						..varyings
					};
					shade([x, y], fragment.run(varyings, &mut []));
				}
			}
		}
	}

	// Initializes the shared origin/extent contract used by regional compute stages.
	fn blur_region_push_constant(executable: &ExecutableProgram, origin: [u32; 2], extent: [u32; 2]) -> Buffer {
		let mut push_constant = Buffer::new(
			executable
				.push_constant_layout()
				.expect("Missing blur region push constants. The most likely cause is a changed production shader interface.")
				.clone(),
		);
		push_constant
			.write("origin", Value::Vec2U(origin))
			.expect("Failed to initialize the blur region origin. The most likely cause is a changed push constant type.");
		push_constant
			.write("extent", Value::Vec2U(extent))
			.expect("Failed to initialize the blur region extent. The most likely cause is a changed push constant type.");
		push_constant
	}

	// Mirrors the aligned host record through named VM fields so production
	// shader tests validate both the coefficients and the reflected interface.
	fn blur_filter_push_constant(executable: &ExecutableProgram, push: UiBlurFilterPush) -> Buffer {
		let mut push_constant = Buffer::new(
			executable
				.push_constant_layout()
				.expect("Missing blur filter push constants. The most likely cause is a changed production shader interface.")
				.clone(),
		);
		for (name, value) in [
			("filter_data", Value::Vec4F(push.filter_data)),
			("origin", Value::Vec2U(push.origin)),
			("extent", Value::Vec2U(push.extent)),
			("pair_weights_0_3", Value::Vec4F(push.pair_weights_0_3)),
			("pair_weights_4_7", Value::Vec4F(push.pair_weights_4_7)),
			("pair_weights_8_10", Value::Vec4F(push.pair_weights_8_10_pad)),
			("pair_offsets_0_3", Value::Vec4F(push.pair_offsets_0_3)),
			("pair_offsets_4_7", Value::Vec4F(push.pair_offsets_4_7)),
			("pair_offsets_8_10", Value::Vec4F(push.pair_offsets_8_10_pad)),
		] {
			push_constant.write(name, value).unwrap_or_else(|error| {
				panic!(
					"Failed to initialize blur filter field `{name}`: {error}. The most likely cause is a changed push constant type."
				)
			});
		}
		push_constant
	}

	// Reconstructs the integer Gaussian taps represented by the bilinear pairs
	// so tests can compare the actual discrete variance with the requested one.
	fn blur_kernel_variance(kernel: UiBlurKernel) -> f32 {
		let mut second_moment = 0.0;
		for pair_index in 0..UI_BLUR_GAUSSIAN_PAIR_COUNT {
			let first = (pair_index * 2 + 1) as f32;
			let weight = kernel.pair_weights[pair_index];
			let offset = kernel.pair_offsets[pair_index];
			let first_weight = weight * (first + 1.0 - offset);
			let second_weight = weight * (offset - first);
			second_moment += 2.0 * (first_weight * first * first + second_weight * (first + 1.0) * (first + 1.0));
		}
		second_moment
	}

	// Executes the production composite shader with a full-coverage rectangle.
	// A square backdrop blur primitive under one feather mask, which is all the composite tests vary.
	fn blur_composite_vm() -> UiFragmentVm {
		UiFragmentVm::new(
			&[UiPrimitive {
				bounds: [0.0, 0.0, 8.0, 4.0],
				a: [0.0, 0.0, 8.0, 4.0],
				b: [0.0, 2.0, 0.0, 0.0],
				kind: UI_KIND_BLUR,
				mask: 1,
				..UiPrimitive::default()
			}],
			&[
				UiClipMaskEntry::NONE,
				UiClipMaskEntry {
					rect: [0.0, 0.0, 8.0, 4.0],
					..UiClipMaskEntry::NONE
				},
			],
			None,
		)
	}

	fn run_blur_composite_vm(
		full_texels: &[[f32; 4]],
		full_extent: [u32; 2],
		half_texels: &[[f32; 4]],
		half_extent: [u32; 2],
		pixel_position: [f32; 2],
		resolution_mix: f32,
		feather_edges: [f32; 4],
	) -> [f32; 4] {
		let mut full_blurred = texture_2d(full_extent[0], full_extent[1], full_texels);
		let mut half_blurred = texture_2d(half_extent[0], half_extent[1], half_texels);
		run_blur_composite_textures_vm(
			&mut blur_composite_vm(),
			&mut full_blurred,
			&mut half_blurred,
			pixel_position,
			resolution_mix,
			feather_edges,
		)
	}

	// Executes one backdrop blur fragment against reusable textures and a precompiled shader,
	// which keeps production-chain parameter sweeps fast enough for unit tests.
	fn run_blur_composite_textures_vm(
		composite: &mut UiFragmentVm,
		full_blurred: &mut Texture,
		half_blurred: &mut Texture,
		pixel_position: [f32; 2],
		resolution_mix: f32,
		feather_edges: [f32; 4],
	) -> [f32; 4] {
		composite
			.primitives
			.write_array_member(0, "b", Value::Vec4F([0.0, 2.0, 0.0, resolution_mix]))
			.expect("Failed to set the blur resolution mix. The most likely cause is a changed primitive record layout.");
		composite
			.masks
			.write_array_member(1, "edges", Value::Vec4F(feather_edges))
			.expect("Failed to set the feather edges. The most likely cause is a changed clip mask layout.");
		composite.run(
			UiVaryings {
				pixel_position,
				..UiVaryings::default()
			},
			&mut [
				(UI_BLUR_FULL_SLOT.index(), full_blurred),
				(UI_BLUR_HALF_SLOT.index(), half_blurred),
			],
		)
	}

	// Executes one regional production downsample dispatch into a caller-owned
	// image so tests can seed untouched texels with stale sentinels.
	fn run_blur_downsample_region_vm(
		executable: &ExecutableProgram,
		source: &mut Texture,
		result: &mut Texture,
		region: UiPixelRegion,
	) {
		let mut push_constant = blur_region_push_constant(executable, region.origin, region.push_extent());
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(besl::vm::ResourceSlot::new(0), source);
		descriptors.bind_image(besl::vm::ResourceSlot::new(1), result);
		descriptors.bind_push_constant(&mut push_constant);
		for y in 0..region.extent.height() {
			for x in 0..region.extent.width() {
				run_at(executable, &mut descriptors, [x, y]);
			}
		}
	}

	// Executes one regional production Gaussian dispatch using the same packed
	// coefficients and local-thread convention as command recording.
	fn run_blur_filter_region_vm(
		executable: &ExecutableProgram,
		source: &mut Texture,
		result: &mut Texture,
		kernel: UiBlurKernel,
		direction: [f32; 2],
		region: UiPixelRegion,
	) {
		let mut push_constant = blur_filter_push_constant(executable, kernel.push(direction, region));
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(besl::vm::ResourceSlot::new(0), source);
		descriptors.bind_image(besl::vm::ResourceSlot::new(1), result);
		descriptors.bind_push_constant(&mut push_constant);
		for y in 0..region.extent.height() {
			for x in 0..region.extent.width() {
				run_at(executable, &mut descriptors, [x, y]);
			}
		}
	}

	fn full_blur_region(extent: Extent) -> UiPixelRegion {
		UiPixelRegion { origin: [0, 0], extent }
	}

	// Runs every production BESL stage selected for one radius and returns the
	// composited center scanline. Radius zero follows production's skipped path.
	fn run_adaptive_blur_scanline_vm(
		downsample: &ExecutableProgram,
		filter: &ExecutableProgram,
		composite: &mut UiFragmentVm,
		texels: &[[f32; 4]],
		extent: Extent,
		radius: f32,
		display_scale: f32,
	) -> Vec<[f32; 4]> {
		let width = extent.width();
		let height = extent.height();
		if radius <= 0.0 {
			let row = height / 2;
			return texels[(row * width) as usize..((row + 1) * width) as usize].to_vec();
		}

		let sigma = blur_sigma((radius * display_scale).clamp(0.0, 64.0));
		let resolution_mix = blur_resolution_mix(sigma);
		let full_region = full_blur_region(extent);
		let half_extent = blur_half_extent(extent);
		let half_region = full_blur_region(half_extent);
		let mut source = texture_2d(width, height, texels);
		let mut full_output = empty_image(width, height);
		if blur_uses_full_resolution(resolution_mix) {
			let mut horizontal = empty_image(width, height);
			run_blur_filter_region_vm(
				filter,
				&mut source,
				&mut horizontal,
				UiBlurKernel::gaussian(sigma),
				[1.0, 0.0],
				full_region,
			);
			run_blur_filter_region_vm(
				filter,
				&mut horizontal,
				&mut full_output,
				UiBlurKernel::gaussian(sigma),
				[0.0, 1.0],
				full_region,
			);
		}

		let mut half_output = empty_image(half_extent.width(), half_extent.height());
		if blur_uses_half_resolution(resolution_mix) {
			let mut half_source = empty_image(half_extent.width(), half_extent.height());
			run_blur_downsample_region_vm(downsample, &mut source, &mut half_source, half_region);
			let mut horizontal = empty_image(half_extent.width(), half_extent.height());
			let half_kernel = UiBlurKernel::gaussian(blur_half_sigma(sigma));
			run_blur_filter_region_vm(
				filter,
				&mut half_source,
				&mut horizontal,
				half_kernel,
				[1.0, 0.0],
				half_region,
			);
			run_blur_filter_region_vm(
				filter,
				&mut horizontal,
				&mut half_output,
				half_kernel,
				[0.0, 1.0],
				half_region,
			);
		}

		let row = height / 2;
		(0..width)
			.map(|x| {
				run_blur_composite_textures_vm(
					composite,
					&mut full_output,
					&mut half_output,
					[x as f32 + 0.5, row as f32 + 0.5],
					resolution_mix,
					[0.0; 4],
				)
			})
			.collect()
	}

	#[derive(Clone, Copy)]
	enum BlurChainPattern {
		Impulse,
		ThinLine,
		Checkerboard,
		Constant,
	}

	// Builds bounded semantic inputs that expose ringing, energy drift, and
	// failure to preserve constant colors without requiring a full-size frame.
	fn blur_chain_fixture(pattern: BlurChainPattern, extent: Extent) -> Vec<[f32; 4]> {
		let mut texels = vec![[0.0, 0.0, 0.0, 1.0]; (extent.width() * extent.height()) as usize];
		for y in 0..extent.height() {
			for x in 0..extent.width() {
				let color = match pattern {
					BlurChainPattern::Impulse if x == extent.width() / 2 && y == extent.height() / 2 => [1.0; 4],
					BlurChainPattern::ThinLine if x == extent.width() / 2 => [1.0; 4],
					BlurChainPattern::Checkerboard if (x + y) % 2 == 0 => [1.0; 4],
					BlurChainPattern::Constant => [0.25, 0.5, 0.75, 1.0],
					_ => [0.0, 0.0, 0.0, 1.0],
				};
				texels[(y * extent.width() + x) as usize] = color;
			}
		}
		texels
	}

	/// Verifies the half-resolution prefilter uses the positive binomial marginal and guards extra lanes.
	#[test]
	fn backdrop_blur_downsample_besl_vm_uses_binomial_prefilter() {
		let executable = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let mut texels = [[0.0; 4]; 6];
		texels[1] = [1.0, 0.0, 0.0, 0.0];
		texels[2] = [0.0, 1.0, 0.0, 0.0];
		texels[3] = [0.0, 0.0, 1.0, 0.0];
		texels[4] = [0.0, 0.0, 0.0, 1.0];
		let mut source = texture_2d(6, 1, &texels);
		let mut result = empty_image(3, 1);
		let mut push_constant = blur_region_push_constant(&executable, [1, 0], [1, 1]);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(besl::vm::ResourceSlot::new(0), &mut source);
		descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut result);
		descriptors.bind_push_constant(&mut push_constant);
		run_at(&executable, &mut descriptors, [0, 0]);
		run_at(&executable, &mut descriptors, [1, 0]);
		drop(descriptors);

		assert_rgba_close(rgba(&result, [1, 0]), [0.125, 0.375, 0.375, 0.125], 1e-6);
		assert_rgba_close(rgba(&result, [0, 0]), [0.0; 4], 1e-6);
		assert_rgba_close(rgba(&result, [2, 0]), [0.0; 4], 1e-6);
	}

	#[test]
	fn backdrop_blur_filter_push_layout_matches_the_production_shader() {
		assert_eq!(size_of::<UiBlurFilterPush>(), 128);
		assert_eq!(align_of::<UiBlurFilterPush>(), 16);
		assert_eq!(offset_of!(UiBlurFilterPush, filter_data), 0);
		assert_eq!(offset_of!(UiBlurFilterPush, origin), 16);
		assert_eq!(offset_of!(UiBlurFilterPush, extent), 24);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_weights_0_3), 32);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_weights_4_7), 48);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_weights_8_10_pad), 64);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_offsets_0_3), 80);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_offsets_4_7), 96);
		assert_eq!(offset_of!(UiBlurFilterPush, pair_offsets_8_10_pad), 112);

		let executable = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let layout = executable
			.push_constant_layout()
			.expect("Missing production blur push constants. The most likely cause is a changed filter interface.");

		assert_eq!(layout.size(), 128);
		for (name, expected_offset) in [
			("filter_data", 0),
			("origin", 16),
			("extent", 24),
			("pair_weights_0_3", 32),
			("pair_weights_4_7", 48),
			("pair_weights_8_10", 64),
			("pair_offsets_0_3", 80),
			("pair_offsets_4_7", 96),
			("pair_offsets_8_10", 112),
		] {
			let actual = layout
				.members()
				.iter()
				.find(|member| member.name() == name)
				.unwrap_or_else(|| panic!("Missing reflected blur field `{name}`"))
				.offset();

			assert_eq!(actual, expected_offset, "Unexpected reflected offset for `{name}`");
		}
	}

	#[test]
	fn backdrop_blur_gaussian_coefficients_are_normalized_and_preserve_variance() {
		let smallest_test_sigma = blur_sigma(0.25);
		let largest_half_sigma = blur_half_sigma(blur_sigma(64.0));
		for sigma in [0.0, smallest_test_sigma, 4.0, 5.0, 6.0, largest_half_sigma] {
			let kernel = UiBlurKernel::gaussian(sigma);
			let energy = kernel.center_weight + 2.0 * kernel.pair_weights.iter().sum::<f32>();

			assert!(
				(energy - 1.0).abs() <= 2e-6,
				"Gaussian energy drifted to {energy} at sigma {sigma}"
			);
			assert!(kernel.center_weight.is_finite() && kernel.center_weight >= 0.0);

			let mut second_moment = 0.0f32;
			for pair_index in 0..UI_BLUR_GAUSSIAN_PAIR_COUNT {
				let first = (pair_index * 2 + 1) as f32;
				let weight = kernel.pair_weights[pair_index];
				let offset = kernel.pair_offsets[pair_index];

				assert!(weight.is_finite() && weight >= 0.0);
				assert!(offset.is_finite() && (first..=first + 1.0).contains(&offset));
				let first_weight = weight * (first + 1.0 - offset);
				let second_weight = weight * (offset - first);
				second_moment += 2.0 * (first_weight * first * first + second_weight * (first + 1.0) * (first + 1.0));
			}
			if sigma >= smallest_test_sigma {
				let relative_error = (second_moment - sigma * sigma).abs() / (sigma * sigma);

				assert!(
					relative_error < 0.02,
					"Gaussian variance error {relative_error} at sigma {sigma}"
				);
			} else {
				assert_eq!(second_moment, 0.0);
			}
		}
	}

	#[test]
	fn backdrop_blur_variance_mapping_preserves_strength_at_one_and_two_x_scale() {
		for display_scale in [1.0f32, 2.0] {
			for radius in [0.25, 1.0, 4.0, 18.0, 32.0, 64.0] {
				let sigma = blur_sigma((radius * display_scale).clamp(0.0, 64.0));
				let resolution_mix = blur_resolution_mix(sigma);
				if blur_uses_full_resolution(resolution_mix) {
					let observed = blur_kernel_variance(UiBlurKernel::gaussian(sigma));
					let relative_error = (observed - sigma * sigma).abs() / (sigma * sigma);

					assert!(
						relative_error < 0.02,
						"Full-resolution variance error {relative_error} at radius {radius} and scale {display_scale}"
					);
				}
				if blur_uses_half_resolution(resolution_mix) {
					let half_variance = blur_kernel_variance(UiBlurKernel::gaussian(blur_half_sigma(sigma)));
					let observed = 4.0 * half_variance + 2.75;
					let relative_error = (observed - sigma * sigma).abs() / (sigma * sigma);

					assert!(
						relative_error < 0.05,
						"Half-resolution variance error {relative_error} at radius {radius} and scale {display_scale}"
					);
				}
			}
		}
	}

	/// Verifies the production Gaussian preserves constants, selects one axis, and guards extra lanes.
	#[test]
	fn backdrop_blur_filter_besl_vm_preserves_constants_and_direction() {
		let executable = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let region = UiPixelRegion {
			origin: [1, 1],
			extent: Extent::rectangle(1, 1),
		};
		let mut push_constant = blur_filter_push_constant(&executable, UiBlurKernel::gaussian(5.0).push([1.0, 0.0], region));
		let constant = [0.25, 0.5, 0.75, 1.0];
		let mut source = texture_2d(5, 5, &[constant; 25]);
		let mut result = empty_image(5, 5);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(besl::vm::ResourceSlot::new(0), &mut source);
			descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut result);
			descriptors.bind_push_constant(&mut push_constant);
			run_at(&executable, &mut descriptors, [0, 0]);
			run_at(&executable, &mut descriptors, [1, 0]);
		}
		assert_rgba_close(rgba(&result, [1, 1]), constant, 1e-5);
		assert_rgba_close(rgba(&result, [2, 1]), [0.0; 4], 1e-5);

		let width = 65;
		let center = width / 2;
		let mut impulse = vec![[0.0; 4]; width as usize * 3];
		impulse[(width + center) as usize] = [1.0; 4];
		let mut source = texture_2d(width, 3, &impulse);
		let mut result = empty_image(width, 3);
		let region = UiPixelRegion {
			origin: [0, 0],
			extent: Extent::rectangle(width, 3),
		};
		let mut push_constant = blur_filter_push_constant(&executable, UiBlurKernel::gaussian(5.0).push([1.0, 0.0], region));
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(besl::vm::ResourceSlot::new(0), &mut source);
			descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut result);
			descriptors.bind_push_constant(&mut push_constant);
			run_at(&executable, &mut descriptors, [center - 1, 1]);
			run_at(&executable, &mut descriptors, [center, 0]);
		}

		assert!(rgba(&result, [center - 1, 1])[0] > 0.0);
		assert_eq!(rgba(&result, [center, 0])[0], 0.0);
	}

	/// Verifies the effective-radius-36 production profile has one Gaussian peak without secondary bands.
	#[test]
	fn backdrop_blur_filter_besl_vm_has_no_secondary_lobe_at_effective_radius_36() {
		let executable = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let width = 65;
		let center = width / 2;
		let sigma = blur_half_sigma(blur_sigma(36.0));
		let kernel = UiBlurKernel::gaussian(sigma);
		let region = UiPixelRegion {
			origin: [0, 0],
			extent: Extent::rectangle(width, 1),
		};
		let mut push_constant = blur_filter_push_constant(&executable, kernel.push([1.0, 0.0], region));
		let mut impulse = vec![[0.0; 4]; width as usize];
		impulse[center as usize] = [1.0; 4];
		let mut source = texture_2d(width, 1, &impulse);
		let mut result = empty_image(width, 1);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(besl::vm::ResourceSlot::new(0), &mut source);
			descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut result);
			descriptors.bind_push_constant(&mut push_constant);
			for x in 0..width {
				run_at(&executable, &mut descriptors, [x, 0]);
			}
		}

		let profile = (0..width).map(|x| rgba(&result, [x, 0])[0]).collect::<Vec<_>>();
		let normalization = 1.0
			+ 2.0
				* (1..=UI_BLUR_GAUSSIAN_SUPPORT)
					.map(|distance| (-0.5 * (distance as f32 / sigma).powi(2)).exp())
					.sum::<f32>();
		for distance in 0..=UI_BLUR_GAUSSIAN_SUPPORT {
			let positive = profile[(center + distance) as usize];
			let negative = profile[(center - distance) as usize];
			let expected = (-0.5 * (distance as f32 / sigma).powi(2)).exp() / normalization;

			assert!(
				(positive - negative).abs() < 2e-6,
				"Asymmetric Gaussian at distance {distance}"
			);
			assert!(
				(positive - expected).abs() < 2e-5,
				"Unexpected Gaussian tap at distance {distance}"
			);
			if distance > 0 {
				assert!(profile[(center + distance - 1) as usize] >= positive);
			}
		}
		let energy = profile.iter().sum::<f32>();

		assert!((energy - 1.0).abs() < 2e-5, "Production Gaussian energy drifted to {energy}");
	}

	/// Verifies full-resolution composite sampling uses the texture's exact pixel lattice.
	#[test]
	fn backdrop_blur_composite_besl_vm_samples_full_resolution_lattice() {
		let output = run_blur_composite_vm(
			&[[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
			[2, 1],
			&[[0.0; 4]],
			[1, 1],
			[1.5, 0.5],
			0.0,
			[0.0; 4],
		);
		assert_rgba_close(output, [0.0, 1.0, 0.0, 1.0], 1e-6);
	}

	/// Verifies skipped resolution paths cannot contaminate a composite through stale values.
	#[test]
	fn backdrop_blur_composite_besl_vm_does_not_sample_inactive_resolution() {
		let nan = [f32::NAN; 4];
		let full = [0.2, 0.4, 0.6, 1.0];
		let half = [0.8, 0.6, 0.4, 1.0];
		let full_only = run_blur_composite_vm(&[full], [1, 1], &[nan], [1, 1], [0.5, 0.5], 0.0, [0.0; 4]);
		let half_only = run_blur_composite_vm(&[nan], [1, 1], &[half], [1, 1], [0.5, 0.5], 1.0, [0.0; 4]);
		assert_rgba_close(full_only, full, 1e-6);
		assert_rgba_close(half_only, half, 1e-6);
	}

	#[test]
	fn backdrop_blur_composite_besl_vm_blends_paths_and_preserves_feather_coverage() {
		let blended = run_blur_composite_vm(
			&[[1.0, 0.0, 0.0, 1.0]],
			[1, 1],
			&[[0.0, 0.0, 1.0, 1.0]],
			[1, 1],
			[0.5, 0.5],
			0.5,
			[0.0; 4],
		);
		assert_rgba_close(blended, [0.5, 0.0, 0.5, 1.0], 1e-6);

		let feathered = run_blur_composite_vm(
			&[[0.25, 0.5, 0.75, 1.0]],
			[1, 1],
			&[[0.0; 4]],
			[1, 1],
			[2.0, 2.0],
			0.0,
			[4.0, 0.0, 0.0, 0.0],
		);
		assert_rgba_close(feathered, [0.25, 0.5, 0.75, 0.5], 1e-6);
	}

	#[test]
	fn backdrop_blur_composite_besl_vm_keeps_awkward_widths_on_the_fixed_half_lattice() {
		for full_width in [2_801u32, 2_802, 2_803] {
			let half_width = full_width.div_ceil(UI_BLUR_HALF_DOWNSCALE);
			let full = vec![[0.0; 4]; full_width as usize];
			let half = (0..half_width)
				.map(|index| [index as f32 / (half_width - 1) as f32, 0.0, 0.0, 1.0])
				.collect::<Vec<_>>();
			let pixel_position = [full_width as f32 * 0.5, 0.5];
			let expected_coordinate = pixel_position[0] * 0.5 - 0.5;
			let output = run_blur_composite_vm(&full, [full_width, 1], &half, [half_width, 1], pixel_position, 1.0, [0.0; 4]);
			let expected = expected_coordinate / (half_width - 1) as f32;

			assert!(
				(output[0] - expected).abs() < 2e-5,
				"Half-lattice phase drift at width {full_width}"
			);
		}
	}

	#[test]
	fn backdrop_blur_resolution_crossover_selects_two_three_or_five_dispatches() {
		let dispatch_count = |sigma| {
			let resolution_mix = blur_resolution_mix(sigma);
			usize::from(blur_uses_full_resolution(resolution_mix)) * 2
				+ usize::from(blur_uses_half_resolution(resolution_mix)) * 3
		};

		assert_eq!(blur_resolution_mix(4.0), 0.0);
		assert_eq!(blur_resolution_mix(5.0), 0.5);
		assert_eq!(blur_resolution_mix(6.0), 1.0);
		assert_eq!(dispatch_count(4.0), 2);
		assert_eq!(dispatch_count(5.0), 5);
		assert_eq!(dispatch_count(6.0), 3);
		assert!(blur_resolution_mix(4.001) < 0.000_001);
		assert!(1.0 - blur_resolution_mix(5.999) < 0.000_001);

		let mut previous = 0.0;
		for step in 0..=512 {
			let resolution_mix = blur_resolution_mix(blur_sigma(step as f32 * 0.125));

			assert!(
				resolution_mix >= previous,
				"Resolution crossover stepped backward at sweep index {step}"
			);
			previous = resolution_mix;
		}
	}

	#[test]
	fn backdrop_blur_half_extent_keeps_every_awkward_edge_texel() {
		assert_eq!(blur_half_extent(Extent::rectangle(1920, 1080)), Extent::rectangle(960, 540));
		assert_eq!(blur_half_extent(Extent::rectangle(1919, 1079)), Extent::rectangle(960, 540));
		assert_eq!(blur_half_extent(Extent::rectangle(2802, 1)), Extent::rectangle(1401, 1));
		assert_eq!(blur_half_extent(Extent::rectangle(1, 1)), Extent::rectangle(1, 1));
	}

	#[test]
	fn backdrop_blur_dispatch_regions_pad_each_adaptive_path() {
		let viewport = Extent::rectangle(1920, 1080);
		let bounds = [400.0, 300.0, 800.0, 600.0];
		let full = blur_full_dispatch_regions(bounds, viewport);

		assert_eq!(
			full.vertical,
			UiPixelRegion {
				origin: [399, 299],
				extent: Extent::rectangle(402, 302),
			}
		);
		assert_eq!(
			full.horizontal,
			UiPixelRegion {
				origin: [398, 277],
				extent: Extent::rectangle(404, 346),
			}
		);

		let half = blur_half_dispatch_regions(bounds, viewport);

		assert_eq!(
			half.filter.vertical,
			UiPixelRegion {
				origin: [198, 148],
				extent: Extent::rectangle(204, 154),
			}
		);
		assert_eq!(
			half.filter.horizontal,
			UiPixelRegion {
				origin: [197, 126],
				extent: Extent::rectangle(206, 198),
			}
		);
		assert_eq!(
			half.downsample,
			UiPixelRegion {
				origin: [175, 125],
				extent: Extent::rectangle(250, 200),
			}
		);
	}

	#[test]
	// The footprint assertion intentionally visits every downsampled texel and its full tent support.
	#[allow(clippy::excessive_nesting)]
	fn backdrop_blur_half_region_contains_every_tent_sample_on_fixed_lattice() {
		let tent_offsets = [
			[-1.0, 0.0],
			[-0.5, 0.5],
			[0.0, 1.0],
			[0.5, 0.5],
			[1.0, 0.0],
			[0.5, -0.5],
			[0.0, -1.0],
			[-0.5, -0.5],
		];
		for width in [19, 2_801, 2_802, 2_803] {
			let viewport = Extent::rectangle(width, 13);
			let target = blur_half_extent(viewport);
			let bounds = [2.25, 1.75, width as f32 - 1.6, 11.2];
			let region = blur_half_dispatch_regions(bounds, viewport).filter.vertical;
			let end = [
				region.origin[0] + region.extent.width(),
				region.origin[1] + region.extent.height(),
			];
			let sample_xs = if width == 19 {
				(0..width).collect::<Vec<_>>()
			} else {
				vec![2, 3, width / 2, width - 3]
			};
			for y in 0..viewport.height() {
				for &x in &sample_xs {
					let pixel = [x as f32 + 0.5, y as f32 + 0.5];
					if pixel[0] < bounds[0] || pixel[0] >= bounds[2] || pixel[1] < bounds[1] || pixel[1] >= bounds[3] {
						continue;
					}
					let base = [pixel[0] * 0.5 - 0.5, pixel[1] * 0.5 - 0.5];
					for offset in tent_offsets {
						let sample = [base[0] + offset[0], base[1] + offset[1]];
						for sampled_y in [sample[1].floor(), sample[1].ceil()] {
							for sampled_x in [sample[0].floor(), sample[0].ceil()] {
								let sampled_x = sampled_x.clamp(0.0, target.width().saturating_sub(1) as f32) as u32;
								let sampled_y = sampled_y.clamp(0.0, target.height().saturating_sub(1) as f32) as u32;

								assert!((region.origin[0]..end[0]).contains(&sampled_x));
								assert!((region.origin[1]..end[1]).contains(&sampled_y));
							}
						}
					}
				}
			}
		}
	}

	/// Verifies every adaptive path executes the production shaders over representative UI signals.
	#[test]
	// The sweep keeps all radius, scale, and sampled-color assertions in one production-chain regression.
	#[allow(clippy::excessive_nesting)]
	fn backdrop_blur_production_besl_chain_sweep_preserves_positive_filtering() {
		let downsample = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let filter = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let mut composite = blur_composite_vm();
		let extent = Extent::rectangle(49, 5);
		let radii = [0.0, 0.25, 1.0, 4.0, 18.0, 32.0, 64.0];
		for pattern in [
			BlurChainPattern::Impulse,
			BlurChainPattern::ThinLine,
			BlurChainPattern::Checkerboard,
			BlurChainPattern::Constant,
		] {
			let texels = blur_chain_fixture(pattern, extent);
			let row = extent.height() / 2;
			let input = &texels[(row * extent.width()) as usize..((row + 1) * extent.width()) as usize];
			let input_variation = input.windows(2).map(|pair| (pair[1][0] - pair[0][0]).abs()).sum::<f32>();
			for display_scale in [1.0, 2.0] {
				for radius in radii {
					let output = run_adaptive_blur_scanline_vm(
						&downsample,
						&filter,
						&mut composite,
						&texels,
						extent,
						radius,
						display_scale,
					);
					for color in &output {
						for channel in color.iter().take(3) {
							assert!(
								channel.is_finite() && (0.0..=1.0).contains(channel),
								"Adaptive blur introduced an invalid color at radius {radius} and scale {display_scale}"
							);
						}
					}
					let output_variation = output.windows(2).map(|pair| (pair[1][0] - pair[0][0]).abs()).sum::<f32>();

					assert!(
						output_variation <= input_variation + 1e-4,
						"Positive blur increased scanline variation at radius {radius} and scale {display_scale}"
					);
					if matches!(pattern, BlurChainPattern::Constant) {
						for color in output {
							assert_rgba_close(color, [0.25, 0.5, 0.75, 1.0], 2e-5);
						}
					} else if radius == 0.0 {
						assert_eq!(output, input);
					} else {
						assert!(
							output
								.iter()
								.zip(input)
								.any(|(actual, source)| (actual[0] - source[0]).abs() > 1e-5)
						);
					}
				}
			}
		}
	}

	#[test]
	fn backdrop_blur_production_chain_changes_continuously_across_radius_sweep() {
		let downsample = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let filter = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let mut composite = blur_composite_vm();
		let extent = Extent::rectangle(49, 1);
		let texels = blur_chain_fixture(BlurChainPattern::ThinLine, extent);
		let mut sample_center = |radius| {
			run_adaptive_blur_scanline_vm(&downsample, &filter, &mut composite, &texels, extent, radius, 1.0)
				[extent.width() as usize / 2][0]
		};

		let at_zero = sample_center(0.0);
		let near_zero = sample_center(1e-6);

		assert!((at_zero - near_zero).abs() < 1e-6, "Blur popped when leaving radius zero");

		let mut previous = at_zero;
		let mut plateau_steps = 0;
		let mut largest_step = 0.0f32;
		for step in 1..=512 {
			let current = sample_center(step as f32 * 0.125);
			let delta = (current - previous).abs();

			assert!(current.is_finite());
			largest_step = largest_step.max(delta);
			plateau_steps += usize::from(delta <= 1e-7);
			previous = current;
		}

		assert!(
			largest_step < 0.4,
			"Radius sweep contained a visible output jump of {largest_step}"
		);
		assert!(plateau_steps <= 1, "Radius sweep retained {plateau_steps} quantized plateaus");

		let sigma_scale = blur_sigma(1.0);
		for crossover_sigma in [4.0f32, 6.0] {
			let crossover_radius = (crossover_sigma / sigma_scale).powi(2);
			let before = sample_center(crossover_radius - 0.001);
			let after = sample_center(crossover_radius + 0.001);

			assert!(
				(before - after).abs() < 5e-4,
				"Resolution crossover at sigma {crossover_sigma} introduced a discontinuity"
			);
		}
	}

	#[test]
	fn backdrop_blur_awkward_width_impulse_centroid_stays_phase_aligned() {
		let downsample = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let filter = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let mut composite = blur_composite_vm();
		for width in [2_801, 2_802] {
			let extent = Extent::rectangle(width, 1);
			let texels = blur_chain_fixture(BlurChainPattern::Impulse, extent);
			let output = run_adaptive_blur_scanline_vm(&downsample, &filter, &mut composite, &texels, extent, 18.0, 2.0);
			let energy = output.iter().map(|color| color[0]).sum::<f32>();
			let centroid = output.iter().enumerate().map(|(x, color)| x as f32 * color[0]).sum::<f32>() / energy;
			let source_centroid = (width / 2) as f32;

			assert!(
				(centroid - source_centroid).abs() <= 0.25,
				"Blur centroid drifted from {source_centroid} to {centroid} at width {width}"
			);
		}
	}

	#[test]
	fn backdrop_blur_regional_production_chain_never_samples_stale_texels() {
		let downsample = compile_ui_blur_shader(UI_BLUR_DOWNSAMPLE_BESL);
		let filter = compile_ui_blur_shader(UI_BLUR_FILTER_BESL);
		let mut composite = blur_composite_vm();
		let viewport = Extent::rectangle(129, 33);
		let bounds = [45.25, 10.25, 83.75, 22.75];
		let regions = blur_half_dispatch_regions(bounds, viewport);
		let target = blur_half_extent(viewport);
		let constant = [0.2, 0.4, 0.6, 1.0];
		let source_texels = vec![constant; (viewport.width() * viewport.height()) as usize];
		let stale_texels = vec![[f32::NAN; 4]; (target.width() * target.height()) as usize];
		let mut source = texture_2d(viewport.width(), viewport.height(), &source_texels);
		let mut downsampled = texture_2d(target.width(), target.height(), &stale_texels);
		run_blur_downsample_region_vm(&downsample, &mut source, &mut downsampled, regions.downsample);
		for y in regions.downsample.origin[1]..regions.downsample.origin[1] + regions.downsample.extent.height() {
			for x in regions.downsample.origin[0]..regions.downsample.origin[0] + regions.downsample.extent.width() {
				assert!(
					rgba(&downsampled, [x, y]).iter().all(|channel| channel.is_finite()),
					"Stale downsample texel at [{x}, {y}]"
				);
			}
		}

		let sigma = blur_sigma(36.0);
		let kernel = UiBlurKernel::gaussian(blur_half_sigma(sigma));
		let mut horizontal = texture_2d(target.width(), target.height(), &stale_texels);
		run_blur_filter_region_vm(
			&filter,
			&mut downsampled,
			&mut horizontal,
			kernel,
			[1.0, 0.0],
			regions.filter.horizontal,
		);
		for y in
			regions.filter.horizontal.origin[1]..regions.filter.horizontal.origin[1] + regions.filter.horizontal.extent.height()
		{
			for x in regions.filter.horizontal.origin[0]
				..regions.filter.horizontal.origin[0] + regions.filter.horizontal.extent.width()
			{
				assert!(
					rgba(&horizontal, [x, y]).iter().all(|channel| channel.is_finite()),
					"Stale horizontal texel at [{x}, {y}]"
				);
			}
		}
		let mut vertical = texture_2d(target.width(), target.height(), &stale_texels);
		run_blur_filter_region_vm(
			&filter,
			&mut horizontal,
			&mut vertical,
			kernel,
			[0.0, 1.0],
			regions.filter.vertical,
		);
		for y in regions.filter.vertical.origin[1]..regions.filter.vertical.origin[1] + regions.filter.vertical.extent.height()
		{
			for x in
				regions.filter.vertical.origin[0]..regions.filter.vertical.origin[0] + regions.filter.vertical.extent.width()
			{
				assert!(
					rgba(&vertical, [x, y]).iter().all(|channel| channel.is_finite()),
					"Stale vertical texel at [{x}, {y}]"
				);
			}
		}

		let full_stale = vec![[f32::NAN; 4]; (viewport.width() * viewport.height()) as usize];
		let mut full = texture_2d(viewport.width(), viewport.height(), &full_stale);
		for y in 0..viewport.height() {
			for x in 0..viewport.width() {
				let pixel = [x as f32 + 0.5, y as f32 + 0.5];
				if pixel[0] < bounds[0] || pixel[0] >= bounds[2] || pixel[1] < bounds[1] || pixel[1] >= bounds[3] {
					continue;
				}
				let output = run_blur_composite_textures_vm(&mut composite, &mut full, &mut vertical, pixel, 1.0, [0.0; 4]);
				assert_rgba_close(output, constant, 2e-5);
			}
		}
	}

	fn draw_element(corner_radius: f32, corner_exponent: f32) -> UiDrawElement {
		UiDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [50.0, 50.0],
			clip: None,
			clip_mask: None,
			color: [1.0, 1.0, 1.0, 1.0],
			corner_radius,
			corner_exponent,
			layer_kind: LayerKind::Fill,
			stroke_width: 0.0,
		}
	}

	fn image_pixels(width: u32, height: u32) -> Vec<u8> {
		vec![255; width as usize * height as usize * 4]
	}

	fn curve_element(segments: Vec<CurveSegment>) -> UiCurveDrawElement {
		UiCurveDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [100.0, 100.0],
			clip: None,
			clip_mask: None,
			color: [1.0, 1.0, 1.0, 1.0],
			stroke_width: 4.0,
			segments,
		}
	}

	fn elements_list(layout_size: [f32; 2], elements: Vec<UiDrawElement>) -> UiDrawList {
		UiDrawList {
			layout_size,
			elements,
			..UiDrawList::default()
		}
	}

	/// Builds a frame's primitives without caches or text, and returns them after the clear quad.
	fn build<'a>(
		draw_list: &UiDrawList,
		viewport: Extent,
		arena: &'a bumpalo::Bump,
	) -> (Vec<UiPrimitive>, UiPrimitives<'a>, UiMaskTable) {
		let mut masks = UiMaskTable::default();
		let output = build_ui_primitives_uncached(draw_list, viewport, arena, &mut masks);
		assert_eq!(output.primitives[0], clear_primitive(viewport));
		(output.primitives[1..].to_vec(), output, masks)
	}

	#[test]
	fn builds_one_primitive_and_one_draw_for_a_rectangle() {
		let frame_allocator = bumpalo::Bump::new();
		let list = elements_list(
			[100.0, 100.0],
			vec![UiDrawElement {
				position: [10.0, 20.0],
				size: [30.0, 40.0],
				color: [0.25, 0.5, 0.75, 1.0],
				..draw_element(8.0, 2.0)
			}],
		);
		let (primitives, output, _) = build(&list, Extent::rectangle(200, 100), &frame_allocator);

		// The first record is the clear quad, so draws start after it.
		assert_eq!(output.steps.as_slice(), [UiStep::Draw { first: 1, count: 1 }]);
		assert_eq!(
			primitives,
			[UiPrimitive {
				bounds: [20.0, 20.0, 80.0, 60.0],
				color: [0.25, 0.5, 0.75, 1.0],
				a: [20.0, 20.0, 60.0, 40.0],
				b: [8.0, 2.0, 0.0, 0.0],
				kind: UI_KIND_RECT,
				mask: 0,
				data0: 0,
				data1: 0,
			}]
		);
	}

	#[test]
	fn blur_builds_an_adaptive_composite_primitive_at_display_scale() {
		let frame_allocator = bumpalo::Bump::new();
		let list = UiDrawList {
			layout_size: [100.0, 100.0],
			blurs: vec![UiBlurDrawElement {
				depth: 2,
				order: 7,
				position: [10.0, 20.0],
				size: [30.0, 40.0],
				clip: None,
				clip_mask: None,
				color: [0.0, 0.0, 0.0, 0.45],
				corner_radius: 8.0,
				corner_exponent: 2.0,
				radius: 18.0,
			}],
			..UiDrawList::default()
		};
		let (primitives, output, _) = build(&list, Extent::rectangle(200, 200), &frame_allocator);

		let expected_sigma = blur_sigma(36.0);
		// The empty first draw is the pass that clears the damage before the blur reads the layer.
		let [
			UiStep::Draw { first: 1, count: 0 },
			UiStep::Blur(blur),
			UiStep::Draw { first: 1, count: 1 },
		] = output.steps.as_slice()
		else {
			panic!("A blur must dispatch before the draw that holds its quad: {:?}", output.steps);
		};
		assert_eq!(blur.resolution_mix, 1.0);
		assert_eq!(blur.full_kernel, UiBlurKernel::gaussian(expected_sigma));
		assert_eq!(blur.half_kernel, UiBlurKernel::gaussian(blur_half_sigma(expected_sigma)));
		assert_eq!(
			blur.half_regions.filter.vertical,
			UiPixelRegion {
				origin: [8, 18],
				extent: Extent::rectangle(34, 44),
			}
		);
		assert_eq!(primitives.len(), 1);
		assert_eq!(primitives[0].kind, UI_KIND_BLUR);
		assert_eq!(primitives[0].bounds, [20.0, 40.0, 80.0, 120.0]);
		assert_eq!(primitives[0].b, [16.0, 2.0, 0.0, 1.0]);
		assert_eq!(primitives[0].color, [0.0, 0.0, 0.0, 0.45]);
	}

	#[test]
	fn primitives_merge_every_element_type_in_painter_order() {
		let frame_allocator = bumpalo::Bump::new();
		let rectangle = |depth: u32, order: u32| UiDrawElement {
			depth,
			order,
			..draw_element(0.0, 2.0)
		};
		let list = UiDrawList {
			layout_size: [100.0, 100.0],
			elements: vec![rectangle(0, 1), rectangle(1, 3), rectangle(1, 6), rectangle(2, 9)],
			blurs: vec![UiBlurDrawElement {
				depth: 1,
				order: 3,
				position: [0.0, 0.0],
				size: [50.0, 50.0],
				clip: None,
				clip_mask: None,
				color: [0.0; 4],
				corner_radius: 0.0,
				corner_exponent: 2.0,
				radius: 8.0,
			}],
			curves: vec![UiCurveDrawElement {
				depth: 1,
				order: 5,
				..curve_element(vec![CurveSegment::Line {
					from: CurvePoint::new(10.0, 10.0),
					to: CurvePoint::new(40.0, 10.0),
				}])
			}],
			images: vec![UiImageDrawElement {
				depth: 1,
				order: 4,
				image_id: 1,
				version: 1,
				source_width: 1,
				source_height: 1,
				pixels: image_pixels(1, 1).into(),
				position: [0.0, 0.0],
				size: [10.0, 10.0],
				clip: None,
				clip_mask: None,
				opacity: 1.0,
			}],
			texts: Vec::new(),
		};
		let (primitives, output, _) = build(&list, Extent::square(100), &frame_allocator);

		// Depth orders first and the element second, across types. A blur goes under its own element's layers.
		let kinds: Vec<_> = primitives.iter().map(|primitive| primitive.kind).collect();
		assert_eq!(
			kinds,
			[
				UI_KIND_RECT,
				UI_KIND_BLUR,
				UI_KIND_RECT,
				UI_KIND_IMAGE,
				UI_KIND_CURVE,
				UI_KIND_RECT,
				UI_KIND_RECT
			]
		);
		// Only the blur splits the frame: everything before it is one draw and everything from its quad on is another.
		assert!(matches!(
			output.steps.as_slice(),
			[
				UiStep::Draw { first: 1, count: 1 },
				UiStep::Blur(_),
				UiStep::Draw { first: 2, count: 6 }
			]
		));
		// The pass fills in the texture slot of the image primitive it is told about.
		assert_eq!(output.images.as_slice(), [(4, 0)]);
	}

	#[test]
	fn many_rectangles_share_one_draw() {
		let frame_allocator = bumpalo::Bump::new();
		let count = (u16::MAX as usize + 1) / 4 + 1;
		let list = elements_list([1.0, 1.0], vec![draw_element(0.0, 2.0); count]);
		let (primitives, output, _) = build(&list, Extent::square(1), &frame_allocator);

		// No index buffer means no vertex limit per draw.
		assert_eq!(primitives.len(), count);
		assert_eq!(
			output.steps.as_slice(),
			[UiStep::Draw {
				first: 1,
				count: count as u32
			}]
		);
	}

	#[test]
	fn scales_corner_radius_to_viewport_pixels() {
		let frame_allocator = bumpalo::Bump::new();
		let list = elements_list([100.0, 100.0], vec![draw_element(8.0, 2.0)]);
		let (primitives, ..) = build(&list, Extent::square(200), &frame_allocator);

		assert_eq!(primitives[0].b[0], 16.0);
	}

	#[test]
	fn clamps_corner_radius_to_half_the_shortest_edge() {
		let frame_allocator = bumpalo::Bump::new();
		let list = elements_list(
			[100.0, 100.0],
			vec![UiDrawElement {
				size: [40.0, 20.0],
				..draw_element(64.0, 2.0)
			}],
		);
		let (primitives, ..) = build(&list, Extent::square(100), &frame_allocator);

		assert_eq!(primitives[0].b[0], 10.0);
	}

	#[test]
	fn clipped_rectangle_trims_its_quad_but_keeps_its_shape() {
		let frame_allocator = bumpalo::Bump::new();
		let list = elements_list(
			[100.0, 100.0],
			vec![UiDrawElement {
				position: [10.0, 10.0],
				size: [40.0, 40.0],
				clip: Some(DrawClip {
					position: [20.0, 15.0],
					size: [10.0, 50.0],
				}),
				..draw_element(8.0, 2.0)
			}],
		);
		let (primitives, _, masks) = build(&list, Extent::square(100), &frame_allocator);

		// The quad shrinks to the clip while the shader still measures corners from the whole rectangle.
		assert_eq!(primitives[0].bounds, [20.0, 15.0, 30.0, 50.0]);
		assert_eq!(primitives[0].a, [10.0, 10.0, 40.0, 40.0]);
		// A quad the CPU trimmed needs no clip on the GPU.
		assert_eq!(primitives[0].mask, 0);
		assert_eq!(masks.entries().len(), 1);
	}

	#[test]
	fn clip_masks_scale_to_viewport_pixels_and_are_shared() {
		let frame_allocator = bumpalo::Bump::new();
		let mask = Some(DrawClipMask {
			position: [10.0, 20.0],
			size: [30.0, 40.0],
			edges: [1.0, 2.0, 3.0, 4.0],
			corner: [5.0, 3.0],
			rotation: Rotation {
				cos: 0.0,
				sin: 1.0,
				x: 7.0,
				y: 9.0,
			},
		});
		let list = elements_list(
			[100.0, 100.0],
			vec![
				UiDrawElement {
					clip_mask: mask,
					..draw_element(0.0, 2.0)
				},
				draw_element(0.0, 2.0),
				UiDrawElement {
					clip_mask: mask,
					..draw_element(0.0, 2.0)
				},
			],
		);
		let (primitives, _, masks) = build(&list, Extent::rectangle(200, 300), &frame_allocator);

		assert_eq!(
			primitives.iter().map(|primitive| primitive.mask).collect::<Vec<_>>(),
			[1, 0, 1]
		);
		assert_eq!(masks.entries().len(), 2);
		let entry = masks.entries()[1];
		assert_eq!(entry.rect, [20.0, 60.0, 60.0, 120.0]);
		assert_eq!(entry.edges, [3.0, 4.0, 9.0, 8.0]);
		assert_eq!(entry.corner, [10.0, 3.0, 0.0, 0.0]);
		assert_eq!(entry.rotation, [0.0, 1.0, 14.0, 27.0]);
	}

	#[test]
	fn skips_elements_that_draw_nothing_before_capacity_checks() {
		let frame_allocator = bumpalo::Bump::new();
		let hidden = [
			UiDrawElement {
				color: [1.0, 1.0, 1.0, 0.0],
				..draw_element(0.0, 2.0)
			},
			UiDrawElement {
				clip: Some(DrawClip {
					position: [80.0, 80.0],
					size: [10.0, 10.0],
				}),
				..draw_element(0.0, 2.0)
			},
			UiDrawElement {
				layer_kind: LayerKind::Stroke { width: f32::NAN },
				stroke_width: f32::NAN,
				..draw_element(0.0, 2.0)
			},
		];
		let mut elements: Vec<_> = hidden.iter().cycle().take(MAX_UI_PRIMITIVES + 3).copied().collect();
		elements.push(draw_element(0.0, 2.0));
		let (primitives, output, _) = build(
			&elements_list([100.0, 100.0], elements),
			Extent::square(100),
			&frame_allocator,
		);

		assert!(!output.truncated);
		assert_eq!(primitives.len(), 1);
	}

	#[test]
	fn reports_truncation_when_the_primitive_buffer_is_full() {
		let frame_allocator = bumpalo::Bump::new();
		let list = elements_list([1.0, 1.0], vec![draw_element(0.0, 2.0); MAX_UI_PRIMITIVES]);
		let (_, output, _) = build(&list, Extent::square(1), &frame_allocator);

		// The clear quad takes the first record.
		assert!(output.truncated);
		assert_eq!(output.primitives.len(), MAX_UI_PRIMITIVES);
		assert_eq!(
			output.steps.as_slice(),
			[UiStep::Draw {
				first: 1,
				count: MAX_UI_PRIMITIVES as u32 - 1
			}]
		);
	}

	#[test]
	fn corner_shapes_resolve_to_what_the_shader_can_draw() {
		let frame_allocator = bumpalo::Bump::new();
		let list = elements_list(
			[100.0, 100.0],
			vec![
				draw_element(-4.0, 2.0),
				draw_element(8.0, 3.5),
				draw_element(8.0, f32::NAN),
				draw_element(8.0, 0.5),
				draw_element(8.0, 64.0),
			],
		);
		let (primitives, ..) = build(&list, Extent::square(100), &frame_allocator);

		let shapes: Vec<_> = primitives.iter().map(|primitive| [primitive.b[0], primitive.b[1]]).collect();
		assert_eq!(shapes, [[0.0, 2.0], [8.0, 3.5], [8.0, 2.0], [8.0, 2.0], [8.0, 8.0]]);
	}

	#[test]
	fn stroke_width_tells_a_stroke_from_a_fill() {
		let frame_allocator = bumpalo::Bump::new();
		let list = elements_list(
			[100.0, 100.0],
			vec![
				draw_element(8.0, 2.0),
				UiDrawElement {
					layer_kind: LayerKind::Stroke { width: 3.0 },
					stroke_width: 3.0,
					..draw_element(8.0, 2.0)
				},
			],
		);
		let (primitives, ..) = build(&list, Extent::square(200), &frame_allocator);

		assert_eq!(primitives[0].b[2], 0.0);
		// Strokes scale with the viewport like corner radii.
		assert_eq!(primitives[1].b[2], 6.0);
	}

	fn cubic_wire() -> CurveSegment {
		CurveSegment::Cubic {
			from: CurvePoint::new(10.0, 20.0),
			control0: CurvePoint::new(50.0, 20.0),
			control1: CurvePoint::new(50.0, 80.0),
			to: CurvePoint::new(90.0, 80.0),
		}
	}

	fn curve_list(curve: UiCurveDrawElement) -> UiDrawList {
		UiDrawList {
			layout_size: [100.0, 100.0],
			curves: vec![curve],
			..UiDrawList::default()
		}
	}

	#[test]
	fn curve_segments_become_pieces_of_one_cubic() {
		let frame_allocator = bumpalo::Bump::new();
		let list = curve_list(curve_element(vec![cubic_wire()]));
		let (primitives, output, _) = build(&list, Extent::square(200), &frame_allocator);

		// The CPU only picks a piece count; every piece carries the whole cubic in viewport pixels.
		let count = primitives.len() as u32;
		assert!((2..=16).contains(&count), "unexpected piece count {count}");
		assert_eq!(output.steps.as_slice(), [UiStep::Draw { first: 1, count }]);
		for (piece, primitive) in primitives.iter().enumerate() {
			assert_eq!(primitive.kind, UI_KIND_CURVE);
			assert_eq!(primitive.bounds, [20.0, 40.0, 100.0, 40.0]);
			assert_eq!(primitive.a, [100.0, 160.0, 180.0, 160.0]);
			// Half of the four unit stroke, scaled by two.
			assert_eq!(primitive.b[0], 4.0);
			assert_eq!(primitive.data0, piece as u32 | count << 16);
			// A lone segment rounds off both of its ends.
			assert_eq!(primitive.data1, UI_CURVE_CAP_START | UI_CURVE_CAP_END);
		}
	}

	#[test]
	fn lines_and_quadratics_are_stored_as_the_cubic_that_traces_them() {
		let frame_allocator = bumpalo::Bump::new();
		let list = curve_list(curve_element(vec![
			CurveSegment::Line {
				from: CurvePoint::new(0.0, 0.0),
				to: CurvePoint::new(30.0, 60.0),
			},
			CurveSegment::Quadratic {
				from: CurvePoint::new(0.0, 0.0),
				control: CurvePoint::new(3.0, 6.0),
				to: CurvePoint::new(9.0, 0.0),
			},
		]));
		let (primitives, ..) = build(&list, Extent::square(100), &frame_allocator);

		// A straight line needs one piece however long it is.
		assert_eq!(primitives.len(), 2);
		assert_eq!(primitives[0].bounds, [0.0, 0.0, 10.0, 20.0]);
		assert_eq!(primitives[0].a, [20.0, 40.0, 30.0, 60.0]);
		assert_eq!(primitives[1].bounds, [0.0, 0.0, 2.0, 4.0]);
		assert_eq!(primitives[1].a, [5.0, 4.0, 9.0, 0.0]);
	}

	#[test]
	fn curve_piece_count_follows_bend_and_fit_error() {
		let line = [[0.0, 0.0], [100.0, 0.0], [200.0, 0.0], [300.0, 0.0]];
		let gentle = [[0.0, 0.0], [100.0, 4.0], [200.0, 4.0], [300.0, 0.0]];
		let wire = [[0.0, 0.0], [150.0, 0.0], [150.0, 100.0], [300.0, 100.0]];
		let wild = [[0.0, 0.0], [90_000.0, 0.0], [-90_000.0, 100.0], [300.0, 100.0]];

		assert_eq!(curve_piece_count(&line), 1);
		assert!(curve_piece_count(&gentle) <= 2);
		assert!((4..=12).contains(&curve_piece_count(&wire)));
		assert_eq!(curve_piece_count(&wild), MAX_CURVE_PIECES);
		// Zooming in bends a curve further from its chords in pixels, so it takes more pieces.
		let zoomed = wire.map(|point| [point[0] * 4.0, point[1] * 4.0]);
		assert!(curve_piece_count(&zoomed) > curve_piece_count(&wire));
	}

	#[test]
	fn curve_caps_round_path_ends_and_corners_but_not_smooth_joints() {
		let frame_allocator = bumpalo::Bump::new();
		let line = |from: [f32; 2], to: [f32; 2]| CurveSegment::Line {
			from: CurvePoint::new(from[0], from[1]),
			to: CurvePoint::new(to[0], to[1]),
		};
		let list = curve_list(curve_element(vec![
			line([10.0, 10.0], [30.0, 10.0]),
			// Continues straight on: a smooth joint.
			line([30.0, 10.0], [50.0, 10.0]),
			// Turns: a corner, which the segment that starts at it rounds.
			line([50.0, 10.0], [50.0, 40.0]),
			// No length: it leaves no primitive and its neighbors still meet.
			line([50.0, 40.0], [50.0, 40.0]),
			// Starts somewhere else: a new path.
			line([70.0, 70.0], [90.0, 70.0]),
		]));
		let (primitives, ..) = build(&list, Extent::square(100), &frame_allocator);

		let caps: Vec<_> = primitives.iter().map(|primitive| primitive.data1).collect();
		assert_eq!(
			caps,
			[
				UI_CURVE_CAP_START,
				0,
				UI_CURVE_CAP_START | UI_CURVE_CAP_END,
				UI_CURVE_CAP_START | UI_CURVE_CAP_END
			]
		);
	}

	#[test]
	fn curves_carry_their_clip_to_the_shader() {
		let frame_allocator = bumpalo::Bump::new();
		let list = curve_list(UiCurveDrawElement {
			clip: Some(DrawClip {
				position: [20.0, 0.0],
				size: [30.0, 100.0],
			}),
			..curve_element(vec![cubic_wire()])
		});
		let (primitives, _, masks) = build(&list, Extent::square(200), &frame_allocator);

		// A curve's quads are not axis aligned, so the fragment shader clips them instead of the CPU.
		assert!(primitives.iter().all(|primitive| primitive.mask == 1));
		assert_eq!(masks.entries()[1].clip, [40.0, 0.0, 100.0, 200.0]);
		assert_eq!(masks.entries()[1].rect, [0.0; 4]);
	}

	#[test]
	fn curve_skips_invalid_or_non_positive_strokes() {
		let frame_allocator = bumpalo::Bump::new();
		for (stroke_width, alpha) in [(0.0, 1.0), (-1.0, 1.0), (f32::NAN, 1.0), (4.0, 0.0)] {
			let list = curve_list(UiCurveDrawElement {
				stroke_width,
				color: [1.0, 1.0, 1.0, alpha],
				..curve_element(vec![cubic_wire()])
			});
			let (primitives, ..) = build(&list, Extent::square(100), &frame_allocator);
			assert!(primitives.is_empty());
		}
	}

	#[test]
	fn prepared_frame_only_matches_its_own_revision_extent_and_glyph_generation() {
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("root").container(Container::default());
			})
		});
		let frame_allocator = bumpalo::Bump::new();
		let mut snapshot = engine.evaluate(Size::new(10, 10), &frame_allocator);
		let revision = Some(engine.render(&mut snapshot).revision());
		let damage = vec![UiPixelRegion::full(Extent::square(64))];
		let prepared = UiPreparedFrame {
			revision,
			extent: Extent::square(64),
			glyph_generation: 2,
			damage: damage.clone(),
			steps: Vec::new(),
		};

		assert!(prepared.matches(revision, Extent::square(64), 2, &damage));
		assert!(!prepared.matches(None, Extent::square(64), 2, &damage));
		assert!(!prepared.matches(revision, Extent::square(65), 2, &damage));
		assert!(!prepared.matches(revision, Extent::square(64), 3, &damage));
		assert!(!prepared.matches(revision, Extent::square(64), 2, &[]));
	}

	#[test]
	fn update_ignores_a_render_whose_revision_was_already_adopted() {
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("label").text(Text::new("Stable"));
				loop {
					ctx.render().await;
				}
			})
		});
		let frame_allocator = bumpalo::Bump::new();
		let mut draw_list = UiDrawList::default();
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let first = engine.render(&mut snapshot);
		update_from_render(first, &mut draw_list);
		let first = first.revision();
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let second = engine.render(&mut snapshot);

		assert_eq!(first, second.revision());
		assert_eq!(draw_list.texts.len(), 1);
	}

	/// Verifies the fragment shader scales an atlas glyph's alpha by the sampled coverage.
	#[test]
	fn ui_fragment_besl_vm_multiplies_color_alpha_by_atlas_coverage() {
		let glyph = UiPrimitive {
			bounds: [0.0, 0.0, 2.0, 2.0],
			color: [0.2, 0.4, 0.6, 0.8],
			a: [0.0, 0.0, 1.0, 1.0],
			kind: UI_KIND_ATLAS_GLYPH,
			..UiPrimitive::default()
		};
		let mut atlas = texture_2d(2, 2, &[[0.5, 0.0, 0.0, 1.0]; 4]);
		let color = UiFragmentVm::new(&[glyph], &[UiClipMaskEntry::NONE], None).run(
			UiVaryings {
				pixel_position: [1.5, 1.5],
				uv: [0.5, 0.5],
				..UiVaryings::default()
			},
			// The atlas is the first element of the texture array.
			&mut [(UI_TEXTURES_SLOT.index() + UI_ATLAS_TEXTURE_SLOT, &mut atlas)],
		);

		assert_vec4_close(color, [0.2, 0.4, 0.6, 0.4]);
	}

	/// Verifies the fragment shader samples an image from its texture slot and applies its opacity.
	#[test]
	fn ui_fragment_besl_vm_samples_an_image_from_its_texture_slot() {
		let image = UiPrimitive {
			bounds: [0.0, 0.0, 2.0, 2.0],
			color: [1.0, 1.0, 1.0, 0.5],
			a: [0.0, 0.0, 1.0, 1.0],
			kind: UI_KIND_IMAGE,
			data0: 7,
			..UiPrimitive::default()
		};
		let mut texture = texture_2d(1, 1, &[[0.1, 0.2, 0.3, 0.8]]);
		let color = UiFragmentVm::new(&[image], &[UiClipMaskEntry::NONE], None).run(
			UiVaryings {
				pixel_position: [1.0, 1.0],
				uv: [0.5, 0.5],
				..UiVaryings::default()
			},
			&mut [(UI_TEXTURES_SLOT.index() + 7, &mut texture)],
		);

		assert_vec4_close(color, [0.1, 0.2, 0.3, 0.4]);
	}

	/// Runs the production shaders at every pixel center of a label's glyph quads.
	///
	/// Returns each covered pixel with the alpha the shader wrote, which is the glyph coverage for an opaque white label.
	fn slug_label_coverage(content: &str, position: [f32; 2], font_size: f32, extent: u32) -> Vec<([u32; 2], f32)> {
		let mut text_system = TextSystem::new();
		let mut glyphs = UiGlyphCurves::new(UI_GLYPH_CURVE_CAPACITY, UI_GLYPH_BAND_CAPACITY);
		let mut masks = UiMaskTable::default();
		let arena = bumpalo::Bump::new();
		let draw_list = UiDrawList {
			layout_size: [extent as f32; 2],
			texts: vec![UiTextDrawElement {
				depth: 0,
				order: 0,
				position,
				size: [extent as f32; 2],
				clip: None,
				clip_mask: None,
				color: RGBA::white(),
				font_size,
				text: content.to_string(),
			}],
			..UiDrawList::default()
		};
		let geometry = build_ui_slug_geometry(
			&draw_list,
			Extent::square(extent),
			&mut text_system,
			&mut glyphs,
			&mut masks,
			&arena,
		);

		let mut vertex = UiVertexVm::new(&geometry.primitives, masks.entries(), [extent as f32; 2]);
		let mut fragment = UiFragmentVm::new(&geometry.primitives, masks.entries(), Some(&glyphs));
		let mut pixels = Vec::new();
		for glyph in 0..geometry.primitives.len() as u32 {
			rasterize_primitive(&mut vertex, &mut fragment, glyph, |[x, y], color| {
				pixels.push(([x as u32, y as u32], color[3]));
			});
		}
		pixels
	}

	/// Verifies that packed outlines evaluated by the Slug shader cover the same pixels as the CPU rasterizer.
	#[test]
	fn ui_fragment_besl_vm_slug_glyphs_match_rasterized_coverage() {
		let mut fonts = TextSystem::new();
		if !fonts.has_font() {
			return;
		}
		for (character, font_size) in [('@', 32.0f32), ('g', 24.0), ('B', 16.0), ('W', 12.0), ('8', 48.0)] {
			// A whole-pixel pen and baseline put the shader's samples on the rasterizer's pixel grid.
			let position = [8.0f32, 4.0];
			let line = fonts.line_metrics(font_size).unwrap();
			let baseline = (position[1] + line.ascent.max(font_size * 0.8f32)).round();
			let glyph = fonts.glyph(character, font_size).unwrap().clone();
			let left = position[0] as i32 + glyph.xmin;
			let top = baseline as i32 - glyph.ymin - glyph.height as i32;

			let pixels = slug_label_coverage(&character.to_string(), position, font_size, 96);
			// The quad spans the whole bitmap, so no rasterized coverage goes unsampled.
			assert!(pixels.len() >= (glyph.width * glyph.height) as usize);
			let (mut total, mut worst) = (0.0f32, 0.0f32);
			for ([x, y], coverage) in &pixels {
				let (column, row) = (*x as i32 - left, *y as i32 - top);
				let inside = column >= 0 && row >= 0 && column < glyph.width as i32 && row < glyph.height as i32;
				let expected = if inside {
					glyph.bitmap[(row as u32 * glyph.width + column as u32) as usize] as f32 / 255.0
				} else {
					0.0
				};
				total += (coverage - expected).abs();
				worst = worst.max((coverage - expected).abs());
			}
			// Two rays per pixel estimate the exact area the rasterizer integrates, so single edge
			// pixels may differ while the glyph as a whole must agree closely.
			let mean = total / pixels.len() as f32;
			assert!(
				mean < 0.03,
				"'{character}' at {font_size} px differs by {mean} per pixel on average"
			);
			assert!(
				worst < 0.35,
				"'{character}' at {font_size} px differs by {worst} at one pixel"
			);
		}
	}

	#[test]
	fn checked_in_ui_raster_besl_sources_link() {
		for (shader_name, source) in [("UI vertex shader", UI_VERTEX_BESL), ("UI fragment shader", UI_FRAGMENT_BESL)] {
			ui_raster_program(source, shader_name);
		}
	}

	/// Verifies the vertex shader pulls a quad's corners, texture rectangle, and record index without vertex buffers.
	#[test]
	fn ui_vertex_besl_vm_pulls_quads_from_primitive_records() {
		let primitives = [
			UiPrimitive::default(),
			UiPrimitive {
				bounds: [20.0, 20.0, 80.0, 60.0],
				a: [0.25, 0.5, 0.75, 1.0],
				kind: UI_KIND_IMAGE,
				..UiPrimitive::default()
			},
			UiPrimitive {
				bounds: [20.0, 20.0, 80.0, 60.0],
				mask: 1,
				..UiPrimitive::default()
			},
		];
		// A quarter turn clockwise around the quad's top left corner.
		let masks = [
			UiClipMaskEntry::NONE,
			UiClipMaskEntry {
				rotation: [0.0, 1.0, 40.0, 0.0],
				..UiClipMaskEntry::NONE
			},
		];
		let mut vertex = UiVertexVm::new(&primitives, &masks, [200.0, 100.0]);

		// Six vertices per record, so a draw that starts at record one reaches it with its first vertex.
		let corners: Vec<_> = (0..6).map(|index| vertex.run(1, index)).collect();
		let pixels: Vec<_> = corners.iter().map(|(_, varyings)| varyings.pixel_position).collect();
		assert_eq!(
			pixels,
			[
				[20.0, 20.0],
				[80.0, 20.0],
				[80.0, 60.0],
				[80.0, 60.0],
				[20.0, 60.0],
				[20.0, 20.0]
			]
		);
		assert_vec4_close(corners[0].0, [-0.8, 0.6, 0.0, 1.0]);
		assert_vec4_close(corners[2].0, [-0.2, -0.2, 0.0, 1.0]);
		assert_vec2_close(corners[0].1.uv, [0.25, 0.5]);
		assert_vec2_close(corners[2].1.uv, [0.75, 1.0]);
		assert!(corners.iter().all(|(_, varyings)| varyings.primitive == 1));
		// The next six vertices of the same draw belong to the next record.
		assert_eq!(vertex.run(0, 6).1.primitive, 1);
		// A turned quad moves on screen while its fragments keep shading where it was laid out. It
		// also grows by a pixel, which the fragment stage fades its edges across.
		let (position, turned) = vertex.run(2, 1);
		assert_vec2_close(turned.pixel_position, [81.0, 19.0]);
		assert_vec2_close(turned.screen_position.unwrap(), [21.0, 81.0]);
		assert_vec4_close(position, [-0.79, -0.62, 0.0, 1.0]);
	}

	/// The distance from a point to a cubic, by dense sampling.
	fn distance_to_cubic(cubic: &[[f32; 2]; 4], point: [f32; 2]) -> f32 {
		let samples = 4000;
		let at = |t: f32| {
			let s = 1.0 - t;
			[0, 1].map(|axis| {
				s * s * s * cubic[0][axis]
					+ 3.0 * s * s * t * cubic[1][axis]
					+ 3.0 * s * t * t * cubic[2][axis]
					+ t * t * t * cubic[3][axis]
			})
		};
		(0..samples)
			.map(|index| {
				let (from, to) = (at(index as f32 / samples as f32), at((index + 1) as f32 / samples as f32));
				let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
				let t =
					(((point[0] - from[0]) * dx + (point[1] - from[1]) * dy) / (dx * dx + dy * dy).max(1e-12)).clamp(0.0, 1.0);
				(point[0] - from[0] - dx * t).hypot(point[1] - from[1] - dy * t)
			})
			.fold(f32::INFINITY, f32::min)
	}

	/// Verifies that the pieces of an analytic curve tile its stroke: every pixel matches the exact
	/// distance to the curve, no pixel is shaded by two pieces, and no stroke pixel falls outside the quads.
	#[test]
	fn ui_shaders_besl_vm_draw_an_analytic_curve_without_seams() {
		let frame_allocator = bumpalo::Bump::new();
		let extent = 64u32;
		for (segment, stroke_width) in [
			(
				CurveSegment::Cubic {
					from: CurvePoint::new(6.5, 10.25),
					control0: CurvePoint::new(34.0, 10.25),
					control1: CurvePoint::new(30.0, 52.0),
					to: CurvePoint::new(57.0, 52.75),
				},
				3.0,
			),
			(
				CurveSegment::Quadratic {
					from: CurvePoint::new(8.0, 50.0),
					control: CurvePoint::new(30.0, 4.0),
					to: CurvePoint::new(56.0, 44.0),
				},
				1.5,
			),
			(
				CurveSegment::Line {
					from: CurvePoint::new(9.0, 9.5),
					to: CurvePoint::new(52.25, 40.0),
				},
				2.0,
			),
		] {
			let cubic = match segment {
				CurveSegment::Cubic {
					from,
					control0,
					control1,
					to,
				} => [from, control0, control1, to].map(|point| [point.x, point.y]),
				CurveSegment::Quadratic { from, control, to } => {
					let third = |a: CurvePoint, b: CurvePoint| [a.x + (b.x - a.x) * 2.0 / 3.0, a.y + (b.y - a.y) * 2.0 / 3.0];
					[[from.x, from.y], third(from, control), third(to, control), [to.x, to.y]]
				}
				CurveSegment::Line { from, to } => [[from.x, from.y], [from.x, from.y], [to.x, to.y], [to.x, to.y]],
			};
			let list = UiDrawList {
				layout_size: [extent as f32; 2],
				curves: vec![UiCurveDrawElement {
					stroke_width,
					..curve_element(vec![segment.clone()])
				}],
				..UiDrawList::default()
			};
			let (primitives, _, masks) = build(&list, Extent::square(extent), &frame_allocator);
			let mut vertex = UiVertexVm::new(&primitives, masks.entries(), [extent as f32; 2]);
			let mut fragment = UiFragmentVm::new(&primitives, &[UiClipMaskEntry::NONE], None);

			// Alpha per pixel, and how many pieces shaded it at all.
			let mut coverage = vec![(0.0f32, 0u32); (extent * extent) as usize];
			for piece in 0..primitives.len() as u32 {
				rasterize_primitive(&mut vertex, &mut fragment, piece, |[x, y], color| {
					if x < 0 || y < 0 || x >= extent as i32 || y >= extent as i32 || color[3] <= 0.0 {
						return;
					}
					let pixel = &mut coverage[(y as u32 * extent + x as u32) as usize];
					// Alpha blending accumulates coverage the way the layer does.
					pixel.0 = pixel.0 + color[3] * (1.0 - pixel.0);
					pixel.1 += 1;
				});
			}

			// Only a cubic is approximated, by quadratics that stay within the fit tolerance of it.
			let tolerance = if matches!(segment, CurveSegment::Cubic { .. }) {
				CURVE_QUADRATIC_TOLERANCE_PIXELS + 0.01
			} else {
				0.001
			};
			for y in 0..extent {
				for x in 0..extent {
					let (alpha, pieces) = coverage[(y * extent + x) as usize];
					let distance = distance_to_cubic(&cubic, [x as f32 + 0.5, y as f32 + 0.5]);
					let expected = (0.5 - (distance - stroke_width * 0.5)).clamp(0.0, 1.0);
					assert!(
						(alpha - expected).abs() < tolerance,
						"{segment:?}: pixel ({x}, {y}) has coverage {alpha}, expected {expected}"
					);
					assert!(pieces <= 1, "{segment:?}: pixel ({x}, {y}) is shaded by {pieces} pieces");
				}
			}
		}
	}

	/// Verifies that a curve's hard clip reaches the fragment shader through its mask entry.
	#[test]
	fn ui_fragment_besl_vm_clips_curves_per_pixel() {
		let frame_allocator = bumpalo::Bump::new();
		let list = UiDrawList {
			layout_size: [64.0, 64.0],
			curves: vec![UiCurveDrawElement {
				clip: Some(DrawClip {
					position: [0.0, 0.0],
					size: [32.0, 64.0],
				}),
				..curve_element(vec![CurveSegment::Line {
					from: CurvePoint::new(8.0, 20.5),
					to: CurvePoint::new(56.0, 20.5),
				}])
			}],
			..UiDrawList::default()
		};
		let (primitives, _, masks) = build(&list, Extent::square(64), &frame_allocator);
		let mut vertex = UiVertexVm::new(&primitives, masks.entries(), [64.0; 2]);
		let mut fragment = UiFragmentVm::new(&primitives, masks.entries(), None);

		let (mut inside, mut outside) = (0.0f32, 0.0f32);
		rasterize_primitive(&mut vertex, &mut fragment, 0, |[x, _], color| {
			if x < 32 {
				inside += color[3];
			} else {
				outside += color[3];
			}
		});
		assert!(inside > 50.0, "the visible half of the line must be drawn, found {inside}");
		assert_eq!(outside, 0.0);
	}

	/// The `UiRectFragment` struct describes one fragment of a rectangle layer for the BESL VM tests.
	struct UiRectFragment {
		color: [f32; 4],
		pixel_position: [f32; 2],
		rect: [f32; 4],
		corner_radius: f32,
		corner_exponent: f32,
		stroke_width: f32,
		mask: Option<UiClipMaskEntry>,
	}

	impl Default for UiRectFragment {
		/// Provides a centered fill invocation whose output should preserve the input color.
		fn default() -> Self {
			Self {
				color: [0.2, 0.4, 0.6, 0.8],
				pixel_position: [50.0, 50.0],
				rect: [0.0, 0.0, 100.0, 100.0],
				corner_radius: 12.0,
				corner_exponent: 2.0,
				stroke_width: 0.0,
				mask: None,
			}
		}
	}

	/// Executes the production UI fragment shader for one fragment of a rectangle layer.
	fn run_ui_rect_fragment_vm(values: UiRectFragment) -> [f32; 4] {
		let rectangle = UiPrimitive {
			bounds: [
				values.rect[0],
				values.rect[1],
				values.rect[0] + values.rect[2],
				values.rect[1] + values.rect[3],
			],
			color: values.color,
			a: values.rect,
			b: [values.corner_radius, values.corner_exponent, values.stroke_width, 0.0],
			kind: UI_KIND_RECT,
			mask: values.mask.is_some() as u32,
			..UiPrimitive::default()
		};
		let masks = [UiClipMaskEntry::NONE, values.mask.unwrap_or(UiClipMaskEntry::NONE)];
		UiFragmentVm::new(&[rectangle], &masks, None).run(
			UiVaryings {
				pixel_position: values.pixel_position,
				..UiVaryings::default()
			},
			&mut [],
		)
	}

	/// Verifies a centered fill fragment emits its unmodified layer color.
	#[test]
	fn ui_fragment_besl_vm_preserves_centered_fill_color() {
		let expected = UiRectFragment::default().color;
		assert_vec4_close(run_ui_rect_fragment_vm(UiRectFragment::default()), expected);
		// A square fill skips the shape math entirely and must agree.
		assert_vec4_close(
			run_ui_rect_fragment_vm(UiRectFragment {
				corner_radius: 0.0,
				..Default::default()
			}),
			expected,
		);
	}

	/// Verifies rounded-corner coverage rejects a fragment outside the rounded boundary.
	#[test]
	fn ui_fragment_besl_vm_rejects_rounded_corner_exterior() {
		for corner_exponent in [2.0, 4.0] {
			let output = run_ui_rect_fragment_vm(UiRectFragment {
				pixel_position: [0.0, 0.0],
				corner_radius: 20.0,
				corner_exponent,
				..Default::default()
			});

			assert!(
				output[3] < 0.001,
				"Expected rounded corner alpha near zero, found {}",
				output[3]
			);
		}
	}

	/// Verifies that circular corners, which skip the superellipse math, agree with it.
	#[test]
	fn ui_fragment_besl_vm_circular_corners_match_the_superellipse_field() {
		for pixel_position in [[3.5, 3.5], [5.5, 2.5], [6.5, 6.5], [1.5, 9.5]] {
			for stroke_width in [0.0, 2.0] {
				let coverage = |corner_exponent: f32| {
					run_ui_rect_fragment_vm(UiRectFragment {
						pixel_position,
						corner_radius: 20.0,
						corner_exponent,
						stroke_width,
						..Default::default()
					})[3]
				};
				// An exponent just past the shortcut's tolerance takes the general path on the same shape.
				assert!((coverage(2.0) - coverage(2.002)).abs() < 0.01);
			}
		}
	}

	/// Verifies stroke coverage removes fragments that lie inside the hollow center.
	#[test]
	fn ui_fragment_besl_vm_stroke_excludes_the_center() {
		for corner_radius in [12.0, 0.0] {
			let center = run_ui_rect_fragment_vm(UiRectFragment {
				stroke_width: 3.0,
				corner_radius,
				..Default::default()
			});
			let edge = run_ui_rect_fragment_vm(UiRectFragment {
				stroke_width: 3.0,
				corner_radius,
				pixel_position: [1.5, 50.0],
				..Default::default()
			});

			assert!(
				center[3] < 0.001,
				"Expected stroke center alpha near zero, found {}",
				center[3]
			);
			assert!(
				(edge[3] - 0.8).abs() < 0.001,
				"Expected a solid stroke edge, found {}",
				edge[3]
			);
		}
	}

	/// Verifies the feather mask suppresses fragments outside its clipped region.
	/// Verifies a turned square fill fades across its edge, which an unrotated one leaves to the rasterizer.
	#[test]
	fn ui_fragment_besl_vm_fades_the_edges_of_a_turned_square_fill() {
		let turned = UiClipMaskEntry {
			rotation: [0.8, 0.6, 0.0, 0.0],
			..UiClipMaskEntry::NONE
		};
		let alpha = |x: f32, mask: Option<UiClipMaskEntry>| {
			run_ui_rect_fragment_vm(UiRectFragment {
				pixel_position: [x, 50.0],
				corner_radius: 0.0,
				mask,
				..Default::default()
			})[3]
		};
		let inside = alpha(50.0, Some(turned));
		let default_left = UiRectFragment::default().rect[0];
		// On the edge half the pixel is covered, a pixel inside all of it, and a pixel outside none.
		assert!((alpha(default_left, Some(turned)) - inside * 0.5).abs() < 0.001);
		assert!((alpha(default_left + 1.0, Some(turned)) - inside).abs() < 0.001);
		assert!(alpha(default_left - 1.0, Some(turned)).abs() < 0.001);
		assert!((alpha(default_left, None) - inside).abs() < 0.001);
	}

	#[test]
	fn ui_fragment_besl_vm_clip_mask_suppresses_outside_pixels() {
		let mask = UiClipMaskEntry {
			rect: [25.0, 25.0, 50.0, 50.0],
			edges: [5.0; 4],
			..UiClipMaskEntry::NONE
		};
		let outside = run_ui_rect_fragment_vm(UiRectFragment {
			pixel_position: [10.0, 10.0],
			mask: Some(mask),
			..Default::default()
		});
		let feathered = run_ui_rect_fragment_vm(UiRectFragment {
			pixel_position: [27.5, 50.0],
			mask: Some(mask),
			..Default::default()
		});
		let inside = run_ui_rect_fragment_vm(UiRectFragment {
			mask: Some(mask),
			..Default::default()
		});

		assert!(
			outside[3] < 0.001,
			"Expected feathered pixel alpha near zero, found {}",
			outside[3]
		);
		// Halfway through a five pixel feather is half coverage.
		assert!(
			(feathered[3] - 0.4).abs() < 0.001,
			"Expected half coverage, found {}",
			feathered[3]
		);
		assert_vec4_close(inside, UiRectFragment::default().color);
	}

	#[test]
	fn skips_zero_alpha_text_before_rasterization() {
		assert!(!should_rasterize_text(&UiTextDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [32.0, 16.0],
			clip: None,
			clip_mask: None,
			color: RGBA::new(1.0, 1.0, 1.0, 0.0),
			font_size: 16.0,
			text: "Hidden".to_string(),
		}));
		assert!(should_rasterize_text(&UiTextDrawElement {
			depth: 0,
			order: 0,
			position: [0.0, 0.0],
			size: [32.0, 16.0],
			clip: None,
			clip_mask: None,
			color: RGBA::new(1.0, 1.0, 1.0, 1.0),
			font_size: 16.0,
			text: "Visible".to_string(),
		}));
	}

	#[test]
	fn update_from_render_clears_removed_text_entries() {
		let frame_allocator = bumpalo::Bump::new();
		let mut draw_list = UiDrawList::default();

		let mut text_engine = Engine::new();
		text_engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("label").text(Text::new("Option"));
			})
		});
		let mut text_snapshot = text_engine.evaluate(Size::new(100, 100), &frame_allocator);
		let text_render = text_engine.render(&mut text_snapshot);
		update_from_render(text_render, &mut draw_list);

		assert_eq!(draw_list.texts.len(), 1);

		let mut no_text_engine = Engine::new();
		no_text_engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(Container::default());
			})
		});
		let mut no_text_snapshot = no_text_engine.evaluate(Size::new(100, 100), &frame_allocator);
		let no_text_render = no_text_engine.render(&mut no_text_snapshot);
		update_from_render(no_text_render, &mut draw_list);

		assert!(draw_list.texts.is_empty());
	}

	#[test]
	fn update_from_render_clears_removed_image_entries() {
		let frame_allocator = bumpalo::Bump::new();
		let mut draw_list = UiDrawList::default();

		let mut image_engine = Engine::new();
		image_engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("preview").image(Image::from_rgba(2, 2, image_pixels(2, 2)));
			})
		});
		let mut image_snapshot = image_engine.evaluate(Size::new(100, 100), &frame_allocator);
		let image_render = image_engine.render(&mut image_snapshot);
		update_from_render(image_render, &mut draw_list);

		assert_eq!(draw_list.images.len(), 1);

		let mut no_image_engine = Engine::new();
		no_image_engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(Container::default());
			})
		});
		let mut no_image_snapshot = no_image_engine.evaluate(Size::new(100, 100), &frame_allocator);
		let no_image_render = no_image_engine.render(&mut no_image_snapshot);
		update_from_render(no_image_render, &mut draw_list);

		assert!(draw_list.images.is_empty());
	}

	#[test]
	fn draw_list_multiplies_effective_opacity_into_layers_and_text() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(
					Container::default().opacity(0.5).style(
						ConcreteStyle::new()
							.layer(ConcreteLayer::default().color(RGBA::new(1.0, 0.0, 0.0, 0.8).into()))
							.layer(
								ConcreteLayer::default()
									.color(RGBA::new(0.0, 1.0, 0.0, 0.6).into())
									.stroke(2.0),
							),
					),
				);
				frame
					.element("label")
					.text(Text::new("Visible").style(ConcreteLayer::default().color(RGBA::new(1.0, 1.0, 1.0, 0.4).into())));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let mut draw_list = UiDrawList::default();
		update_from_render(render, &mut draw_list);

		assert_eq!(draw_list.elements[0].color[3], 0.4);
		assert_eq!(draw_list.elements[1].color[3], 0.3);
		assert_eq!(draw_list.texts[0].color, RGBA::new(1.0, 1.0, 1.0, 0.2));
	}

	#[test]
	fn draw_list_multiplies_effective_opacity_into_images() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default().opacity(0.5));
				frame
					.element("preview")
					.image(Image::from_rgba(4, 4, image_pixels(4, 4)).opacity(0.4));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let mut draw_list = UiDrawList::default();
		update_from_render(render, &mut draw_list);

		assert_eq!(draw_list.images.len(), 1);
		assert!((draw_list.images[0].opacity - 0.2).abs() < 0.0001);
	}

	#[test]
	fn image_trims_its_texture_rectangle_to_the_clip() {
		let frame_allocator = bumpalo::Bump::new();
		let draw_list = UiDrawList {
			layout_size: [100.0, 100.0],
			images: vec![UiImageDrawElement {
				depth: 7,
				order: 0,
				image_id: 1,
				version: 0,
				source_width: 10,
				source_height: 10,
				pixels: image_pixels(10, 10).into(),
				position: [10.0, 20.0],
				size: [40.0, 20.0],
				clip: Some(DrawClip {
					position: [20.0, 25.0],
					size: [20.0, 10.0],
				}),
				clip_mask: None,
				opacity: 0.5,
			}],
			..UiDrawList::default()
		};

		let (primitives, output, _) = build(&draw_list, Extent::rectangle(100, 100), &frame_allocator);

		assert_eq!(primitives.len(), 1);
		assert_eq!(primitives[0].kind, UI_KIND_IMAGE);
		assert_eq!(primitives[0].bounds, [20.0, 25.0, 40.0, 35.0]);
		assert_vec4_close(primitives[0].a, [0.25, 0.25, 0.75, 0.75]);
		// The shader multiplies the texel's alpha by the color's.
		assert_eq!(primitives[0].color, [1.0, 1.0, 1.0, 0.5]);
		assert_eq!(output.images.as_slice(), [(1, 0)]);
	}

	#[test]
	fn image_skips_invalid_or_transparent_images() {
		let frame_allocator = bumpalo::Bump::new();
		let hidden = UiImageDrawElement {
			depth: 0,
			order: 0,
			image_id: 1,
			version: 0,
			source_width: 2,
			source_height: 2,
			pixels: image_pixels(2, 2).into(),
			position: [0.0, 0.0],
			size: [20.0, 20.0],
			clip: None,
			clip_mask: None,
			opacity: 0.0,
		};

		assert!(!should_draw_image(&hidden));

		let draw_list = UiDrawList {
			layout_size: [100.0, 100.0],
			images: vec![hidden],
			..UiDrawList::default()
		};
		let (primitives, output, _) = build(&draw_list, Extent::rectangle(100, 100), &frame_allocator);

		assert!(primitives.is_empty());
		assert!(output.images.is_empty());
		assert_eq!(output.steps.as_slice(), [UiStep::Draw { first: 1, count: 0 }]);
	}
}
