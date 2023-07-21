//! The [`RenderSystem`] implements easy to use rendering functionality.
//! It provides useful abstractions to interact with the GPU.
//! It's not tied to any particular render pipeline implementation.

use std::collections::HashMap;
use std::hash::Hasher;

use crate::render_backend::RenderBackend;
use crate::{window_system, orchestrator::System, Extent};
use crate::{render_backend, insert_return_length, render_debugger, orchestrator};

/// Returns the best value from a slice of values based on a score function.
pub fn select_by_score<T>(values: &[T], score: impl Fn(&T) -> i64) -> Option<&T> {
	let mut best_score = -1 as i64;
	let mut best_value: Option<&T> = None;

	for value in values {
		let score = score(value);

		if score > best_score {
			best_score = score;
			best_value = Some(value);
		}
	}

	best_value
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineLayoutHandle(u32);

struct PipelineLayout {
	pipeline_layout: crate::render_backend::PipelineLayout,
}

struct CommandBuffer {
	frame_handle: Option<FrameHandle>,
	next: Option<CommandBufferHandle>,
	command_buffer: crate::render_backend::CommandBuffer,
}

struct Shader {
	shader: crate::render_backend::Shader,
}

struct Pipeline {
	pipeline: crate::render_backend::Pipeline,
}

struct Texture {
	frame_handle: Option<FrameHandle>,
	next: Option<TextureHandle>,
	parent: Option<TextureHandle>,
	texture: crate::render_backend::Texture,
	texture_view: Option<crate::render_backend::TextureView>,
	allocation_handle: AllocationHandle,
	format: render_backend::TextureFormats,
	extent: Extent,
	role: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerHandle(u32);

struct Sampler {
	sampler: render_backend::Sampler,
}

pub enum ShaderSourceType {
	GLSL,
	SPIRV,
}

#[derive(Hash, Clone, Copy)]
pub enum DataTypes {
	Float,
	Float2,
	Float3,
	Float4,
	Int,
	Int2,
	Int3,
	Int4,
	UInt,
	UInt2,
	UInt3,
	UInt4,
}

#[derive(Hash)]
pub struct VertexElement {
	pub name: String,
	pub format: DataTypes,
	pub shuffled: bool,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct BufferHandle(u32);

struct Buffer {
	frame_handle: Option<FrameHandle>,
	next: Option<BufferHandle>,
	buffer: crate::render_backend::Buffer,
	size: usize,
	pointer: *mut u8,
}

unsafe impl Send for Buffer {} // Needed for pointer field
unsafe impl Sync for Buffer {} // Needed for pointer field

struct Mesh {
	vertex_buffer: Buffer,
	index_buffer: Buffer,
	vertex_layout_hash: u64,
	index_count: u32,
}

#[derive(Clone)]
struct Allocation {
	allocation: crate::render_backend::Allocation,
}

use bitflags::bitflags;

bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	pub struct DeviceAccesses: u16 {
		const CpuRead = 1 << 0;
		const CpuWrite = 1 << 1;
		const GpuRead = 1 << 2;
		const GpuWrite = 1 << 3;
	}
}

#[derive(Clone, Copy)]
pub struct CommandBufferHandle(u32);

struct Synchronizer {
	frame_handle: Option<FrameHandle>,
	next: Option<SynchronizerHandle>,
	synchronizer: crate::render_backend::Synchronizer,
}

pub struct ShaderHandle(u32);
pub struct PipelineHandle(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(u32);

pub struct MeshHandle(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceHandle(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SynchronizerHandle(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorSetLayoutHandle(u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorSetHandle(u32);

pub struct AttachmentInfo {
	pub(crate) texture: TextureHandle,
	pub(crate) format: crate::render_backend::TextureFormats,
	pub(crate) clear: Option<crate::RGBA>,
	pub(crate) load: bool,
	pub(crate) store: bool,
}

pub struct CommandBufferRecording<'a> {
	render_system: &'a RenderSystem,
	command_buffer: CommandBufferHandle,
	frame_handle: Option<FrameHandle>,
	in_render_pass: bool,
	/// `texture_states` is used to perform resource tracking on textures.\ It is mainly useful for barriers and transitions and copies.
	texture_states: HashMap<TextureHandle, crate::render_backend::TransitionState>,
}

pub struct BufferDescriptor {
	pub buffer: BufferHandle,
	pub offset: u64,
	pub range: u64,
	pub slot: u32,
}

impl CommandBufferRecording<'_> {
	pub fn new(render_system: &'_ RenderSystem, command_buffer: CommandBufferHandle, frame_handle: Option<FrameHandle>) -> CommandBufferRecording<'_> {
		CommandBufferRecording {
			render_system,
			command_buffer,
			frame_handle,
			in_render_pass: false,
			texture_states: HashMap::new(),
		}
	}

	/// Retrieves the current state of a texture.\
	/// If the texture has no known state, it will return a default state with undefined layout. This is useful for the first transition of a texture.\
	/// If the texture has a known state, it will return the known state.
	fn get_texture_state(&self, texture_handle: TextureHandle) -> Option<crate::render_backend::TransitionState> {
		if let Some(state) = self.texture_states.get(&texture_handle) {
			Some(*state)
		} else {
			None
		}
	}

	/// Inserts or updates state for a texture.\
	/// If the texture has no known state, it will insert the given state.\
	/// If the texture has a known state, it will update it with the given state.
	/// It will return the given state.
	/// This is useful to perform a transition on a texture.
	fn upsert_texture_state(&mut self, texture_handle: TextureHandle, texture_state: crate::render_backend::TransitionState) -> crate::render_backend::TransitionState {
		self.texture_states.insert(texture_handle, texture_state);
		texture_state
	}

	/// Enables recording on the command buffer.
	pub fn begin(&self) {
		self.render_system.render_backend.begin_command_buffer_recording(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer);
	}

	/// Starts a render pass on the GPU.
	/// A render pass is a particular configuration of render targets which will be used simultaneously to render certain imagery.
	pub fn start_render_pass(&mut self, extent: Extent, attachments: &[AttachmentInfo]) {
		let barriers = attachments.iter().map(|attachment| crate::render_backend::BarrierDescriptor {
			barrier: crate::render_backend::Barrier::Texture(self.render_system.get_texture(self.frame_handle, attachment.texture).texture),
			source: self.get_texture_state(attachment.texture),
			destination: self.upsert_texture_state(attachment.texture, crate::render_backend::TransitionState {
				layout: crate::render_backend::Layouts::RenderTarget,
				format: attachment.format,
				stage: crate::render_backend::Stages::FRAGMENT,
				access: crate::render_backend::AccessPolicies::WRITE,
			}),
		}).collect::<Vec<_>>();

		self.render_system.render_backend.execute_barriers(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &barriers);

		let mut attachment_information = Vec::new();

		for attachment in attachments {
			attachment_information.push(crate::render_backend::AttachmentInformation {
				texture_view: self.render_system.get_texture(self.frame_handle, attachment.texture).texture_view.unwrap(),
				format: attachment.format,
				layout: crate::render_backend::Layouts::RenderTarget,
				clear: attachment.clear,
				load: attachment.load,
				store: attachment.store,
			});
		}

		self.render_system.render_backend.start_render_pass(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, extent, &attachment_information);

		self.in_render_pass = true;
	}

	/// Ends a render pass on the GPU.
	pub fn end_render_pass(&mut self) {
		self.render_system.render_backend.end_render_pass(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer);
		self.in_render_pass = false;
	}

	/// Binds a shader to the GPU.
	pub fn bind_shader(&self, shader_handle: ShaderHandle) {
		self.render_system.render_backend.bind_shader(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &self.render_system.shaders[shader_handle.0 as usize].shader);
	}

	/// Binds a pipeline to the GPU.
	pub fn bind_pipeline(&mut self, pipeline_handle: &PipelineHandle) {
		self.render_system.render_backend.bind_pipeline(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &self.render_system.pipelines[pipeline_handle.0 as usize].pipeline);
	}

	/// Writes to the push constant register.
	pub fn write_to_push_constant(&mut self, pipeline_layout_handle: &PipelineLayoutHandle, offset: u32, data: &[u8]) {
		self.render_system.render_backend.write_to_push_constant(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &self.render_system.pipeline_layouts[pipeline_layout_handle.0 as usize].pipeline_layout, offset, data);
	}

	/// Draws a render system mesh.
	pub fn draw_mesh(&mut self, mesh_handle: &MeshHandle) {
		let mesh = &self.render_system.meshes[mesh_handle.0 as usize];

		let vertex_buffer_descriptors = [
			render_backend::BufferDescriptor {
				buffer: mesh.vertex_buffer.buffer,
				offset: 0,
				range: 0,
				slot: 0,
			}
		];

		let index_buffer_descritor = render_backend::BufferDescriptor {
			buffer: mesh.index_buffer.buffer,
			offset: 0,
			range: 0,
			slot: 0,
		};

		self.render_system.render_backend.bind_vertex_buffers(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &vertex_buffer_descriptors);
		self.render_system.render_backend.bind_index_buffer(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &index_buffer_descritor);

		self.render_system.render_backend.draw_indexed(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, mesh.index_count, 1, 0, 0, 0);
	}

	pub fn bind_vertex_buffers(&mut self, buffer_descriptors: &[BufferDescriptor]) {
		let buffer_descriptors = buffer_descriptors.iter().map(|buffer_descriptor| crate::render_backend::BufferDescriptor {
			buffer: self.render_system.buffers[buffer_descriptor.buffer.0 as usize].buffer,
			offset: buffer_descriptor.offset,
			range: buffer_descriptor.range,
			slot: buffer_descriptor.slot,
		}).collect::<Vec<_>>();

		self.render_system.render_backend.bind_vertex_buffers(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &buffer_descriptors);
	}

	pub fn bind_index_buffer(&mut self, buffer_descriptor: &BufferDescriptor) {
		let buffer_descriptor = crate::render_backend::BufferDescriptor {
			buffer: self.render_system.buffers[buffer_descriptor.buffer.0 as usize].buffer,
			offset: buffer_descriptor.offset,
			range: buffer_descriptor.range,
			slot: buffer_descriptor.slot,
		};

		self.render_system.render_backend.bind_index_buffer(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &buffer_descriptor);
	}

	pub fn draw_indexed(&mut self, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32) {
		self.render_system.render_backend.draw_indexed(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, index_count, instance_count, first_index, vertex_offset, first_instance);
	}

	/// Performs a transition on a series of textures.
	/// A transition is a change in layout, stage and access.
	/// The `keep_old` parameter determines whether the texture's contents should be kept or discarded.
	/// The `layout` parameter determines the new layout of the texture.
	/// The `stage` parameter determines the new stage of the texture.
	/// The `access` parameter determines the new access of the texture.
	/// The resource states are automatically tracked.
	pub fn transition_textures(&mut self, transitions: &[(TextureHandle, bool, crate::render_backend::Layouts, crate::render_backend::Stages, crate::render_backend::AccessPolicies)]) {
		let mut barriers = Vec::new();

		for (texture_handle, keep_old, layout, stage, access) in transitions {
			let texture = self.render_system.get_texture(self.frame_handle, *texture_handle);

			barriers.push(crate::render_backend::BarrierDescriptor {
				barrier: crate::render_backend::Barrier::Texture(texture.texture),
				source: if *keep_old { self.get_texture_state(*texture_handle) } else { None },
				destination: self.upsert_texture_state(*texture_handle, crate::render_backend::TransitionState { layout: *layout, format: texture.format, stage: *stage, access: *access, }),
			});
		}

		self.render_system.render_backend.execute_barriers(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &barriers);
	}

	/// Copies texture data from a CPU accessible buffer to a GPU accessible texture.
	pub fn write_texture_data(&mut self, texture_handle: TextureHandle, data: &[RGBAu8]) {
		let source_texture_res = self.render_system.textures.iter().enumerate().find(|(_, texture)| if let Some(parent) = texture.parent { parent  == texture_handle && texture.role == "CPU_WRITE" } else { false }).unwrap();

		let source_texture = &self.render_system.textures[source_texture_res.0 as usize];
		let destination_texture = &self.render_system.textures[texture_handle.0 as usize];

		let allocation = &self.render_system.allocations[source_texture.allocation_handle.0 as usize];

		let pointer = self.render_system.render_backend.get_allocation_pointer(&allocation.allocation);

		let subresource_layout = self.render_system.render_backend.get_image_subresource_layout(&source_texture.texture, 0);

		if pointer.is_null() {
			for i in data.len()..source_texture.extent.width as usize * source_texture.extent.height as usize * source_texture.extent.depth as usize {
				unsafe {
					std::ptr::write(pointer.offset(i as isize), if i % 4 == 0 { 255 } else { 0 });
				}
			}
		} else {
			let pointer = unsafe { pointer.offset(subresource_layout.offset as isize) };

			for i in 0..source_texture.extent.height {
				let pointer = unsafe { pointer.offset(subresource_layout.row_pitch as isize * i as isize) };

				unsafe {
					std::ptr::copy_nonoverlapping((data.as_ptr().add(i as usize * source_texture.extent.width as usize)) as *mut u8, pointer, source_texture.extent.width as usize * 4);
				}
			}
		}

		let source_texture_handle = TextureHandle(source_texture_res.0 as u32);
		let destination_texture_handle = texture_handle;

		self.transition_textures(&[
			(source_texture_handle, false, crate::render_backend::Layouts::Transfer, crate::render_backend::Stages::TRANSFER, crate::render_backend::AccessPolicies::READ), // Source texture
			(destination_texture_handle, false, crate::render_backend::Layouts::Transfer, crate::render_backend::Stages::TRANSFER, crate::render_backend::AccessPolicies::WRITE), // Destination texture
		]);

		let texture_copy = crate::render_backend::TextureCopy {
			source: source_texture.texture,
			source_format: source_texture.format,
			destination: destination_texture.texture,
			destination_format: destination_texture.format,
			extent: source_texture.extent,
		};

		self.render_system.render_backend.copy_textures(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &[texture_copy]);

		self.transition_textures(&[
			(texture_handle, true, crate::render_backend::Layouts::Texture, crate::render_backend::Stages::FRAGMENT, crate::render_backend::AccessPolicies::READ), // Destination texture
		]);
	}

	/// Performs a series of texture copies.
	pub fn copy_textures(&mut self, copies: &[(TextureHandle, TextureHandle)]) {
		let mut transitions = Vec::new();

		for (f, t) in copies {
			transitions.push((*f, true, render_backend::Layouts::Transfer, render_backend::Stages::TRANSFER, render_backend::AccessPolicies::READ));
			transitions.push((*t, false, render_backend::Layouts::Transfer, render_backend::Stages::TRANSFER, render_backend::AccessPolicies::WRITE));
		}

		self.transition_textures(&transitions);

		let mut texture_copies = Vec::new();

		for (f, t,) in copies {
			let source_texture = self.render_system.get_texture(self.frame_handle, *f);
			let destination_texture = self.render_system.get_texture(self.frame_handle, *t);
			texture_copies.push(crate::render_backend::TextureCopy {
				source: source_texture.texture,
				source_format: source_texture.format,
				destination: destination_texture.texture,
				destination_format: destination_texture.format,
				extent: source_texture.extent,
			});
		}

		self.render_system.render_backend.copy_textures(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &texture_copies);
	}

	/// Copies GPU accessible texture data to a CPU accessible buffer.
	pub fn synchronize_texture(&mut self, texture_handle: TextureHandle) {
		let mut texture_copies = Vec::new();

		let texture = self.render_system.get_texture(self.frame_handle, texture_handle);

		let copy_dst_texture = self.render_system.textures.iter().enumerate().find(|(_, texture)| texture.parent == Some(texture_handle) && texture.role == "CPU_READ").expect("No CPU_READ texture found. Texture must be created with the CPU read access flag.");
		
		let source_texture_handle = texture_handle;
		let destination_texture_handle = TextureHandle(copy_dst_texture.0 as u32);
		
		let transitions = [
			(source_texture_handle, true, crate::render_backend::Layouts::Transfer, crate::render_backend::Stages::TRANSFER, crate::render_backend::AccessPolicies::READ),
			(destination_texture_handle, false, crate::render_backend::Layouts::Transfer, crate::render_backend::Stages::TRANSFER, crate::render_backend::AccessPolicies::WRITE)
		];

		self.transition_textures(&transitions);

		texture_copies.push(crate::render_backend::TextureCopy {
			source: texture.texture,
			source_format: texture.format,
			destination: copy_dst_texture.1.texture,
			destination_format: copy_dst_texture.1.format,
			extent: texture.extent,
		});

		self.render_system.render_backend.copy_textures(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &texture_copies);
	}

	/// Ends recording on the command buffer.
	pub fn end(&mut self) {
		if self.in_render_pass {
			self.render_system.render_backend.end_render_pass(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer);
		}

		if let Some(surface) = &self.render_system.surface {
			//barrier: crate::render_backend::Barrier::Texture(self.render_system.textures[surface.textures[self.frame_handle.unwrap().0 as usize].0 as usize].texture),

			let transitions = [
				(surface.textures[self.frame_handle.unwrap().0 as usize], true, crate::render_backend::Layouts::Present, crate::render_backend::Stages::TRANSFER, crate::render_backend::AccessPolicies::READ),
			];

			self.transition_textures(&transitions);
		}

		self.render_system.render_backend.end_command_buffer_recording(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer);
	}

	/// Binds a decriptor set on the GPU.
	pub fn bind_descriptor_set(&self, pipeline_layout: PipelineLayoutHandle, arg: u32, descriptor_set_handle: &DescriptorSetHandle) {
		self.render_system.render_backend.bind_descriptor_set(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &self.render_system.pipeline_layouts[pipeline_layout.0 as usize].pipeline_layout, &self.render_system.descriptor_sets[descriptor_set_handle.0 as usize].descriptor_set, arg);
	}
}

pub struct AllocationHandle(u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameHandle(u32);

struct Frame {}

struct Surface {
	surface: crate::render_backend::Surface,
	swapchain: crate::render_backend::Swapchain,
	textures: Vec<TextureHandle>,
}

pub struct DescriptorSetLayout {
	descriptor_set_layout: render_backend::DescriptorSetLayout,
}

pub struct DescriptorSet {
	descriptor_set: render_backend::DescriptorSet,
}

pub enum Descriptor {
	Buffer(BufferHandle),
	Texture(TextureHandle),
}

pub struct DescriptorWrite {
	descriptor_set: DescriptorSetHandle,
	binding: u32,
	array_element: u32,
	descriptor: Descriptor,
}

pub struct DescriptorSetLayoutBinding {
	pub binding: u32,
	pub descriptor_type: render_backend::DescriptorType,
	pub descriptor_count: u32,
	pub stage_flags: render_backend::Stages,
	pub immutable_samplers: Option<Vec<SamplerHandle>>,
}

/// The [`RenderSystem`] implements easy to use rendering functionality.
/// It is a wrapper around the [`render_backend::RenderBackend`].
/// It is responsible for creating and managing resources.
/// It also provides a [`CommandBufferRecording`] struct that can be used to record commands.
pub struct RenderSystem {
	render_backend: crate::vulkan_render_backend::VulkanRenderBackend,
	debugger: crate::render_debugger::RenderDebugger,
	allocations: Vec<Allocation>,
	buffers: Vec<Buffer>,
	textures: Vec<Texture>,
	samplers: Vec<Sampler>,
	descriptor_set_layouts: Vec<DescriptorSetLayout>,
	pipeline_layouts: Vec<PipelineLayout>,
	descriptor_sets: Vec<DescriptorSet>,
	shaders: Vec<Shader>,
	pipelines: Vec<Pipeline>,
	command_buffers: Vec<CommandBuffer>,
	meshes: Vec<Mesh>,
	surface: Option<Surface>,
	synchronizers: Vec<Synchronizer>,
	frames: Vec<Frame>,
}

impl RenderSystem {
	/// Creates a new [`RenderSystem`].
	fn new() -> RenderSystem {
		let render_backend = crate::vulkan_render_backend::VulkanRenderBackend::new();

		let render_system = RenderSystem {
			render_backend,
			debugger: render_debugger::RenderDebugger::new(),
			allocations: Vec::new(),
			buffers: Vec::new(),
			textures: Vec::new(),
			samplers: Vec::new(),
			descriptor_set_layouts: Vec::new(),
			pipeline_layouts: Vec::new(),
			descriptor_sets: Vec::new(),
			shaders: Vec::new(),
			pipelines: Vec::new(),
			command_buffers: Vec::new(),
			meshes: Vec::new(),
			surface: None,
			synchronizers: Vec::new(),
			frames: Vec::new(),
		};

		render_system
	}

	/// Creates a new [`RenderSystem`].
	pub fn new_as_system(orchestrator: orchestrator::OrchestratorReference) -> RenderSystem {
		Self::new()
	}

	pub fn has_errors(&self) -> bool {
		self.render_backend.get_log_count() > 0
	}

	/// Creates a new frame
	/// A frame let's the render system know how many queued frames it can have
	pub fn create_frame(&mut self) -> FrameHandle {
		FrameHandle(insert_return_length(&mut self.frames, Frame {}) as u32)
	}

	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	pub fn create_allocation(&mut self, size: usize, _resource_uses: render_backend::Uses, resource_device_accesses: DeviceAccesses) -> AllocationHandle {
		let allocation = self.render_backend.allocate_memory(size, resource_device_accesses);

		let allocation_handle = AllocationHandle(self.allocations.len() as u64);

		self.allocations.push(Allocation {
			allocation,
		});

		allocation_handle
	}

	pub fn add_mesh_from_vertices_and_indices(&mut self, vertices: &[u8], indices: &[u8], vertex_layout: &[VertexElement]) -> MeshHandle {
		//self.render_backend.add_mesh_from_vertices_and_indices(vertices, indices);

		let mut hasher = std::collections::hash_map::DefaultHasher::new();

		std::hash::Hash::hash_slice(&vertex_layout, &mut hasher);

		let vertex_layout_hash = hasher.finish();

		let vertex_buffer_size = vertices.len();
		let vertex_buffer_creation_result = self.render_backend.create_buffer(vertex_buffer_size, render_backend::Uses::Vertex);
		let index_buffer_size = indices.len();
		let index_buffer_creation_result = self.render_backend.create_buffer(index_buffer_size, render_backend::Uses::Index);

		let vertex_buffer_offset = 0usize;
		let index_buffer_offset = vertices.len().next_multiple_of(index_buffer_creation_result.alignment);

		let allocation_handle = self.create_allocation(vertex_buffer_creation_result.size.next_multiple_of(index_buffer_creation_result.alignment) + index_buffer_creation_result.size, render_backend::Uses::Vertex | render_backend::Uses::Index, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead);

		let allocation = &self.allocations[allocation_handle.0 as usize];

		self.render_backend.bind_buffer_memory(crate::render_backend::Memory{ allocation: &allocation.allocation, offset: vertex_buffer_offset, size: vertex_buffer_size }, &vertex_buffer_creation_result);
		self.render_backend.bind_buffer_memory(crate::render_backend::Memory{ allocation: &allocation.allocation, offset: index_buffer_offset, size: index_buffer_size }, &index_buffer_creation_result);		

		unsafe {
			let vertex_buffer_pointer = self.render_backend.get_allocation_pointer(&allocation.allocation).offset(vertex_buffer_offset as isize);
			std::ptr::copy_nonoverlapping(vertices.as_ptr(), vertex_buffer_pointer, vertex_buffer_size as usize);
			let index_buffer_pointer = self.render_backend.get_allocation_pointer(&allocation.allocation).offset(index_buffer_offset as isize);
			std::ptr::copy_nonoverlapping(indices.as_ptr(), index_buffer_pointer, index_buffer_size as usize);
		}

		let mesh_handle = MeshHandle(self.meshes.len() as u32);

		self.meshes.push(Mesh {
			vertex_buffer: Buffer {
				frame_handle: None,
				next: None,
				buffer: vertex_buffer_creation_result.resource,
				size: vertex_buffer_creation_result.size,
				pointer: std::ptr::null_mut(),
			},
			index_buffer: Buffer {
				frame_handle: None,
				next: None,
				buffer: index_buffer_creation_result.resource,
				size: index_buffer_creation_result.size,
				pointer: std::ptr::null_mut(),
			},
			vertex_layout_hash,
			index_count: indices.len() as u32 / 4,
		});

		mesh_handle
	}

	/// Creates a shader.
	pub fn add_shader(&mut self, _shader_source_type: ShaderSourceType, shader: &[u8]) -> ShaderHandle {
		let compiler = shaderc::Compiler::new().unwrap();
		let mut options = shaderc::CompileOptions::new().unwrap();

		options.set_optimization_level(shaderc::OptimizationLevel::Performance);
		options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_2 as u32);
		options.set_generate_debug_info();
		options.set_target_spirv(shaderc::SpirvVersion::V1_5);
		options.set_invert_y(true);

		let shader_text = std::str::from_utf8(shader).unwrap();

		let binary = compiler.compile_into_spirv(shader_text, shaderc::ShaderKind::InferFromSource, "shader_name", "main", Some(&options));

		let shader_handle = ShaderHandle(self.shaders.len() as u32);

		let shader_stage: String = shader_text.find("#pragma shader_stage(").map(|index| shader_text[index + 21..].split(')').next().unwrap().to_string()).unwrap_or(String::from(""));

		let shader_stage = match shader_stage.as_str() {
			"vertex" => {
				crate::render_backend::ShaderTypes::Vertex
			},
			"fragment" => {
				crate::render_backend::ShaderTypes::Fragment
			},
			_ => {
				crate::render_backend::ShaderTypes::Vertex
			},
		};

		match binary {
			Ok(binary) => {
				self.shaders.push(Shader {
					shader: self.render_backend.create_shader(shader_stage, binary.as_binary_u8())
				});

				shader_handle
			},
			Err(error) => {
				println!("Error compiling shader: {}", error);

				shader_handle
			}
		}
	}

	pub fn create_descriptor_set_layout(&mut self, bindings: &[DescriptorSetLayoutBinding]) -> DescriptorSetLayoutHandle {
		let bindings = bindings.iter().map(|binding| crate::render_backend::DescriptorSetLayoutBinding {
			binding: binding.binding,
			descriptor_type: binding.descriptor_type,
			descriptor_count: binding.descriptor_count,
			stage_flags: binding.stage_flags,
			immutable_samplers: binding.immutable_samplers.as_ref().map(|immutable_samplers| immutable_samplers.iter().map(|sampler_handle| self.samplers[sampler_handle.0 as usize].sampler).collect::<Vec<_>>()),
		}).collect::<Vec<_>>();

		let descriptor_set_layout = self.render_backend.create_descriptor_set_layout(&bindings);

		let descriptor_set_layout_handle = DescriptorSetLayoutHandle(self.descriptor_set_layouts.len() as u32);

		self.descriptor_set_layouts.push(DescriptorSetLayout {
			descriptor_set_layout,
		});

		descriptor_set_layout_handle
	}

	pub fn create_descriptor_set(&mut self, descriptor_set_layout_handle: &DescriptorSetLayoutHandle, bindings: &[DescriptorSetLayoutBinding]) -> DescriptorSetHandle {
		let bindings = bindings.iter().map(|binding| crate::render_backend::DescriptorSetLayoutBinding {
			binding: binding.binding,
			descriptor_type: binding.descriptor_type,
			descriptor_count: binding.descriptor_count,
			stage_flags: binding.stage_flags,
			immutable_samplers: binding.immutable_samplers.as_ref().map(|immutable_samplers| immutable_samplers.iter().map(|sampler_handle| self.samplers[sampler_handle.0 as usize].sampler).collect::<Vec<_>>()),
		}).collect::<Vec<_>>();

		let descriptor_set_layout = &self.descriptor_set_layouts[descriptor_set_layout_handle.0 as usize];

		let descriptor_set = self.render_backend.create_descriptor_set(&descriptor_set_layout.descriptor_set_layout, &bindings);

		let descriptor_set_handle = DescriptorSetHandle(self.descriptor_sets.len() as u32);

		self.descriptor_sets.push(DescriptorSet {
			descriptor_set,
		});

		descriptor_set_handle
	}

	pub fn write(&self, descriptor_set_writes: &[DescriptorWrite]) {
		let descriptor_set_writes = descriptor_set_writes.iter().map(|descriptor_write| crate::render_backend::DescriptorSetWrite {
			descriptor_set: self.descriptor_sets[descriptor_write.descriptor_set.0 as usize].descriptor_set,
			binding: descriptor_write.binding,
			array_element: descriptor_write.array_element,
			descriptor_type: match descriptor_write.descriptor {
				Descriptor::Buffer(_) => crate::render_backend::DescriptorType::UniformBuffer,
				Descriptor::Texture(_) => crate::render_backend::DescriptorType::SampledImage,
			},
			descriptor_info: match descriptor_write.descriptor {
				Descriptor::Buffer(buffer_handle) => crate::render_backend::DescriptorInfo::Buffer{ buffer: self.buffers[buffer_handle.0 as usize].buffer, offset: 0, range: 64 },
				Descriptor::Texture(texture_handle) => crate::render_backend::DescriptorInfo::Texture{ texture: self.textures[texture_handle.0 as usize].texture_view.unwrap(), format: render_backend::TextureFormats::RGBAu8, layout: render_backend::Layouts::Texture, },
			},
		}).collect::<Vec<_>>();

		self.render_backend.write_descriptors(&descriptor_set_writes);
	}

	pub fn create_pipeline_layout(&mut self, descriptor_set_layout_handles: &[DescriptorSetLayoutHandle]) -> PipelineLayoutHandle {
		let pipeline_layout = self.render_backend.create_pipeline_layout(&descriptor_set_layout_handles.iter().map(|descriptor_set_layout_handle| self.descriptor_set_layouts[descriptor_set_layout_handle.0 as usize].descriptor_set_layout).collect::<Vec<_>>());
		PipelineLayoutHandle(insert_return_length(&mut self.pipeline_layouts, PipelineLayout{ pipeline_layout }) as u32)
	}

	pub fn create_pipeline(&mut self, pipeline_layout_handle: PipelineLayoutHandle, shader_handles: &[&ShaderHandle], vertex_layout: &[VertexElement], targets: &[AttachmentInfo]) -> PipelineHandle {
		let shaders = shader_handles.iter().map(|shader_handle| self.shaders[shader_handle.0 as usize].shader).collect::<Vec<_>>();

		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];

		let targets = targets.iter().map(|target| render_backend::AttachmentInformation {
			format: target.format,
			texture_view: self.textures[target.texture.0 as usize].texture_view.unwrap(),
			layout: render_backend::Layouts::RenderTarget,
			clear: target.clear,
			load: target.load,
			store: target.store,
		}).collect::<Vec<_>>();

		let pipeline_configuration_blocks = [
			render_backend::PipelineConfigurationBlocks::Shaders { 
				shaders: shaders.as_slice(),
			},
			render_backend::PipelineConfigurationBlocks::Layout { layout: &pipeline_layout.pipeline_layout },
			render_backend::PipelineConfigurationBlocks::VertexInput { vertex_elements: vertex_layout },
			render_backend::PipelineConfigurationBlocks::RenderTargets { targets: &targets },
		];

		let pipeline = self.render_backend.create_pipeline(&pipeline_configuration_blocks);

		let pipeline_handle = PipelineHandle(self.pipelines.len() as u32);

		self.pipelines.push(Pipeline {
			pipeline,
		});

		pipeline_handle
	}

	pub fn create_command_buffer(&mut self) -> CommandBufferHandle {
		let command_buffer_handle = CommandBufferHandle(self.command_buffers.len() as u32);

		let mut previous_command_buffer_handle: Option<CommandBufferHandle> = None;

		for i in 0..self.frames.len() {
			let command_buffer_handle = CommandBufferHandle(self.command_buffers.len() as u32);

			self.command_buffers.push(CommandBuffer {
				frame_handle: Some(FrameHandle(i as u32)),
				next: None,
				command_buffer: self.render_backend.create_command_buffer(),
			});

			if let Some(previous_command_buffer_handle) = previous_command_buffer_handle {
				self.command_buffers[previous_command_buffer_handle.0 as usize].next = Some(command_buffer_handle);
			}

			previous_command_buffer_handle = Some(command_buffer_handle);
		}

		command_buffer_handle
	}

	pub fn create_command_buffer_recording(&self, frame_handle: Option<FrameHandle>, command_buffer_handle: CommandBufferHandle) -> CommandBufferRecording {
		let recording = CommandBufferRecording::new(self, self.get_command_buffer_handle(frame_handle, command_buffer_handle), frame_handle);
		recording.begin();
		recording
	}

	/// Creates a new buffer.\
	/// If the access includes [`DeviceAccesses::CpuWrite`] and [`DeviceAccesses::GpuRead`] then multiple buffers will be created, one for each frame.\
	/// Staging buffers MAY be created if there's is not sufficient CPU writable, fast GPU readable memory.\
	/// 
	/// # Arguments
	/// 
	/// * `size` - The size of the buffer in bytes.
	/// * `resource_uses` - The uses of the buffer.
	/// * `device_accesses` - The accesses of the buffer.
	/// 
	/// # Returns
	/// 
	/// The handle of the buffer.
	pub fn create_buffer(&mut self, size: usize, resource_uses: render_backend::Uses, device_accesses: DeviceAccesses) -> BufferHandle {
		let buffer_handle = BufferHandle(self.buffers.len() as u32);

		let mut previous_buffer_handle: Option<BufferHandle> = None;

		if device_accesses.contains(DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead) {
			for i in 0..self.frames.len() {
				let buffer_handle = BufferHandle(self.buffers.len() as u32);

				let buffer_creation_result = self.render_backend.create_buffer(size, resource_uses);

				let allocation_handle = self.create_allocation(buffer_creation_result.size, resource_uses, device_accesses);

				let allocation = &self.allocations[allocation_handle.0 as usize];

				self.render_backend.bind_buffer_memory(crate::render_backend::Memory{ allocation: &allocation.allocation, offset: 0, size: size }, &buffer_creation_result);

				let pointer = self.render_backend.get_allocation_pointer(&allocation.allocation);

				self.buffers.push(Buffer {
					frame_handle: None,
					next: None,
					buffer: buffer_creation_result.resource,
					size: buffer_creation_result.size,
					pointer,
				});

				if let Some(previous_buffer_handle) = previous_buffer_handle {
					self.buffers[previous_buffer_handle.0 as usize].next = Some(buffer_handle);
				}

				previous_buffer_handle = Some(buffer_handle);
			}
		}

		buffer_handle
	}

	pub fn get_buffer_address(&self, frame_handle: Option<FrameHandle>, buffer_handle: BufferHandle) -> u64 {
		let buffer_handle = self.get_buffer_handle(frame_handle, buffer_handle);
		let buffer = &self.buffers[buffer_handle.0 as usize];
		self.render_backend.get_buffer_address(&buffer.buffer)
	}

	pub fn get_buffer_pointer(&mut self, frame_handle: Option<FrameHandle>, buffer_handle: BufferHandle) -> *mut u8 {
		let buffer_handle = self.get_buffer_handle(frame_handle, buffer_handle);
		let buffer = &mut self.buffers[buffer_handle.0 as usize];
		buffer.pointer
	}

	pub fn get_buffer_slice(&mut self, frame_handle: Option<FrameHandle>, buffer_handle: BufferHandle) -> &[u8] {
		let buffer_handle = self.get_buffer_handle(frame_handle, buffer_handle);
		let buffer = &mut self.buffers[buffer_handle.0 as usize];
		unsafe {
			std::slice::from_raw_parts(buffer.pointer, buffer.size as usize)
		}
	}

	// Return a mutable slice to the buffer data.
	pub fn get_mut_buffer_slice(&self, frame_handle: Option<FrameHandle>, buffer_handle: BufferHandle) -> &mut [u8] {
		let buffer_handle = self.get_buffer_handle(frame_handle, buffer_handle);
		let buffer = &self.buffers[buffer_handle.0 as usize];
		unsafe {
			std::slice::from_raw_parts_mut(buffer.pointer, buffer.size as usize)
		}
	}

	/// Creates a texture.
	pub fn create_texture(&mut self, extent: crate::Extent, format: crate::render_backend::TextureFormats, resource_uses: render_backend::Uses, device_accesses: DeviceAccesses) -> TextureHandle {
		let size = (extent.width * extent.height * extent.depth * 4) as usize;

		// CPU readeble render target
		if device_accesses == DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite {
			let texture_handle = TextureHandle(self.textures.len() as u32);
			let mut previous_texture_handle: Option<TextureHandle> = None;
			let mut previous_cpu_texture_handle: Option<TextureHandle> = None;

			for i in 0..self.frames.len() {
				let texture_creation_result = self.render_backend.create_texture(extent, format, resource_uses | render_backend::Uses::TransferSource, DeviceAccesses::GpuWrite, crate::render_backend::AccessPolicies::WRITE, 1);

				let allocation_handle = self.create_allocation(texture_creation_result.size, resource_uses, DeviceAccesses::GpuWrite);

				let allocation = &self.allocations[allocation_handle.0 as usize];

				self.render_backend.bind_texture_memory(crate::render_backend::Memory{ allocation: &allocation.allocation, offset: 0, size: size }, &texture_creation_result);

				let texture_view = self.render_backend.create_texture_view(&texture_creation_result.resource, format, 1);

				let texture_handle = TextureHandle(self.textures.len() as u32);

				self.textures.push(Texture {
					frame_handle: Some(FrameHandle(i as u32)),
					next: None,
					parent: None,
					format,
					texture: texture_creation_result.resource,
					texture_view: Some(texture_view),
					allocation_handle,
					extent,
					role: String::from("GPU_WRITE"),
				});

				if let Some(previous_texture_handle) = previous_texture_handle {
					self.textures[previous_texture_handle.0 as usize].next = Some(texture_handle);
				}

				previous_texture_handle = Some(texture_handle);

				let texture_creation_result = self.render_backend.create_texture(extent, format, render_backend::Uses::TransferDestination, DeviceAccesses::CpuRead, crate::render_backend::AccessPolicies::READ, 1);

				let allocation_handle = self.create_allocation(texture_creation_result.size, resource_uses, DeviceAccesses::CpuRead);

				let allocation = &self.allocations[allocation_handle.0 as usize];

				self.render_backend.bind_texture_memory(crate::render_backend::Memory{ allocation: &allocation.allocation, offset: 0, size: size }, &texture_creation_result);

				let cpu_texture_handle = TextureHandle(self.textures.len() as u32);

				self.textures.push(Texture {
					frame_handle: Some(FrameHandle(i as u32)),
					next: None,
					parent: Some(texture_handle),
					format,
					texture: texture_creation_result.resource,
					texture_view: None,
					allocation_handle,
					extent,
					role: String::from("CPU_READ"),
				});

				if let Some(previous_texture_handle) = previous_cpu_texture_handle {
					self.textures[previous_texture_handle.0 as usize].next = Some(cpu_texture_handle);
				}

				previous_cpu_texture_handle = Some(cpu_texture_handle);
			}

			texture_handle
		} else if device_accesses == DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead {
			let texture_creation_result = self.render_backend.create_texture(extent, format, resource_uses | render_backend::Uses::TransferSource, DeviceAccesses::CpuWrite, crate::render_backend::AccessPolicies::WRITE, 1);

			let allocation_handle = self.create_allocation(texture_creation_result.size, resource_uses, DeviceAccesses::CpuWrite);

			let allocation = &self.allocations[allocation_handle.0 as usize];

			self.render_backend.bind_texture_memory(crate::render_backend::Memory{ allocation: &allocation.allocation, offset: 0, size: size }, &texture_creation_result);

			let texture_view = self.render_backend.create_texture_view(&texture_creation_result.resource, format, 1);

			let proxy_texture_handle = TextureHandle(self.textures.len() as u32);

			self.textures.push(Texture {
				frame_handle: Some(FrameHandle(0)),
				next: None,
				parent: None,
				format,
				texture: texture_creation_result.resource,
				texture_view: Some(texture_view),
				allocation_handle,
				extent,
				role: String::from("CPU_WRITE"),
			});

			let texture_creation_result = self.render_backend.create_texture(extent, format, resource_uses | render_backend::Uses::TransferDestination, DeviceAccesses::GpuRead, crate::render_backend::AccessPolicies::READ, 1);

			let allocation_handle = self.create_allocation(texture_creation_result.size, resource_uses, DeviceAccesses::GpuRead);
	
			let allocation = &self.allocations[allocation_handle.0 as usize];
	
			self.render_backend.bind_texture_memory(crate::render_backend::Memory{ allocation: &allocation.allocation, offset: 0, size: size }, &texture_creation_result);
	
			let texture_view = self.render_backend.create_texture_view(&texture_creation_result.resource, format, 1);
	
			let texture_handle = self.create_texture_internal(Texture {
				frame_handle: Some(FrameHandle(0)),
				next: None,
				parent: None,
				format,
				texture: texture_creation_result.resource,
				texture_view: Some(texture_view),
				allocation_handle,
				extent,
				role: String::from("GPU_READ"),
			});
	
			self.textures[proxy_texture_handle.0 as usize].parent = Some(texture_handle);

			texture_handle
		} else {
			let texture_handle = TextureHandle(self.textures.len() as u32);
		
			let mut previous_texture_handle: Option<TextureHandle> = None;

			for i in 0..self.frames.len() {
				let texture_creation_result = self.render_backend.create_texture(extent, format, render_backend::Uses::RenderTarget | render_backend::Uses::TransferSource, DeviceAccesses::GpuWrite, crate::render_backend::AccessPolicies::WRITE, 1);

				let allocation_handle = self.create_allocation(texture_creation_result.size, render_backend::Uses::RenderTarget, DeviceAccesses::GpuWrite);
		
				let allocation = &self.allocations[allocation_handle.0 as usize];
		
				self.render_backend.bind_texture_memory(crate::render_backend::Memory{ allocation: &allocation.allocation, offset: 0, size: size }, &texture_creation_result);
		
				let texture_view = self.render_backend.create_texture_view(&texture_creation_result.resource, format, 1);
		
				let texture_handle = self.create_texture_internal(Texture {
					frame_handle: Some(FrameHandle(i as u32)),
					next: None,
					parent: None,
					format,
					texture: texture_creation_result.resource,
					texture_view: Some(texture_view),
					allocation_handle,
					extent,
					role: String::from(""),
				});

				if let Some(previous_texture_handle) = previous_texture_handle {
					self.textures[previous_texture_handle.0 as usize].next = Some(texture_handle);
				}

				previous_texture_handle = Some(texture_handle);
			}			

			texture_handle
		}
	}

	pub fn create_sampler(&mut self) -> SamplerHandle {
		let sampler_handle = SamplerHandle(self.samplers.len() as u32);

		self.samplers.push(Sampler {
			sampler: self.render_backend.create_sampler(),
		});

		sampler_handle
	}

	fn create_texture_internal(&mut self, texture: Texture) -> TextureHandle {
		let texture_handle = TextureHandle(self.textures.len() as u32);
		self.textures.push(texture);
		texture_handle
	}

	pub fn bind_to_window(&mut self, window_os_handles: window_system::WindowOsHandles) -> SurfaceHandle {
		let surface = self.render_backend.create_surface(window_os_handles); 

		let properties = self.render_backend.get_surface_properties(&surface);

		let extent = properties.extent;

		let swapchain = self.render_backend.create_swapchain(&surface, extent, std::cmp::max(self.frames.len() as u32, 2));

		let images = self.render_backend.get_swapchain_images(&swapchain);

		let mut textures = Vec::new();

		let mut previous_texture_handle: Option<TextureHandle> = None;

		for (i, image) in images.iter().enumerate() {
			let texture_view = self.render_backend.create_texture_view(&image, crate::render_backend::TextureFormats::BGRAu8, 1);

			let texture_handle = self.create_texture_internal(Texture {
				frame_handle: Some(FrameHandle(i as u32)),
				next: previous_texture_handle,
				parent: None,
				format: crate::render_backend::TextureFormats::BGRAu8,
				texture: *image,
				texture_view: Some(texture_view),
				allocation_handle: AllocationHandle(0xFFFFFFFFFFFFFFFF),
				extent: extent,
				role: String::from("SURFACE"),
			});

			if let Some(previous_texture_handle) = previous_texture_handle {
				self.textures[previous_texture_handle.0 as usize].next = Some(texture_handle);
			}

			textures.push(texture_handle);

			previous_texture_handle = Some(texture_handle);
		}

		self.surface = Some(
			Surface {
				surface,
				swapchain,
				textures,
			}
		);

		SurfaceHandle(0)
	}

	pub fn get_texture_data<T: 'static>(&self, texture_handle: TextureHandle) -> &[T] {
		let texture = self.textures.iter().find(|texture| texture.parent.map_or(false, |x| texture_handle == x)).unwrap(); // Get the proxy texture
		let allocation = &self.allocations[texture.allocation_handle.0 as usize];
		let slice = unsafe { std::slice::from_raw_parts::<'static, T>(self.render_backend.get_allocation_pointer(&allocation.allocation) as *mut T, (texture.extent.width*texture.extent.height*texture.extent.depth) as usize) };

		slice
	}

	/// Creates a synchronization primitive (implemented as a semaphore/fence/event).\
	/// Multiple underlying synchronization primitives are created, one for each frame
	pub fn create_synchronizer(&mut self, signaled: bool) -> SynchronizerHandle {
		let synchronizer_handle = SynchronizerHandle(self.synchronizers.len() as u32);

		let mut previous_synchronizer_handle: Option<SynchronizerHandle> = None;

		for i in 0..self.frames.len() {
			let synchronizer_handle = SynchronizerHandle(self.synchronizers.len() as u32);

			self.synchronizers.push(Synchronizer {
				frame_handle: Some(FrameHandle(i as u32)),
				next: None,
				synchronizer: self.render_backend.create_synchronizer(signaled),
			});

			if let Some(previous_synchronizer_handle) = previous_synchronizer_handle {
				self.synchronizers[previous_synchronizer_handle.0 as usize].next = Some(synchronizer_handle);
			}

			previous_synchronizer_handle = Some(synchronizer_handle);
		}

		synchronizer_handle
	}

	/// Acquires an image from the swapchain as to have it ready for presentation.
	/// 
	/// # Arguments
	/// 
	/// * `frame_handle` - The frame to acquire the image for. If `None` is passed, the image will be acquired for the next frame.
	/// * `synchronizer_handle` - The synchronizer to wait for before acquiring the image. If `None` is passed, the image will be acquired immediately.
	///
	/// # Panics
	///
	/// Panics if .
	pub fn acquire_swapchain_image(&mut self, frame_handle: Option<FrameHandle>, synchronizer_handle: SynchronizerHandle) -> u32 {
		let synchronizer = self.get_synchronizer(frame_handle, synchronizer_handle);
		let (index, swapchain_state) = self.render_backend.acquire_swapchain_image(&self.surface.as_ref().unwrap().swapchain, &synchronizer.synchronizer);

		if swapchain_state == render_backend::SwapchainStates::Suboptimal || swapchain_state == render_backend::SwapchainStates::Invalid {
			panic!("Swapchain out of date");
		}

		index
	}

	pub fn get_swapchain_texture_handle(&self, frame_handle: Option<FrameHandle>) -> TextureHandle {
		let surface = self.surface.as_ref().unwrap();
		surface.textures[frame_handle.unwrap().0 as usize]
	}

	pub fn execute(&self, frame_handle: Option<FrameHandle>, command_buffer_recording: CommandBufferRecording, wait_for_synchronizer_handle: Option<SynchronizerHandle>, signal_synchronizer_handle: Option<SynchronizerHandle>, execution_synchronizer_handle: SynchronizerHandle) {
		let command_buffer = self.get_command_buffer(frame_handle, command_buffer_recording.command_buffer);

		let wait_for_synchronizer = if let Some(wait_for_synchronizer) = wait_for_synchronizer_handle {
			Some(&self.get_synchronizer(frame_handle, wait_for_synchronizer).synchronizer)
		} else {
			None
		};

		let signal_synchronizer = if let Some(signal_synchronizer) = signal_synchronizer_handle {
			Some(&self.get_synchronizer(frame_handle, signal_synchronizer).synchronizer)
		} else {
			None
		};

		let execution_synchronizer = self.get_synchronizer(frame_handle, execution_synchronizer_handle);

		self.render_backend.execute(&command_buffer.command_buffer, wait_for_synchronizer, signal_synchronizer, &execution_synchronizer.synchronizer);
	}

	pub fn present(&self, frame_handle: Option<FrameHandle>, image_index: u32, synchronizer_handle: SynchronizerHandle) {
		let synchronizer = &self.get_synchronizer(frame_handle, synchronizer_handle);
		self.render_backend.present(&self.surface.as_ref().unwrap().swapchain,  &synchronizer.synchronizer, image_index);
	}

	pub fn wait(&self, frame_handle: Option<FrameHandle>, synchronizer_handle: SynchronizerHandle) {
		let synchronizer = &self.get_synchronizer(frame_handle, synchronizer_handle);
		self.render_backend.wait(&synchronizer.synchronizer);
	}

	pub fn start_frame_capture(&self) {
		self.debugger.start_frame_capture();
	}

	pub fn end_frame_capture(&self) {
		self.debugger.end_frame_capture();
	}

	fn get_command_buffer(&self, frame_handle: Option<FrameHandle>, resource_handle: CommandBufferHandle) -> &CommandBuffer {
		let mut resource = &self.command_buffers[resource_handle.0 as usize];

		loop {
			if resource.frame_handle == frame_handle { break; }

			if let Some(next) = resource.next {
				resource = &self.command_buffers[next.0 as usize];
			} else {
				panic!("Resource not found");
			}
		}

		resource
	}

	fn get_command_buffer_handle(&self, frame_handle: Option<FrameHandle>, resource_handle: CommandBufferHandle) -> CommandBufferHandle {
		let mut resource = &self.command_buffers[resource_handle.0 as usize];
		let mut command_buffer_handle = resource_handle;

		loop {
			if resource.frame_handle == frame_handle { break; }

			if let Some(next) = resource.next {
				resource = &self.command_buffers[next.0 as usize];
				command_buffer_handle = next;
			} else {
				panic!("Resource not found");
			}
		}

		command_buffer_handle
	}

	fn get_buffer_handle(&self, frame_handle: Option<FrameHandle>, resource_handle: BufferHandle) -> BufferHandle {
		let mut resource = &self.buffers[resource_handle.0 as usize];
		let mut buffer_handle = resource_handle;

		loop {
			if resource.frame_handle == frame_handle { break; }

			if let Some(next) = resource.next {
				resource = &self.buffers[next.0 as usize];
				buffer_handle = next;
			} else {
				panic!("Resource not found");
			}
		}

		buffer_handle
	}

	fn get_texture(&self, frame_handle: Option<FrameHandle>, resource_handle: TextureHandle) -> &Texture {
		let mut resource = &self.textures[resource_handle.0 as usize];

		loop {
			if resource.frame_handle == frame_handle { break; }

			if let Some(next) = resource.next {
				resource = &self.textures[next.0 as usize];
			} else {
				panic!("Resource not found");
			}
		}

		resource
	}

	fn get_synchronizer(&self, frame_handle: Option<FrameHandle>, resource_handle: SynchronizerHandle) -> &Synchronizer {
		let mut resource = &self.synchronizers[resource_handle.0 as usize];

		loop {
			if resource.frame_handle == frame_handle { break; }

			if let Some(next) = resource.next {
				resource = &self.synchronizers[next.0 as usize];
			} else {
				panic!("Resource not found");
			}
		}

		resource
	}
}

// TODO: handle resizing

impl orchestrator::Entity for RenderSystem {}
impl System for RenderSystem {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RGBAu8 {
	r: u8,
	g: u8,
	b: u8,
	a: u8,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn render_triangle() {
		let mut renderer = RenderSystem::new();

		let frame_handle = renderer.create_frame();

		let signal = renderer.create_synchronizer(false);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: true },
			VertexElement{ name: "COLOR".to_string(), format: crate::render_system::DataTypes::Float4, shuffled: true },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0, 1, 2].as_ptr() as *const u8, 3 * 4),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_texture(extent, crate::render_backend::TextureFormats::RGBAu8, crate::render_backend::Uses::RenderTarget, crate::render_system::DeviceAccesses::CpuRead | crate::render_system::DeviceAccesses::GpuWrite);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(pipeline_layout, &[&vertex_shader, &fragment_shader], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(Some(frame_handle), command_buffer_handle);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		command_buffer_recording.start_render_pass(extent, &attachments);

		command_buffer_recording.bind_pipeline(&pipeline);

		command_buffer_recording.draw_mesh(&mesh);

		command_buffer_recording.end_render_pass();

		command_buffer_recording.synchronize_texture(render_target);

		command_buffer_recording.end();

		renderer.execute(Some(frame_handle), command_buffer_recording, None, None, signal);

		renderer.end_frame_capture();

		renderer.wait(Some(frame_handle), signal); // Wait for the render to finish before accessing the texture data

		// assert colored triangle was drawn to texture
		let pixels = renderer.get_texture_data::<RGBAu8>(render_target);

		// let mut file = std::fs::File::create("test.png").unwrap();

		// let mut encoder = png::Encoder::new(&mut file, extent.width, extent.height);

		// encoder.set_color(png::ColorType::Rgba);
		// encoder.set_depth(png::BitDepth::Eight);

		// let mut writer = encoder.write_header().unwrap();
		// writer.write_image_data(unsafe { std::slice::from_raw_parts(pixels.as_ptr() as *const u8, pixels.len() * 4) }).unwrap();

		assert_eq!(pixels.len(), (extent.width * extent.height) as usize);

		let pixel = pixels[0]; // top left
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 0, a: 255 });

		if extent.width % 2 != 0 {
			let pixel = pixels[(extent.width / 2) as usize]; // middle top center
			assert_eq!(pixel, RGBAu8 { r: 255, g: 0, b: 0, a: 255 });
		}
		
		let pixel = pixels[(extent.width - 1) as usize]; // top right
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 0, a: 255 });
		
		let pixel = pixels[(extent.width  * (extent.height - 1)) as usize]; // bottom left
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 255, a: 255 });
		
		let pixel = pixels[(extent.width * extent.height - (extent.width / 2)) as usize]; // middle bottom center
		assert!(pixel == RGBAu8 { r: 0, g: 127, b: 127, a: 255 } || pixel == RGBAu8 { r: 0, g: 128, b: 127, a: 255 }); // FIX: workaround for CI, TODO: make near equal function
		
		let pixel = pixels[(extent.width * extent.height - 1) as usize]; // bottom right
		assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });

		assert!(!renderer.has_errors())
	}

	#[ignore = "Ignore until we have a way to disable this test in CI where presentation is not supported"]
	#[test]
	fn present() {
		let mut renderer = RenderSystem::new();

		let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let window_handle = window_system.create_window("Renderer Test", extent, "test");

		renderer.bind_to_window(window_system.get_os_handles(&window_handle));

		let frame_handle = renderer.create_frame();

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: true },
			VertexElement{ name: "COLOR".to_string(), format: crate::render_system::DataTypes::Float4, shuffled: true },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0, 1, 2].as_ptr() as *const u8, 3 * 4),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		let render_target = renderer.create_texture(extent, crate::render_backend::TextureFormats::RGBAu8, crate::render_backend::Uses::RenderTarget, crate::render_system::DeviceAccesses::GpuWrite);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::BGRAu8,
				clear: None,
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(pipeline_layout, &[&vertex_shader, &fragment_shader], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		let render_finished_synchronizer = renderer.create_synchronizer(false);
		let image_ready = renderer.create_synchronizer(false);

		let image_index = renderer.acquire_swapchain_image(Some(frame_handle), image_ready);

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(Some(frame_handle), command_buffer_handle);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		command_buffer_recording.start_render_pass(extent, &attachments);

		command_buffer_recording.bind_pipeline(&pipeline);

		command_buffer_recording.draw_mesh(&mesh);

		command_buffer_recording.end_render_pass();

		let swapchain_texture_handle = renderer.get_swapchain_texture_handle(Some(frame_handle));

		command_buffer_recording.copy_textures(&[(render_target, swapchain_texture_handle,)]);

		command_buffer_recording.end();

		renderer.execute(Some(frame_handle), command_buffer_recording, Some(image_ready), Some(render_finished_synchronizer), render_finished_synchronizer);

		renderer.present(Some(frame_handle), image_index, render_finished_synchronizer);

		renderer.end_frame_capture();

		renderer.wait(Some(frame_handle), render_finished_synchronizer);

		// TODO: assert rendering results

		assert!(!renderer.has_errors())
	}

	#[test]
	fn multiframe_rendering() {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		let mut renderer = RenderSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let frames = (0..FRAMES_IN_FLIGHT).map(|_| renderer.create_frame()).collect::<Vec<_>>();

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: true },
			VertexElement{ name: "COLOR".to_string(), format: crate::render_system::DataTypes::Float4, shuffled: true },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0, 1, 2].as_ptr() as *const u8, 3 * 4),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_texture(extent, crate::render_backend::TextureFormats::RGBAu8, crate::render_backend::Uses::RenderTarget, crate::render_system::DeviceAccesses::CpuRead | crate::render_system::DeviceAccesses::GpuWrite);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(pipeline_layout, &[&vertex_shader, &fragment_shader], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		let render_finished_synchronizer = renderer.create_synchronizer(true);

		for i in 0..FRAMES_IN_FLIGHT*10 {
			renderer.wait(Some(frames[i % FRAMES_IN_FLIGHT]), render_finished_synchronizer);

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(Some(frames[i % FRAMES_IN_FLIGHT]), command_buffer_handle);

			let attachments = [
				crate::render_system::AttachmentInfo {
					texture: render_target,
					format: crate::render_backend::TextureFormats::RGBAu8,
					clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				}
			];

			command_buffer_recording.start_render_pass(extent, &attachments);

			command_buffer_recording.bind_pipeline(&pipeline);

			command_buffer_recording.draw_mesh(&mesh);

			command_buffer_recording.end_render_pass();

			command_buffer_recording.synchronize_texture(render_target);

			command_buffer_recording.end();

			renderer.execute(Some(frames[i % FRAMES_IN_FLIGHT]), command_buffer_recording, None, None, render_finished_synchronizer);

			renderer.end_frame_capture();
		}

		assert!(!renderer.has_errors())
	}

	// TODO: Test changing frames in flight count during rendering

	#[test]
	fn dynamic_data() {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		let mut renderer = RenderSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let frames = (0..FRAMES_IN_FLIGHT).map(|_| renderer.create_frame()).collect::<Vec<_>>();

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: true },
			VertexElement{ name: "COLOR".to_string(), format: crate::render_system::DataTypes::Float4, shuffled: true },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0, 1, 2].as_ptr() as *const u8, 3 * 4),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(row_major) uniform;

			layout(push_constant) uniform PushConstants {
				mat4 matrix;
			} push_constants;

			void main() {
				out_color = in_color;
				gl_Position = push_constants.matrix * vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_texture(extent, crate::render_backend::TextureFormats::RGBAu8, crate::render_backend::Uses::RenderTarget, crate::render_system::DeviceAccesses::CpuRead | crate::render_system::DeviceAccesses::GpuWrite);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(pipeline_layout, &[&vertex_shader, &fragment_shader], &vertex_layout, &attachments);

		let buffer = renderer.create_buffer(64, crate::render_backend::Uses::Storage, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead);

		let command_buffer_handle = renderer.create_command_buffer();

		let render_finished_synchronizer = renderer.create_synchronizer(true);

		for i in 0..FRAMES_IN_FLIGHT*10 {
			renderer.wait(Some(frames[i % FRAMES_IN_FLIGHT]), render_finished_synchronizer);

			//let pointer = renderer.get_buffer_pointer(Some(frames[i % FRAMES_IN_FLIGHT]), buffer);

			//unsafe { std::ptr::copy_nonoverlapping(matrix.as_ptr(), pointer as *mut f32, 16); }

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(Some(frames[i % FRAMES_IN_FLIGHT]), command_buffer_handle);

			let attachments = [
				crate::render_system::AttachmentInfo {
					texture: render_target,
					format: crate::render_backend::TextureFormats::RGBAu8,
					clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				}
			];

			command_buffer_recording.start_render_pass(extent, &attachments);

			command_buffer_recording.bind_pipeline(&pipeline);
			
			let angle = (i as f32) / 10.0;

			let matrix: [f32; 16] = 
				[
					angle.cos(), -angle.sin(), 0f32, 0f32,
					angle.sin(), angle.cos(), 0f32, 0f32,
					0f32, 0f32, 1f32, 0f32,
					0f32, 0f32, 0f32, 1f32,
				];

			command_buffer_recording.write_to_push_constant(&pipeline_layout, 0, unsafe { std::slice::from_raw_parts(matrix.as_ptr() as *const u8, 16 * 4) });

			command_buffer_recording.draw_mesh(&mesh);

			command_buffer_recording.end_render_pass();

			command_buffer_recording.synchronize_texture(render_target);

			command_buffer_recording.end();

			renderer.execute(Some(frames[i % FRAMES_IN_FLIGHT]), command_buffer_recording, None, None, render_finished_synchronizer);

			renderer.end_frame_capture();
		}

		assert!(!renderer.has_errors())
	}

	#[test]
	fn descriptor_sets() {
		let mut renderer = RenderSystem::new();

		let frame_handle = renderer.create_frame();

		let signal = renderer.create_synchronizer(false);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: true },
			VertexElement{ name: "COLOR".to_string(), format: crate::render_system::DataTypes::Float4, shuffled: true },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0, 1, 2].as_ptr() as *const u8, 3 * 4),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450 core
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0, binding=1) uniform UniformBufferObject {
				mat4 matrix;
			} ubo;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450 core
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0,binding=0) uniform sampler smpl;
			layout(set=0,binding=2) uniform texture2D tex;

			void main() {
				out_color = texture(sampler2D(tex, smpl), vec2(0, 0));
			}
		";

		let vertex_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(crate::render_system::ShaderSourceType::GLSL, fragment_shader_code.as_bytes());

		let buffer = renderer.create_buffer(64, render_backend::Uses::Uniform, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead);

		let sampled_texture = renderer.create_texture(crate::Extent { width: 2, height: 2, depth: 1 }, crate::render_backend::TextureFormats::RGBAu8, crate::render_backend::Uses::Texture, crate::render_system::DeviceAccesses::CpuWrite | crate::render_system::DeviceAccesses::GpuRead);

		let pixels = vec![
			RGBAu8 { r: 255, g: 0, b: 0, a: 255 },
			RGBAu8 { r: 0, g: 255, b: 0, a: 255 },
			RGBAu8 { r: 0, g: 0, b: 255, a: 255 },
			RGBAu8 { r: 255, g: 255, b: 0, a: 255 },
		];

		let sampler = renderer.create_sampler();

		let bindings = [
			DescriptorSetLayoutBinding {
				descriptor_count: 1,
				descriptor_type: render_backend::DescriptorType::Sampler,
				binding: 0,
				stage_flags: render_backend::Stages::FRAGMENT,
				immutable_samplers: Some(vec![sampler]),
			},
			DescriptorSetLayoutBinding {
				descriptor_count: 1,
				descriptor_type: render_backend::DescriptorType::UniformBuffer,
				binding: 1,
				stage_flags: render_backend::Stages::VERTEX,
				immutable_samplers: None,
			},
			DescriptorSetLayoutBinding {
				descriptor_count: 1,
				descriptor_type: render_backend::DescriptorType::SampledImage,
				binding: 2,
				stage_flags: render_backend::Stages::FRAGMENT,
				immutable_samplers: None,
			},
		];

		let descriptor_set_layout_handle = renderer.create_descriptor_set_layout(&bindings);

		let descriptor_set = renderer.create_descriptor_set(&descriptor_set_layout_handle, &bindings);

		renderer.write(&[
			DescriptorWrite { descriptor_set: descriptor_set, binding: 1, array_element: 0, descriptor: Descriptor::Buffer(buffer) },
			DescriptorWrite { descriptor_set: descriptor_set, binding: 2, array_element: 0, descriptor: Descriptor::Texture(sampled_texture) },
		]);

		let pipeline_layout = renderer.create_pipeline_layout(&[descriptor_set_layout_handle]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_texture(extent, crate::render_backend::TextureFormats::RGBAu8, crate::render_backend::Uses::RenderTarget, crate::render_system::DeviceAccesses::CpuRead | crate::render_system::DeviceAccesses::GpuWrite);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(pipeline_layout, &[&vertex_shader, &fragment_shader], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(Some(frame_handle), command_buffer_handle);

		command_buffer_recording.write_texture_data(sampled_texture, &pixels);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		command_buffer_recording.start_render_pass(extent, &attachments);

		command_buffer_recording.bind_pipeline(&pipeline);

		command_buffer_recording.bind_descriptor_set(pipeline_layout, 0, &descriptor_set);

		command_buffer_recording.draw_mesh(&mesh);

		command_buffer_recording.end_render_pass();

		command_buffer_recording.synchronize_texture(render_target);

		command_buffer_recording.end();

		renderer.execute(Some(frame_handle), command_buffer_recording, None, None, signal);

		renderer.end_frame_capture();

		renderer.wait(Some(frame_handle), signal); // Wait for the render to finish before accessing the texture data

		// assert colored triangle was drawn to texture
		let pixels = renderer.get_texture_data::<RGBAu8>(render_target);

		// TODO: assert rendering results

		assert!(!renderer.has_errors())
	}
}