//! Reusable construction for BESL tile render passes that bind their resources and outputs by name.
//!
//! A tile shader writes its results as `output` declarations. Where the platform runs tile functions, the pass records
//! a tile pass whose color attachments are the outputs, so they need render-target usage and stay losslessly
//! compressed. Elsewhere the shader was baked as a compute shader and the outputs are storage images. Create each output
//! with [`output_uses`] among its uses, and the pass records whichever form the shader was baked to.

use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundTilePipelineMode as _, CommandBufferRecording as _,
	CommonCommandBufferMode as _,
};
use utils::Extent;

use super::{Adoption, RenderPassBuilder, RenderPassReturn, allocate_render_command};
use crate::rendering::Sink;
use crate::rendering::pipeline_compilation::{NativeTile, TilePipelineRequest};
pub use crate::rendering::set_layout::Resource;
use crate::rendering::set_layout::{PipelineSets, create_sets};

/// The most outputs a tile shader writes, the color attachments of one render pass.
const MAX_OUTPUTS: usize = resource_management::resources::material::TileInterface::MAX_OUTPUTS;

/// Returns the uses a tile pass's output needs on `context`: render-target usage where the pass runs as a tile
/// function, and storage usage where it runs as a compute shader.
///
/// Add them to every image a tile pass writes, next to the uses later passes need, such as [`ghi::Uses::Image`] to
/// sample it.
pub fn output_uses(context: &impl ghi::context::Context) -> ghi::Uses {
	if context.supports_tile_functions() {
		ghi::Uses::RenderTarget
	} else {
		ghi::Uses::Storage
	}
}

/// The `Descriptor` struct identifies one tile pipeline, the formats of the outputs its passes bind, and its command
/// label.
pub struct Descriptor<'a> {
	label: &'static str,
	pipeline_id: &'a str,
	outputs: &'a [ghi::Formats],
}

impl<'a> Descriptor<'a> {
	/// Creates a descriptor for one baked tile pipeline resource whose passes bind outputs of `outputs` formats, in the
	/// shader's location order.
	pub fn new(label: &'static str, pipeline_id: &'a str, outputs: &'a [ghi::Formats]) -> Self {
		Self {
			label,
			pipeline_id,
			outputs,
		}
	}
}

/// The `Output` struct binds an image to one `output` declaration of a tile shader, by its BESL name.
#[derive(Clone, Copy)]
pub struct Output {
	name: &'static str,
	target: ghi::ImageOrSwapchain,
}

impl Output {
	/// Binds `target` to the output named `name`. Create it with [`output_uses`] among its uses.
	pub fn new(name: &'static str, target: impl Into<ghi::ImageOrSwapchain>) -> Self {
		Self {
			name,
			target: target.into(),
		}
	}
}

/// The `Pipeline` struct provides reusable tile state to sink-specific retained resource sets.
#[derive(Clone)]
pub struct Pipeline {
	reference: crate::rendering::PipelineRef,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	label: &'static str,
}

impl Pipeline {
	/// Requests a baked tile pipeline for the descriptor's output formats and defers descriptor adoption until
	/// compilation is published.
	pub fn compile(render_pass_builder: &mut RenderPassBuilder<'_>, descriptor: Descriptor<'_>) -> Self {
		let Descriptor {
			label,
			pipeline_id,
			outputs,
		} = descriptor;
		let pipeline_manager = render_pass_builder.pipeline_manager().clone();
		let reference = pipeline_manager.request_tile_pipeline(TilePipelineRequest::new(pipeline_id, outputs.to_vec()));

		Self {
			reference,
			pipeline_manager,
			label,
		}
	}

	/// Freezes the named resources and outputs for one sink.
	///
	/// The pass validates them against the pipeline's descriptor set layouts and outputs once the pipeline is
	/// published, in [`Pass::ready`], and logs an error there when they do not match.
	pub fn bind(&self, descriptor_set_name: &'static str, resources: &[Resource], outputs: &[Output]) -> Pass {
		Pass {
			pipeline: self.reference,
			pipeline_manager: self.pipeline_manager.clone(),
			descriptor_set_name,
			resources: resources.to_vec(),
			outputs: outputs.to_vec(),
			adoption: Adoption::default(),
			label: self.label,
		}
	}
}

/// The `Pass` struct provides one sink with a validated tile pipeline, its descriptor sets, and its outputs.
#[derive(Clone)]
pub struct Pass {
	pipeline: crate::rendering::PipelineRef,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	descriptor_set_name: &'static str,
	resources: Vec<Resource>,
	outputs: Vec<Output>,
	adoption: Adoption<ReadyPass>,
	label: &'static str,
}

impl Pass {
	/// Adopts a published pipeline and creates this sink's descriptor sets once, one per layout the shader uses.
	pub fn ready(&mut self, frame: &mut ghi::implementation::Frame) -> Option<ReadyPass> {
		self.adoption.get(&self.pipeline_manager, self.pipeline, self.label, || {
			// Borrow the reflected layouts under one read view instead of copying them.
			let pipelines = self.pipeline_manager.compute_pipelines();
			let compiled = pipelines.tile(self.pipeline).expect(
				"Published tile pipeline metadata is missing. The most likely cause is that a compute or raster pipeline was supplied to a tile pass.",
			);
			adopt(frame, compiled, self.descriptor_set_name, &self.resources, &self.outputs, self.label)
		})
	}

	/// Allocates a frame command that records this tile pass over the sink extent.
	pub fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pass = self.ready(frame)?;
		let extent = sink.extent();

		Some(allocate_render_command(frame_allocator, move |command_buffer| {
			command_buffer.region(
				|region_label| region_label.write_str(pass.label),
				|command_buffer| pass.record(command_buffer, extent),
			);
		}))
	}
}

/// Orders the bound `outputs` by location and creates the descriptor sets of `resources`, with the outputs among them
/// where the shader runs as compute.
fn adopt(
	frame: &mut ghi::implementation::Frame,
	compiled: &crate::rendering::pipeline_compilation::TilePipeline,
	descriptor_set_name: &'static str,
	resources: &[Resource],
	bound_outputs: &[Output],
	label: &'static str,
) -> Result<ReadyPass, String> {
	let mut outputs = [None; MAX_OUTPUTS];
	for (index, output) in compiled.interface.outputs.iter().enumerate() {
		let bound = bound_outputs.iter().find(|bound| bound.name == output.name).ok_or_else(|| {
			format!(
				"Missing an image for output `{}`. The most likely cause is an output the shader declares that the pass doesn't bind. Bind an image to it with `simple_tile::Output::new`.",
				output.name
			)
		})?;
		outputs[index] = Some(bound.target);
	}
	if let Some(extra) = bound_outputs
		.iter()
		.find(|bound| !compiled.interface.outputs.iter().any(|output| output.name == bound.name))
	{
		return Err(format!(
			"The pass binds output `{}`, which the shader doesn't declare. The most likely cause is a renamed or removed `output` declaration. Bind only the outputs the shader declares.",
			extra.name
		));
	}

	let descriptor_sets = match compiled.native {
		NativeTile::TileFunction => create_sets(frame, descriptor_set_name, &compiled.sets, resources)?,
		// The compute form takes its outputs as storage images, bound by the outputs' names.
		NativeTile::Compute { .. } => {
			let mut resources = resources.to_vec();
			resources.extend(bound_outputs.iter().map(|output| Resource::image(output.name, output.target)));
			create_sets(frame, descriptor_set_name, &compiled.sets, &resources)?
		}
	};
	Ok(ReadyPass {
		pipeline: compiled.handle,
		descriptor_sets,
		label,
		tile: compiled.interface.size,
		native: compiled.native,
		outputs,
	})
}

/// The `ReadyPass` struct provides immutable recording state after pipeline publication.
#[derive(Clone, Copy)]
pub struct ReadyPass {
	pipeline: ghi::PipelineHandle,
	descriptor_sets: PipelineSets,
	label: &'static str,
	/// Pixels along each edge of the shader's tile.
	tile: u32,
	native: NativeTile,
	/// The outputs in location order, filled from the front.
	outputs: [Option<ghi::ImageOrSwapchain>; MAX_OUTPUTS],
}

impl ReadyPass {
	/// Records this pass over `extent`, the extent of its outputs, without allocating a render-command closure.
	///
	/// A tile shader writes every pixel of its outputs, so their earlier contents are discarded.
	pub fn record(&self, command_buffer: &mut ghi::implementation::CommandBufferRecording, extent: Extent) {
		match self.native {
			NativeTile::TileFunction => {
				let attachments = self
					.outputs
					.iter()
					.flatten()
					.map(|target| {
						ghi::AttachmentInformation::new(
							*target,
							ghi::Layouts::RenderTarget,
							ghi::LoadOp::Discard,
							ghi::StoreOp::Store,
						)
					})
					.collect::<smallvec::SmallVec<[_; MAX_OUTPUTS]>>();
				let pass = command_buffer.start_tile_pass(self.pipeline, extent, &attachments);
				pass.bind_descriptor_sets(&self.descriptor_sets.handles());
				pass.dispatch_tiles();
			}
			NativeTile::Compute { workgroup } => {
				// One workgroup covers one tile, so the dispatch counts its threads as if every tile were whole.
				let threads = Extent::rectangle(
					extent.width().div_ceil(self.tile) * workgroup.width(),
					extent.height().div_ceil(self.tile) * workgroup.height(),
				);
				let command_buffer = command_buffer.bind_compute_pipeline(self.pipeline);
				command_buffer.bind_descriptor_sets(&self.descriptor_sets.handles());
				command_buffer.dispatch(ghi::DispatchExtent::new(threads, workgroup));
			}
		}
	}
}
