use utils::Extent;

use crate::{
	buffer, descriptors, image,
	pipelines::VertexElement,
	sampler,
	shader::{self, Sources},
	window, AllocationHandle, BaseBufferHandle, BottomLevelAccelerationStructure, BottomLevelAccelerationStructureHandle,
	BufferHandle, CommandBufferHandle, DescriptorSetHandle, DeviceAccesses, DynamicBufferHandle, DynamicImageHandle, Formats,
	ImageHandle, MeshHandle, PipelineHandle, PresentationModes, QueueHandle, SamplerHandle, ShaderHandle, ShaderTypes,
	Size as _, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TopLevelAccelerationStructureHandle, Uses,
};

/// The `TextureReadback` struct owns the bytes and layout from one completed texture-transfer invocation.
///
/// `bytes_per_row` and `bytes_per_image` describe the authoritative layout in `bytes`.
/// Callers must use these values instead of deriving strides from the extent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureReadback {
	/// The copied bytes for this transfer invocation.
	pub bytes: Vec<u8>,
	/// The copied source extent.
	pub extent: Extent,
	/// The copied source format.
	pub format: Formats,
	/// The byte distance between adjacent rows.
	pub bytes_per_row: usize,
	/// The byte distance between adjacent depth slices or array layers.
	pub bytes_per_image: usize,
}

/// Reports why a texture transfer could not be recorded or mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureTransferError {
	/// The selected backend does not implement the requested transfer source.
	Unsupported,
	/// The source handle does not identify a transferable image or swapchain in this context.
	InvalidSource,
	/// The source format has no supported buffer-copy layout.
	UnsupportedFormat(Formats),
	/// The requested mip, extent, or layer count is outside the current transfer contract.
	UnsupportedSubresource,
	/// The physical source resource was not created for transfer-source access.
	MissingTransferSource,
	/// The source layout cannot be represented without overflow.
	UnsupportedLayout,
	/// The transfer handle does not identify a live transfer in this context.
	InvalidHandle(TextureCopyHandle),
	/// The backend could not allocate CPU-readable transfer storage.
	AllocationFailed,
	/// The backend could not synchronize or map transfer storage.
	MappingFailed,
}

impl std::fmt::Display for TextureTransferError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let message = match self {
			Self::Unsupported => "Texture transfer is unsupported. The most likely cause is that the backend cannot read this source.",
			Self::InvalidSource => "Texture transfer source is invalid. The most likely cause is that its handle is stale or belongs to another context.",
			Self::UnsupportedFormat(_) => "Texture transfer format is unsupported. The most likely cause is that it has no color buffer-copy layout.",
			Self::UnsupportedSubresource => "Texture transfer subresource is unsupported. The most likely cause is that it is not one base-mip 2D layer.",
			Self::MissingTransferSource => "Texture transfer source usage is missing. The most likely cause is that the image lacks TransferSource use.",
			Self::UnsupportedLayout => "Texture transfer layout is unsupported. The most likely cause is that its byte layout overflows.",
			Self::InvalidHandle(_) => "Texture transfer handle is invalid. The most likely cause is that it is stale, consumed, or belongs to another context.",
			Self::AllocationFailed => "Texture transfer allocation failed. The most likely cause is that CPU-readable staging memory is unavailable.",
			Self::MappingFailed => "Texture transfer mapping failed. The most likely cause is that recording was not submitted or GPU synchronization failed.",
		};
		formatter.write_str(message)
	}
}

impl std::error::Error for TextureTransferError {}

/// The `TextureTransferLayout` struct provides the checked compact layout for one supported transfer source.
pub(crate) struct TextureTransferLayout {
	pub(crate) bytes_per_row: usize,
	pub(crate) row_count: usize,
	pub(crate) bytes_per_image: usize,
}

/// Validates the base-mip, single-layer 2D color transfer contract and computes its compact layout.
pub(crate) fn texture_transfer_layout(
	format: Formats,
	extent: Extent,
	array_layers: u32,
	uses: Uses,
) -> Result<TextureTransferLayout, TextureTransferError> {
	if !uses.contains(Uses::TransferSource) {
		return Err(TextureTransferError::MissingTransferSource);
	}
	if extent.width() == 0 || extent.height() == 0 || extent.depth() > 1 || array_layers != 1 {
		return Err(TextureTransferError::UnsupportedSubresource);
	}
	if format.is_depth()
		|| matches!(
			format,
			Formats::RGB8F
				| Formats::RGB8UNORM
				| Formats::RGB8SNORM
				| Formats::RGB8sRGB
				| Formats::RGB16F
				| Formats::RGB16UNORM
				| Formats::RGB16SNORM
				| Formats::RGB16sRGB
		) {
		return Err(TextureTransferError::UnsupportedFormat(format));
	}

	let (bytes_per_row, row_count) = if let Some(bytes_per_block) = format.bc_bytes_per_block() {
		let blocks_w =
			usize::try_from(extent.width().max(1).div_ceil(4)).map_err(|_| TextureTransferError::UnsupportedLayout)?;
		let blocks_h =
			usize::try_from(extent.height().max(1).div_ceil(4)).map_err(|_| TextureTransferError::UnsupportedLayout)?;
		(
			blocks_w
				.checked_mul(bytes_per_block as usize)
				.ok_or(TextureTransferError::UnsupportedLayout)?,
			blocks_h,
		)
	} else {
		(
			usize::try_from(extent.width())
				.map_err(|_| TextureTransferError::UnsupportedLayout)?
				.checked_mul(format.size())
				.ok_or(TextureTransferError::UnsupportedLayout)?,
			usize::try_from(extent.height()).map_err(|_| TextureTransferError::UnsupportedLayout)?,
		)
	};
	let bytes_per_image = bytes_per_row
		.checked_mul(row_count)
		.ok_or(TextureTransferError::UnsupportedLayout)?;

	Ok(TextureTransferLayout {
		bytes_per_row,
		row_count,
		bytes_per_image,
	})
}

enum TextureReadbackState<T> {
	Recorded(T),
	Submitted(T),
	MappingFailed,
	Vacant,
}

impl<T> TextureReadbackState<T> {
	fn value(&self) -> Option<&T> {
		match self {
			Self::Recorded(value) | Self::Submitted(value) => Some(value),
			Self::MappingFailed | Self::Vacant => None,
		}
	}

	fn value_mut(&mut self) -> Option<&mut T> {
		match self {
			Self::Recorded(value) | Self::Submitted(value) => Some(value),
			Self::MappingFailed | Self::Vacant => None,
		}
	}

	fn submit(&mut self) -> bool {
		let Self::Recorded(value) = std::mem::replace(self, Self::Vacant) else {
			return false;
		};
		*self = Self::Submitted(value);
		true
	}
}

struct TextureReadbackSlot<T> {
	generation: u32,
	state: TextureReadbackState<T>,
}

/// The `TextureReadbackRegistry` struct provides reusable generational slots for context-local transfer handles.
pub(crate) struct TextureReadbackRegistry<T> {
	slots: Vec<TextureReadbackSlot<T>>,
	free: Vec<u32>,
}

impl<T> TextureReadbackRegistry<T> {
	pub(crate) fn new() -> Self {
		Self {
			slots: Vec::new(),
			free: Vec::new(),
		}
	}

	/// Inserts one recorded readback and returns a generation-qualified stable handle.
	pub(crate) fn insert(&mut self, value: T) -> TextureCopyHandle {
		if let Some(index) = self.free.pop() {
			let slot = &mut self.slots[index as usize];
			slot.generation = slot.generation.checked_add(1).expect(
				"Texture readback generation overflowed. The most likely cause is that one registry slot was reused more than u32::MAX times.",
			);
			slot.state = TextureReadbackState::Recorded(value);
			return Self::handle(index, slot.generation);
		}

		let index = u32::try_from(self.slots.len()).expect(
			"Texture readback registry exhausted its handle index space. The most likely cause is more than u32::MAX simultaneous transfers.",
		);
		let generation = 1;
		self.slots.push(TextureReadbackSlot {
			generation,
			state: TextureReadbackState::Recorded(value),
		});
		Self::handle(index, generation)
	}

	pub(crate) fn mark_submitted(&mut self, handle: TextureCopyHandle) -> bool {
		self.slot_mut(handle).is_some_and(|slot| slot.state.submit())
	}

	pub(crate) fn get(&self, handle: TextureCopyHandle) -> Option<&T> {
		self.slot(handle)?.state.value()
	}

	pub(crate) fn get_mut(&mut self, handle: TextureCopyHandle) -> Option<&mut T> {
		self.slot_mut(handle)?.state.value_mut()
	}

	pub(crate) fn submitted(&self, handle: TextureCopyHandle) -> Result<&T, TextureTransferError> {
		match self.slot(handle).map(|slot| &slot.state) {
			Some(TextureReadbackState::Submitted(value)) => Ok(value),
			Some(TextureReadbackState::Recorded(_) | TextureReadbackState::MappingFailed) => {
				Err(TextureTransferError::MappingFailed)
			}
			Some(TextureReadbackState::Vacant) | None => Err(TextureTransferError::InvalidHandle(handle)),
		}
	}

	pub(crate) fn take_submitted(&mut self, handle: TextureCopyHandle) -> Result<T, TextureTransferError> {
		self.submitted(handle)?;
		let (index, _) = Self::parts(handle);
		let slot = self
			.slot_mut(handle)
			.expect("A validated texture readback slot must remain available.");
		let TextureReadbackState::Submitted(value) = std::mem::replace(&mut slot.state, TextureReadbackState::Vacant) else {
			unreachable!();
		};
		if slot.generation != u32::MAX {
			self.free.push(index);
		}
		Ok(value)
	}

	/// Releases an unsubmitted value and leaves a reusable failed slot for deterministic mapping errors.
	pub(crate) fn abandon_recorded(&mut self, handle: TextureCopyHandle) -> Option<T> {
		let (index, generation) = Self::parts(handle);
		let slot = self.slots.get_mut(index as usize)?;
		if slot.generation != generation || !matches!(slot.state, TextureReadbackState::Recorded(_)) {
			return None;
		}
		let TextureReadbackState::Recorded(value) = std::mem::replace(&mut slot.state, TextureReadbackState::MappingFailed)
		else {
			unreachable!();
		};
		if slot.generation != u32::MAX {
			self.free.push(index);
		}
		Some(value)
	}

	pub(crate) fn values(&self) -> impl Iterator<Item = &T> {
		self.slots.iter().filter_map(|slot| slot.state.value())
	}

	pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
		self.slots.iter_mut().filter_map(|slot| slot.state.value_mut())
	}

	pub(crate) fn entries(&self) -> impl Iterator<Item = (TextureCopyHandle, &T)> {
		self.slots.iter().enumerate().filter_map(|(index, slot)| {
			slot.state
				.value()
				.map(|value| (Self::handle(index as u32, slot.generation), value))
		})
	}

	fn slot(&self, handle: TextureCopyHandle) -> Option<&TextureReadbackSlot<T>> {
		let (index, generation) = Self::parts(handle);
		self.slots.get(index as usize).filter(|slot| slot.generation == generation)
	}

	fn slot_mut(&mut self, handle: TextureCopyHandle) -> Option<&mut TextureReadbackSlot<T>> {
		let (index, generation) = Self::parts(handle);
		self.slots
			.get_mut(index as usize)
			.filter(|slot| slot.generation == generation)
	}

	fn handle(index: u32, generation: u32) -> TextureCopyHandle {
		TextureCopyHandle((u64::from(generation) << 32) | u64::from(index))
	}

	fn parts(handle: TextureCopyHandle) -> (u32, u32) {
		(handle.0 as u32, (handle.0 >> 32) as u32)
	}
}

/// The `Context` trait identifies objects that own render resources created from a GPU device.
/// Implementations use the context lifetime to bound the lifetime of owned GPU resources.
///
/// Create resources through [`ContextCreate`], obtain a command
/// buffer with [`Self::command_buffer`], then submit recorded work through a
/// queue returned by [`Self::queue`] or [`Self::queue_reference`].
pub trait Context: ContextCreate {
	type Queue: crate::queue::Queue;
	type QueueReference<'a>: crate::queue::Queue
	where
		Self: 'a;
	type CommandBuffer<'a>: crate::command_buffer::CommandBuffer
	where
		Self: 'a;

	/// Returns whether the underlying API has encountered any errors.
	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool;

	/// Returns whether the GPU supports BC5 and BC7 block-compressed textures.
	///
	/// Check this value before you create BC-compressed images or samplers.
	fn supports_bc_texture_compression(&self) -> bool;

	/// Returns an owned queue wrapper that exposes queue-local command submission.
	fn queue(&mut self, queue_handle: QueueHandle) -> Self::Queue;

	/// Returns a borrowed queue wrapper that exposes queue-local command submission.
	fn queue_reference<'a>(&'a mut self, queue_handle: QueueHandle) -> Self::QueueReference<'a>;

	/// Returns a command-buffer wrapper that exposes command-buffer-local recording.
	fn command_buffer<'a>(&'a mut self, command_buffer_handle: CommandBufferHandle) -> Self::CommandBuffer<'a>;

	/// Changes the maximum number of frames in flight.
	///
	/// This expensive operation can create more frame resources.
	fn set_frames_in_flight(&mut self, frames: u8);

	/// Returns a device accessible address for the provided buffer handle.
	fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64;

	/// Returns a shared view into a typed buffer's contents.
	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &T;

	/// Returns a mutable view into CPU-visible buffer contents.
	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: BufferHandle<T>) -> &'static mut T;

	/// Flushes or uploads pending writes for the provided buffer.
	fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>);

	/// Returns mutable CPU access to an image's backing bytes.
	fn get_texture_slice_mut(&self, texture_handle: ImageHandle) -> &'static mut [u8];

	/// Flushes or uploads pending writes for the provided image.
	fn sync_texture(&mut self, image_handle: ImageHandle);

	/// Enables writes to a texture and queues a copy operation.
	///
	/// Call `sync` on a command buffer before the GPU uses the texture.
	fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8]));

	/// Updates retained descriptor-set state before command recording.
	///
	/// Rendering only binds complete retained sets; resource overrides are not recorded per draw.
	fn write(&mut self, descriptor_set_writes: &[descriptors::DescriptorWrite]);

	/// Writes one top-level acceleration-structure instance into an instance buffer.
	fn write_instance(
		&mut self,
		instances_buffer_handle: BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: BottomLevelAccelerationStructureHandle,
	);

	/// Writes one shader binding table entry for the provided pipeline shader.
	fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: PipelineHandle,
		shader_handle: ShaderHandle,
	);

	/// Associates a swapchain with a window.
	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		fallback_extent: Extent,
		uses: Uses,
	) -> SwapchainHandle;

	/// Waits for queued GPU work, consumes one transfer handle, and returns its owned result.
	///
	/// A transfer handle is local to the context that created it. Handle values can overlap across contexts,
	/// so pass the handle only to that same context. Record the transfer with
	/// [`crate::command_buffer::CommandBufferRecording::transfer_texture`] and submit its command before calling
	/// this method. Successful mapping releases the backend staging resource. The handle is then consumed even
	/// though it is copyable; a second call with the same value returns [`TextureTransferError::InvalidHandle`].
	fn get_image_data(&mut self, texture_copy_handle: TextureCopyHandle) -> Result<TextureReadback, TextureTransferError>;

	/// Resizes a dynamic buffer to the specified size.
	fn resize_buffer<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>, size: usize);

	/// Starts capturing the underlying's API calls if the application is attached to a graphics debugger.
	fn start_frame_capture(&mut self);

	/// Ends capturing the underlying's API calls if the application is attached to a graphics debugger.
	fn end_frame_capture(&mut self);

	/// Waits for operations associated with one synchronizer to complete.
	///
	/// Call this after submitting a command buffer when later CPU work depends only on that submission.
	fn wait_for_synchronizer(&mut self, synchronizer: SynchronizerHandle);

	/// Waits for all pending operations to complete.
	fn wait(&self);
}

/// The `ContextCreate` trait provides creation operations for resources owned by a GHI context.
pub trait ContextCreate {
	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: Uses,
		resource_device_accesses: DeviceAccesses,
	) -> AllocationHandle;

	/// Uploads indexed mesh data and returns a reusable mesh handle.
	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[VertexElement],
	) -> MeshHandle;

	/// Creates a shader and returns its handle.
	///
	/// # Errors
	///
	/// Returns an error when GLSL compilation fails or SPIR-V input is not aligned
	/// to four bytes.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = shader::ShaderResourceDescriptor>,
	) -> Result<ShaderHandle, ()>;

	/// Creates an empty retained descriptor set.
	///
	/// The set is a lifetime/update grouping only. Its shader-visible slots are established by
	/// [`Context::write`] calls and validated against the active pipeline when it is bound.
	fn create_descriptor_set(&mut self, name: Option<&str>) -> DescriptorSetHandle;

	/// Creates a graphics/rasterization pipeline from a builder.
	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> PipelineHandle;

	/// Creates a compute pipeline.
	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> PipelineHandle;

	/// Creates a ray-tracing pipeline.
	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> PipelineHandle;

	/// Creates a static fixed-size buffer from a builder.
	/// Static buffers are not resizable; use [`ContextCreate::build_dynamic_buffer`] when the allocation must grow.
	fn build_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> BufferHandle<T>;

	/// Creates a dynamic buffer from a builder.
	/// Dynamic buffers can be resized with [`Context::resize_buffer`].
	fn build_dynamic_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> DynamicBufferHandle<T>;

	/// Creates a dynamic image from a builder.
	fn build_dynamic_image(&mut self, builder: image::Builder) -> DynamicImageHandle;

	/// Creates an image from a builder.
	fn build_image(&mut self, builder: image::Builder) -> ImageHandle;

	/// Creates an image sampler from a builder.
	///
	/// Devices can limit their sampler count. Reuse samplers when possible.
	fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle;

	/// Creates a buffer that stores top-level acceleration-structure instances.
	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> BaseBufferHandle;

	/// Creates a top-level acceleration structure for ray tracing.
	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle;

	/// Creates a bottom-level acceleration structure from geometry descriptions.
	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &BottomLevelAccelerationStructure,
	) -> BottomLevelAccelerationStructureHandle;

	/// Creates a synchronization primitive (implemented as a semaphore/fence/event).\
	/// Multiple underlying synchronization primitives are created, one for each frame
	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle;
}

#[cfg(test)]
mod texture_transfer_tests {
	use super::*;

	#[test]
	fn layout_accepts_supported_2d_images() {
		for depth in [0, 1] {
			let layout = texture_transfer_layout(Formats::RGBA8UNORM, Extent::new(3, 2, depth), 1, Uses::TransferSource)
				.expect("A single 2D color layer with TransferSource use must have a readback layout.");
			assert_eq!((layout.bytes_per_row, layout.row_count, layout.bytes_per_image), (12, 2, 24));
		}
	}

	#[test]
	fn layout_rejects_unsupported_sources() {
		let extent = Extent::rectangle(1, 1);
		for (format, extent, layers, uses, error) in [
			(
				Formats::RGBA8UNORM,
				Extent::new(1, 1, 2),
				1,
				Uses::TransferSource,
				TextureTransferError::UnsupportedSubresource,
			),
			(
				Formats::RGBA8UNORM,
				extent,
				2,
				Uses::TransferSource,
				TextureTransferError::UnsupportedSubresource,
			),
			(
				Formats::Depth32,
				extent,
				1,
				Uses::TransferSource,
				TextureTransferError::UnsupportedFormat(Formats::Depth32),
			),
			(
				Formats::RGB8UNORM,
				extent,
				1,
				Uses::TransferSource,
				TextureTransferError::UnsupportedFormat(Formats::RGB8UNORM),
			),
			(
				Formats::RGBA8UNORM,
				extent,
				1,
				Uses::Image,
				TextureTransferError::MissingTransferSource,
			),
			(
				Formats::RGBA16F,
				Extent::rectangle(u32::MAX, u32::MAX),
				1,
				Uses::TransferSource,
				TextureTransferError::UnsupportedLayout,
			),
		] {
			assert_eq!(texture_transfer_layout(format, extent, layers, uses).map(|_| ()), Err(error));
		}
	}

	#[test]
	fn unsubmitted_and_abandoned_handles_fail_mapping_without_leaking_storage() {
		let mut registry = TextureReadbackRegistry::new();
		let handle = registry.insert(7_u32);
		assert_eq!(registry.take_submitted(handle), Err(TextureTransferError::MappingFailed));
		assert_eq!(registry.get(handle), Some(&7));
		assert_eq!(registry.abandon_recorded(handle), Some(7));
		assert_eq!(registry.take_submitted(handle), Err(TextureTransferError::MappingFailed));
		assert_eq!(registry.values().count(), 0);
	}

	#[test]
	fn consumed_handle_is_stale_after_slot_reuse() {
		let mut registry = TextureReadbackRegistry::new();
		let stale = registry.insert(1_u32);
		assert!(registry.mark_submitted(stale));
		assert_eq!(registry.take_submitted(stale), Ok(1));

		let current = registry.insert(2_u32);
		assert_ne!(stale, current);
		assert_eq!(
			registry.take_submitted(stale),
			Err(TextureTransferError::InvalidHandle(stale))
		);
	}

	#[test]
	fn repeated_transfers_reuse_one_slot() {
		let mut registry = TextureReadbackRegistry::new();
		let mut previous = None;

		for value in 0..1_024_u32 {
			let handle = registry.insert(value);
			assert_ne!(previous, Some(handle));
			assert!(registry.mark_submitted(handle));
			assert_eq!(registry.take_submitted(handle), Ok(value));
			previous = Some(handle);
		}

		assert_eq!(registry.slots.len(), 1);
		assert_eq!(registry.free.len(), 1);
	}
}
