//! Reusable construction for flat-resource BESL compute render passes.

use std::sync::Arc;

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::ContextCreate as _,
};
use resource_management::{
	shader::besl::evaluation::{BindingKind, BindingUsage, TextureView},
	types::ShaderTypes as ResourceShaderTypes,
};
use smallvec::SmallVec;
use utils::Extent;

use super::{allocate_render_command, RenderPass, RenderPassBuilder, RenderPassReturn};
use crate::rendering::Sink;

/// The `Descriptor` struct describes the stable shader and naming contract for one single-set BESL compute pipeline.
pub struct Descriptor<'a> {
	label: &'static str,
	shader_id: &'a str,
	shader_name: &'a str,
}

impl<'a> Descriptor<'a> {
	/// Creates a descriptor for one baked shader resource and its human-readable GPU label.
	pub fn new(label: &'static str, shader_id: &'a str, shader_name: &'a str) -> Self {
		Self {
			label,
			shader_id,
			shader_name,
		}
	}
}

/// Compiles one canonical BESL asset and returns its compute entry point for semantic tests.
#[cfg(test)]
pub fn compile_test_program(source: &str) -> besl::NodeReference {
	let program = besl::compile_to_besl(source, None).expect(
		"Failed to compile a canonical render-pass BESL asset. The most likely cause is invalid source syntax or descriptor declarations.",
	);
	program.get_main().expect(
		"Canonical render-pass entry point is missing. The most likely cause is that the BESL asset does not define `main`.",
	)
}

/// The `Pipeline` struct provides reusable compute state to sink-specific retained resource sets.
#[derive(Clone)]
pub struct Pipeline {
	handle: ghi::PipelineHandle,
	label: &'static str,
	workgroup: Extent,
	bindings: Arc<[BindingUsage]>,
}

impl Pipeline {
	/// Loads a baked shader and derives its dispatch and binding contracts from the persisted reflected interface.
	pub fn compile(render_pass_builder: &mut RenderPassBuilder<'_>, descriptor: Descriptor<'_>) -> Result<Self, String> {
		Self::build(render_pass_builder, descriptor, None)
	}

	/// Loads another baked shader against this pipeline's validated binding layout.
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
		let Descriptor {
			label,
			shader_id,
			shader_name,
		} = descriptor;
		let loaded = render_pass_builder.load_shader(shader_id, shader_name)?;
		if loaded.stage != ResourceShaderTypes::Compute {
			return Err(format!(
				"Render-pass shader '{shader_id}' is not a compute shader. The most likely cause is incorrect .besl.bead stage metadata."
			));
		}
		let (width, height, depth) = loaded.interface.workgroup_size.ok_or_else(|| {
			format!(
				"Render-pass shader '{shader_id}' has no compute workgroup size. The most likely cause is missing .besl.bead workgroup metadata."
			)
		})?;
		let workgroup = Extent::new(width, height, depth);
		let bindings = loaded
			.interface
			.bindings
			.into_iter()
			.map(|binding| BindingUsage {
				name: binding.name,
				kind: binding.kind,
				count: binding.count,
				slot: binding.slot,
				read: binding.read,
				write: binding.write,
			})
			.collect::<Vec<_>>();
		validate_binding_schema(&bindings)?;
		let binding_schema = if let Some(shared_layout) = shared_layout {
			validate_shared_schema(&shared_layout.bindings, &bindings)?;
			shared_layout.bindings.clone()
		} else {
			bindings.into()
		};
		let handle = render_pass_builder
			.context()
			.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
				&[],
				ghi::ShaderParameter::new(&loaded.handle, ghi::ShaderTypes::Compute),
			));

		Ok(Self {
			handle,
			label,
			workgroup,
			bindings: binding_schema,
		})
	}

	/// Validates named resources, creates the descriptor set, and freezes binding order for frame recording.
	pub fn bind(
		&self,
		render_pass_builder: &mut RenderPassBuilder<'_>,
		descriptor_set_name: &str,
		resources: &[Resource<'_>],
	) -> Result<Pass, String> {
		validate_resources(&self.bindings, resources)?;

		let context = render_pass_builder.context();
		let descriptor_set = context.create_descriptor_set(Some(descriptor_set_name));
		let mut writes = SmallVec::<[ghi::DescriptorWrite; 8]>::with_capacity(self.bindings.len());
		for binding in self.bindings.iter() {
			let resource = resources.iter().find(|resource| resource.name() == binding.name).expect(
				"Validated compute resource disappeared. The most likely cause is inconsistent named-resource validation.",
			);
			writes.push(resource.descriptor_write(descriptor_set, ghi::ResourceSlot::new(binding.slot)));
		}
		context.write(&writes);

		Ok(Pass {
			pipeline: self.handle,
			descriptor_set,
			label: self.label,
			workgroup: self.workgroup,
		})
	}
}

/// The `Resource` enum names one concrete resource for a reachable or planned BESL binding.
#[derive(Clone, Copy)]
pub enum Resource<'a> {
	Buffer(&'a str, ghi::BaseBufferHandle),
	PlannedBuffer(&'a str, ghi::BaseBufferHandle),
	Image(&'a str, ghi::BaseImageHandle),
	CombinedImageSampler(&'a str, ghi::BaseImageHandle, ghi::SamplerHandle, ghi::Layouts),
	Swapchain(&'a str, ghi::SwapchainHandle),
}

impl<'a> Resource<'a> {
	pub fn buffer(name: &'a str, buffer: impl Into<ghi::BaseBufferHandle>) -> Self {
		Self::Buffer(name, buffer.into())
	}

	/// Keeps a buffer ready for a BESL binding that is intentionally not reachable yet.
	pub fn planned_buffer(name: &'a str, buffer: impl Into<ghi::BaseBufferHandle>) -> Self {
		Self::PlannedBuffer(name, buffer.into())
	}

	pub fn image(name: &'a str, image: impl Into<ghi::BaseImageHandle>) -> Self {
		Self::Image(name, image.into())
	}

	pub fn combined_image_sampler(
		name: &'a str,
		image: impl Into<ghi::BaseImageHandle>,
		sampler: ghi::SamplerHandle,
		layout: ghi::Layouts,
	) -> Self {
		Self::CombinedImageSampler(name, image.into(), sampler, layout)
	}

	pub fn swapchain(name: &'a str, swapchain: ghi::SwapchainHandle) -> Self {
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
#[derive(Clone, Copy)]
pub struct Pass {
	pipeline: ghi::PipelineHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	label: &'static str,
	workgroup: Extent,
}

impl Pass {
	/// Records this pass without allocating a render-command closure.
	pub fn record(&self, command_buffer: &mut ghi::implementation::CommandBufferRecording, extent: Extent) {
		let command_buffer = command_buffer.bind_compute_pipeline(self.pipeline);
		command_buffer.bind_descriptor_sets(&[self.descriptor_set]);
		command_buffer.dispatch(ghi::DispatchExtent::new(extent, self.workgroup));
	}
}

impl RenderPass for Pass {
	fn prepare<'a>(
		&mut self,
		_frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pass = *self;
		let extent = sink.extent();

		Some(allocate_render_command(frame_allocator, move |command_buffer, _| {
			command_buffer.region(
				|region_label| region_label.write_str(pass.label),
				|command_buffer| pass.record(command_buffer, extent),
			);
		}))
	}
}

fn texture_view(view: TextureView) -> ghi::TextureViewTypes {
	match view {
		TextureView::Texture2D => ghi::TextureViewTypes::Texture2D,
		TextureView::Texture2DArray => ghi::TextureViewTypes::Texture2DArray,
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

fn validate_resources(bindings: &[BindingUsage], resources: &[Resource<'_>]) -> Result<(), String> {
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
		assert!(validate_resources(&bindings, &resources).is_ok());
		assert!(bindings[0].read && !bindings[0].write);
		assert!(validate_shared_schema(&bindings, &[bindings[0].clone(), bindings[2].clone()]).is_ok());
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
		assert!(validate_resources(&bindings, &planned).is_ok());
	}
}
