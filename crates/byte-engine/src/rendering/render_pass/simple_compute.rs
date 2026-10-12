//! Reusable construction for BESL compute render passes that bind their resources by name.

use ghi::command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _};
use utils::Extent;

use super::{RenderPassBuilder, RenderPassReturn, allocate_render_command};
use crate::rendering::Sink;
#[cfg(test)]
use crate::rendering::common_shader_generator::common_shader_scope;
pub use crate::rendering::set_layout::Resource;
use crate::rendering::set_layout::{PipelineSets, create_sets};

/// The `Descriptor` struct identifies one compute pipeline and its command label.
pub struct Descriptor<'a> {
	label: &'static str,
	pipeline_id: &'a str,
}

impl<'a> Descriptor<'a> {
	/// Creates a descriptor for one baked pipeline resource and its human-readable command label.
	pub fn new(label: &'static str, pipeline_id: &'a str) -> Self {
		Self { label, pipeline_id }
	}
}

/// Links one canonical BESL asset and returns its program for semantic tests.
///
/// The program owns every function, so keep it alive while compiling or running its `main`.
#[cfg(test)]
pub fn compile_test_program(source: &str) -> besl::NodeReference {
	let mut root = besl::parse(source).expect(
		"Failed to parse a canonical render-pass BESL asset. The most likely cause is invalid source syntax or descriptor declarations.",
	);
	root.add(vec![common_shader_scope()]);
	let program = besl::lex(root).expect(
		"Failed to link a canonical render-pass BESL asset. The most likely cause is an unresolved shared helper or descriptor declaration.",
	);
	program.get_main().expect(
		"Canonical render-pass entry point is missing. The most likely cause is that the BESL asset does not define `main`.",
	);
	program
}

/// The `Pipeline` struct provides reusable compute state to sink-specific retained resource sets.
#[derive(Clone)]
pub struct Pipeline {
	reference: crate::rendering::PipelineRef,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	label: &'static str,
}

impl Pipeline {
	/// Requests a baked pipeline and defers descriptor adoption until compilation is published.
	pub fn compile(render_pass_builder: &mut RenderPassBuilder<'_>, descriptor: Descriptor<'_>) -> Self {
		let Descriptor { label, pipeline_id } = descriptor;
		let pipeline_manager = render_pass_builder.pipeline_manager().clone();
		let reference = pipeline_manager.request_pipeline(pipeline_id);

		Self {
			reference,
			pipeline_manager,
			label,
		}
	}

	/// Freezes the named resources for one sink.
	///
	/// The pass validates them against the pipeline's descriptor set layouts and creates its descriptor sets once the
	/// pipeline is published, in [`Pass::ready`], and logs an error there when they do not match.
	pub fn bind(&self, descriptor_set_name: &'static str, resources: &[Resource]) -> Pass {
		Pass {
			pipeline: self.reference,
			pipeline_manager: self.pipeline_manager.clone(),
			descriptor_set_name,
			resources: resources.to_vec(),
			ready: None,
			revision: 0,
			failed: false,
			label: self.label,
		}
	}
}

/// The `Pass` struct provides one sink with a validated pipeline and descriptor sets for a single compute dispatch.
#[derive(Clone)]
pub struct Pass {
	pipeline: crate::rendering::PipelineRef,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	descriptor_set_name: &'static str,
	resources: Vec<Resource>,
	ready: Option<ReadyPass>,
	revision: u64,
	failed: bool,
	label: &'static str,
}

impl Pass {
	/// Adopts a published pipeline and creates this sink's descriptor sets once, one per layout the shader uses.
	pub fn ready(&mut self, frame: &mut ghi::implementation::Frame) -> Option<ReadyPass> {
		let revision = self.pipeline_manager.revision(self.pipeline);
		if self.revision == revision
			&& let Some(ready) = self.ready
		{
			return Some(ready);
		}
		if self.revision != revision {
			self.ready = None;
			self.failed = false;
		}
		if self.failed {
			return None;
		}
		match self.pipeline_manager.get(self.pipeline) {
			crate::rendering::PipelineState::Pending => return None,
			crate::rendering::PipelineState::Failed => {
				log::error!(
					"Simple compute pipeline is unavailable. The most likely cause is that its pipeline asset or shader dependency failed to bake or compile."
				);
				self.failed = true;
				return None;
			}
			crate::rendering::PipelineState::Ready(_) => {}
		}
		// Borrow the reflected layouts under one read view instead of copying them.
		let pipelines = self.pipeline_manager.compute_pipelines();
		let compiled = pipelines.get(self.pipeline).expect(
			"Published compute pipeline metadata is missing. The most likely cause is that a raster pipeline was supplied to a simple compute pass.",
		);
		let descriptor_sets = match create_sets(frame, self.descriptor_set_name, &compiled.sets, &self.resources) {
			Ok(descriptor_sets) => descriptor_sets,
			Err(error) => {
				log::error!("Simple compute pipeline adoption failed: {error}");
				self.failed = true;
				return None;
			}
		};
		let ready = ReadyPass {
			pipeline: compiled.handle,
			descriptor_sets,
			label: self.label,
			workgroup: compiled.workgroup,
		};
		drop(pipelines);
		self.ready = Some(ready);
		self.revision = revision;
		Some(ready)
	}

	/// Allocates a frame command that records this compute pass for the sink extent.
	pub fn prepare<'a>(
		&mut self,
		_frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pass = self.ready(_frame)?;
		let extent = sink.extent();

		Some(allocate_render_command(frame_allocator, move |command_buffer| {
			command_buffer.region(
				|region_label| region_label.write_str(pass.label),
				|command_buffer| pass.record(command_buffer, extent),
			);
		}))
	}
}

/// The `ReadyPass` struct provides immutable recording state after pipeline publication.
#[derive(Clone, Copy)]
pub struct ReadyPass {
	pipeline: ghi::PipelineHandle,
	descriptor_sets: PipelineSets,
	label: &'static str,
	workgroup: Extent,
}

impl ReadyPass {
	/// Records this pass without allocating a render-command closure.
	pub fn record(&self, command_buffer: &mut ghi::implementation::CommandBufferRecording, extent: Extent) {
		let command_buffer = command_buffer.bind_compute_pipeline(self.pipeline);
		command_buffer.bind_descriptor_sets(&self.descriptor_sets.handles());
		command_buffer.dispatch(ghi::DispatchExtent::new(extent, self.workgroup));
	}
}
