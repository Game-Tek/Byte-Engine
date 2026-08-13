//! Reusable construction for flat-resource BESL compute render passes.

use std::sync::Arc;

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
};
use resource_management::shader::besl::evaluation::{BindingKind, BindingUsage, TextureView};
use smallvec::SmallVec;
use utils::Extent;

use super::{allocate_render_command, RenderPassBuilder, RenderPassReturn};
use crate::rendering::{common_shader_generator::CommonShaderScope, Sink};

/// The `Descriptor` struct identifies one single-set compute pipeline and its command label.
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

/// Compiles one canonical BESL asset and returns its compute entry point for semantic tests.
#[cfg(test)]
pub fn compile_test_program(source: &str) -> besl::NodeReference {
	let mut root = besl::parse(source).expect(
		"Failed to parse a canonical render-pass BESL asset. The most likely cause is invalid source syntax or descriptor declarations.",
	);
	root.add(vec![CommonShaderScope::new()]);
	let program = besl::lex(root).expect(
		"Failed to link a canonical render-pass BESL asset. The most likely cause is an unresolved shared helper or descriptor declaration.",
	);
	program.get_main().expect(
		"Canonical render-pass entry point is missing. The most likely cause is that the BESL asset does not define `main`.",
	)
}

/// The `Pipeline` struct provides reusable compute state to sink-specific retained resource sets.
#[derive(Clone)]
pub struct Pipeline {
	reference: crate::rendering::PipelineRef,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	label: &'static str,
	shared_layout: Option<crate::rendering::PipelineRef>,
}

impl Pipeline {
	/// Requests a baked pipeline and defers descriptor adoption until compilation is published.
	pub fn compile(render_pass_builder: &mut RenderPassBuilder<'_>, descriptor: Descriptor<'_>) -> Result<Self, String> {
		Self::build(render_pass_builder, descriptor, None)
	}

	/// Requests another baked pipeline against this pipeline's validated binding layout.
	pub fn compile_variant(
		&self,
		render_pass_builder: &mut RenderPassBuilder<'_>,
		descriptor: Descriptor<'_>,
	) -> Result<Self, String> {
		Self::build(render_pass_builder, descriptor, Some(self))
	}

	/// Builds a pipeline while optionally reusing a schema already validated by a sibling pipeline.
	fn build(
		render_pass_builder: &mut RenderPassBuilder<'_>,
		descriptor: Descriptor<'_>,
		shared_layout: Option<&Self>,
	) -> Result<Self, String> {
		let Descriptor { label, pipeline_id } = descriptor;
		let pipeline_manager = render_pass_builder.pipeline_manager().clone();
		let reference = pipeline_manager.request_pipeline(pipeline_id);

		Ok(Self {
			reference,
			pipeline_manager,
			label,
			shared_layout: shared_layout.map(|pipeline| pipeline.reference),
		})
	}

	/// Validates named resources, creates the descriptor set, and freezes binding order for frame recording.
	pub fn bind(
		&self,
		render_pass_builder: &mut RenderPassBuilder<'_>,
		descriptor_set_name: &'static str,
		resources: &[Resource],
	) -> Result<Pass, String> {
		Ok(Pass {
			pipeline: self.reference,
			pipeline_manager: self.pipeline_manager.clone(),
			shared_layout: self.shared_layout,
			descriptor_set_name,
			resources: resources.to_vec(),
			ready: None,
			revision: 0,
			failed: false,
			label: self.label,
		})
	}
}

/// The `Resource` enum names one concrete resource for a reachable or planned BESL binding.
#[derive(Clone, Copy)]
pub enum Resource {
	Buffer(&'static str, ghi::BaseBufferHandle),
	PlannedBuffer(&'static str, ghi::BaseBufferHandle),
	Image(&'static str, ghi::BaseImageHandle),
	CombinedImageSampler(&'static str, ghi::BaseImageHandle, ghi::SamplerHandle, ghi::Layouts),
	Swapchain(&'static str, ghi::SwapchainHandle),
}

impl Resource {
	pub fn buffer(name: &'static str, buffer: impl Into<ghi::BaseBufferHandle>) -> Self {
		Self::Buffer(name, buffer.into())
	}

	/// Keeps a buffer ready for a BESL binding that is intentionally not reachable yet.
	pub fn planned_buffer(name: &'static str, buffer: impl Into<ghi::BaseBufferHandle>) -> Self {
		Self::PlannedBuffer(name, buffer.into())
	}

	pub fn image(name: &'static str, image: impl Into<ghi::BaseImageHandle>) -> Self {
		Self::Image(name, image.into())
	}

	pub fn combined_image_sampler(
		name: &'static str,
		image: impl Into<ghi::BaseImageHandle>,
		sampler: ghi::SamplerHandle,
		layout: ghi::Layouts,
	) -> Self {
		Self::CombinedImageSampler(name, image.into(), sampler, layout)
	}

	pub fn swapchain(name: &'static str, swapchain: ghi::SwapchainHandle) -> Self {
		Self::Swapchain(name, swapchain)
	}

	fn name(&self) -> &str {
		match self {
			Self::Buffer(name, ..)
			| Self::PlannedBuffer(name, ..)
			| Self::Image(name, ..)
			| Self::CombinedImageSampler(name, ..)
			| Self::Swapchain(name, ..) => name,
		}
	}

	fn matches(&self, binding: BindingKind) -> bool {
		matches!(
			(binding, self),
			(BindingKind::StorageBuffer, Self::Buffer(..) | Self::PlannedBuffer(..))
				| (BindingKind::StorageImage, Self::Image(..) | Self::Swapchain(..))
				| (BindingKind::CombinedImageSampler { .. }, Self::CombinedImageSampler(..))
		)
	}

	fn descriptor_write(&self, set: ghi::DescriptorSetHandle, slot: ghi::ResourceSlot) -> ghi::DescriptorWrite {
		match *self {
			Self::Buffer(_, buffer) | Self::PlannedBuffer(_, buffer) => ghi::DescriptorWrite::buffer(set, slot, buffer),
			Self::Image(_, image) => ghi::DescriptorWrite::image(set, slot, image, ghi::Layouts::General),
			Self::CombinedImageSampler(_, image, sampler, layout) => {
				ghi::DescriptorWrite::combined_image_sampler(set, slot, image, sampler, layout)
			}
			Self::Swapchain(_, swapchain) => ghi::DescriptorWrite::swapchain(set, slot, swapchain),
		}
	}

	fn is_planned(&self) -> bool {
		matches!(self, Self::PlannedBuffer(..))
	}
}

/// The `Pass` struct provides one sink with a validated pipeline and descriptor set for a single compute dispatch.
#[derive(Clone)]
pub struct Pass {
	pipeline: crate::rendering::PipelineRef,
	pipeline_manager: crate::rendering::PipelineManagerClient,
	shared_layout: Option<crate::rendering::PipelineRef>,
	descriptor_set_name: &'static str,
	resources: Vec<Resource>,
	ready: Option<ReadyPass>,
	revision: u64,
	failed: bool,
	label: &'static str,
}

impl Pass {
	/// Adopts a published pipeline and creates this sink's descriptor set once.
	pub fn ready(&mut self, frame: &mut ghi::implementation::Frame) -> Option<ReadyPass> {
		use ghi::context::Context as _;
		use ghi::context::ContextCreate as _;

		let revision = self.pipeline_manager.revision(self.pipeline);
		if self.revision == revision {
			if let Some(ready) = self.ready {
				return Some(ready);
			}
		}
		if self.revision != revision {
			self.ready = None;
			self.failed = false;
		}
		if self.failed {
			return None;
		}
		let compiled = match self.pipeline_manager.get(self.pipeline) {
			crate::rendering::PipelineState::Pending => return None,
			crate::rendering::PipelineState::Failed => {
				log::error!(
					"Simple compute pipeline is unavailable. The most likely cause is that its pipeline asset or shader dependency failed to bake or compile."
				);
				self.failed = true;
				return None;
			}
			crate::rendering::PipelineState::Ready(_) => self.pipeline_manager.compute_pipeline(self.pipeline).expect(
				"Published compute pipeline metadata is missing. The most likely cause is that a raster pipeline was supplied to a simple compute pass.",
			),
		};
		if let Err(error) = validate_binding_schema(&compiled.bindings) {
			log::error!("Simple compute pipeline adoption failed: {error}");
			self.failed = true;
			return None;
		}
		if let Some(shared) = self.shared_layout {
			let shared = self.pipeline_manager.compute_pipeline(shared)?;
			if let Err(error) = validate_shared_schema(&shared.bindings, &compiled.bindings) {
				log::error!("Simple compute pipeline adoption failed: {error}");
				self.failed = true;
				return None;
			}
		}
		if let Err(error) = validate_resources(&compiled.bindings, &self.resources) {
			log::error!("Simple compute pipeline adoption failed: {error}");
			self.failed = true;
			return None;
		}
		let descriptor_set = frame.create_descriptor_set(Some(self.descriptor_set_name));
		let mut writes = SmallVec::<[ghi::DescriptorWrite; 8]>::with_capacity(compiled.bindings.len());
		for binding in compiled.bindings.iter() {
			let resource = self
				.resources
				.iter()
				.find(|resource| resource.name() == binding.name)
				.unwrap();
			writes.push(resource.descriptor_write(descriptor_set, ghi::ResourceSlot::new(binding.slot)));
		}
		frame.write(&writes);
		let ready = ReadyPass {
			pipeline: compiled.handle,
			descriptor_set,
			label: self.label,
			workgroup: compiled.workgroup,
		};
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

		Some(allocate_render_command(frame_allocator, move |command_buffer, _| {
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
	descriptor_set: ghi::DescriptorSetHandle,
	label: &'static str,
	workgroup: Extent,
}

impl ReadyPass {
	/// Records this pass without allocating a render-command closure.
	pub fn record(&self, command_buffer: &mut ghi::implementation::CommandBufferRecording, extent: Extent) {
		let command_buffer = command_buffer.bind_compute_pipeline(self.pipeline);
		command_buffer.bind_descriptor_sets(&[self.descriptor_set]);
		command_buffer.dispatch(ghi::DispatchExtent::new(extent, self.workgroup));
	}
}

fn texture_view(view: TextureView) -> ghi::TextureViewTypes {
	match view {
		TextureView::Texture2D => ghi::TextureViewTypes::Texture2D,
		TextureView::Texture2DArray => ghi::TextureViewTypes::Texture2DArray,
		TextureView::TextureCube => ghi::TextureViewTypes::TextureCube,
		TextureView::TextureCubeArray => ghi::TextureViewTypes::TextureCubeArray,
		TextureView::Texture3D => ghi::TextureViewTypes::Texture3D,
	}
}

fn validate_binding_schema(bindings: &[BindingUsage]) -> Result<(), &'static str> {
	for (index, binding) in bindings.iter().enumerate() {
		if binding.count != 1 {
			return Err("Descriptor arrays are unsupported in simple compute passes. The most likely cause is that the BESL shader requires multiple resources for one binding.");
		}
		if !binding.read && !binding.write {
			return Err("Inaccessible binding in simple compute pass. The most likely cause is that a BESL binding declares neither read nor write access.");
		}
		if matches!(binding.kind, BindingKind::CombinedImageSampler { .. }) && (!binding.read || binding.write) {
			return Err("Sampled texture access is invalid in a simple compute pass. The most likely cause is that a combined image sampler was declared writable.");
		}
		if bindings[..index].iter().any(|previous| previous.name == binding.name) {
			return Err("Duplicate BESL binding name. The most likely cause is that two descriptor slots use the same symbol.");
		}
	}
	Ok(())
}

/// Ensures a shader variant cannot silently reinterpret its sibling's descriptor layout.
fn validate_shared_schema(schema: &[BindingUsage], bindings: &[BindingUsage]) -> Result<(), &'static str> {
	if bindings.iter().all(|binding| schema.contains(binding)) {
		Ok(())
	} else {
		Err("Compute pipeline variant has an incompatible binding schema. The most likely cause is that a sibling BESL shader changed a shared descriptor declaration.")
	}
}

fn validate_resources(bindings: &[BindingUsage], resources: &[Resource]) -> Result<(), String> {
	if let Some(resource) = resources
		.iter()
		.find(|resource| !resource.is_planned() && !bindings.iter().any(|binding| binding.name == resource.name()))
	{
		return Err(format!(
			"Unknown compute resource `{}`. The most likely cause is that the resource name does not match a reachable BESL binding.",
			resource.name()
		));
	}
	for binding in bindings {
		let mut matches = resources.iter().filter(|resource| resource.name() == binding.name);
		let resource = matches.next().ok_or_else(|| {
			format!(
				"Missing compute resource `{}`. The most likely cause is that the caller did not bind every reachable BESL descriptor.",
				binding.name
			)
		})?;
		if matches.next().is_some() {
			return Err(format!(
				"Duplicate compute resource `{}`. The most likely cause is that the same BESL symbol was bound twice.",
				binding.name
			));
		}
		if !resource.matches(binding.kind) {
			return Err(format!(
				"Compute resource `{}` has the wrong type. The most likely cause is that an image, sampler, or buffer was bound to an incompatible BESL binding.",
				binding.name
			));
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use resource_management::shader::besl::evaluation::{BindingKind, BindingUsage, TextureView};

	use super::{texture_view, validate_binding_schema, validate_resources, validate_shared_schema, Resource};

	fn binding(name: &str, kind: BindingKind, slot: u32, read: bool, write: bool) -> BindingUsage {
		BindingUsage {
			name: name.to_string(),
			kind,
			count: 1,
			slot,
			buffer_stride: matches!(kind, BindingKind::StorageBuffer).then_some(4),
			read,
			write,
		}
	}

	#[test]
	fn named_resources_validate_complete_type_safe_order_independent_sets() {
		let bindings = [
			binding("parameters", BindingKind::StorageBuffer, 0, true, false),
			binding(
				"source",
				BindingKind::CombinedImageSampler {
					view: TextureView::Texture2D,
				},
				1,
				true,
				false,
			),
			binding("result", BindingKind::StorageImage, 2, false, true),
		];
		let mut device = ghi::debug::Device::new();
		let buffer = device.create_acceleration_structure_instance_buffer(None, 1);
		let image = device.build_dynamic_image(ghi::image::Builder::new(
			ghi::Formats::RGBA16F,
			ghi::Uses::Image | ghi::Uses::Storage,
		));
		let sampler = device.build_sampler(ghi::sampler::Builder::new());
		let resources = [
			Resource::image("result", image),
			Resource::buffer("parameters", buffer),
			Resource::combined_image_sampler("source", image, sampler, ghi::Layouts::Read),
		];
		validate_resources(&bindings, &resources).expect("complete resources should validate");
		assert!(bindings[0].read && !bindings[0].write);
		validate_shared_schema(&bindings, &[bindings[0].clone(), bindings[2].clone()])
			.expect("compatible shared bindings should validate");
		let incompatible = BindingUsage {
			kind: BindingKind::StorageImage,
			..bindings[0].clone()
		};
		assert!(validate_shared_schema(&bindings, &[incompatible]).is_err());
		let mut array = bindings[0].clone();
		array.count = 2;
		assert!(validate_binding_schema(&[array]).is_err());
		let mut writable_sampler = bindings[1].clone();
		writable_sampler.write = true;
		assert!(validate_binding_schema(&[writable_sampler]).is_err());
		assert!(matches!(
			texture_view(TextureView::Texture3D),
			ghi::TextureViewTypes::Texture3D
		));
		assert!(validate_resources(&bindings, &resources[..2])
			.expect_err("Expected a missing resource")
			.starts_with("Missing compute resource `source`."));
		let duplicate = [resources[0], resources[1], resources[2], resources[2]];
		assert!(validate_resources(&bindings, &duplicate)
			.expect_err("Expected a duplicate resource")
			.starts_with("Duplicate compute resource `source`."));
		let wrong = [resources[0], resources[2], Resource::image("parameters", image)];
		assert!(validate_resources(&bindings, &wrong)
			.expect_err("Expected a resource type mismatch")
			.starts_with("Compute resource `parameters` has the wrong type."));
		let unknown = [resources[0], resources[1], resources[2], Resource::image("typo", image)];
		assert!(validate_resources(&bindings, &unknown)
			.expect_err("Expected an unknown resource")
			.starts_with("Unknown compute resource `typo`."));
		let planned = [
			resources[0],
			resources[1],
			resources[2],
			Resource::planned_buffer("future", buffer),
		];
		validate_resources(&bindings, &planned).expect("planned resources should validate");
	}
}
