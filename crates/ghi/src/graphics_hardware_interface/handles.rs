//! Backend-independent GHI handles.

// HANDLES

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct QueueHandle(pub(crate) u64);

/// The `BaseBufferHandle` struct identifies a static buffer without exposing its element type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug, PartialOrd, Ord)]
pub struct BaseBufferHandle(pub(crate) u64);

impl MasterHandle for BaseBufferHandle {
	fn new(i: u64) -> Self {
		BaseBufferHandle(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

/// The `BufferHandle` struct identifies a static buffer with its element type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BufferHandle<T>(pub(crate) BaseBufferHandle, pub(crate) std::marker::PhantomData<T>);

/// The `DynamicBufferHandle` struct identifies a resizable buffer with its element type.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct DynamicBufferHandle<T>(pub(crate) BaseBufferHandle, pub(crate) std::marker::PhantomData<T>);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BaseImageHandle(pub(crate) u64);

impl MasterHandle for BaseImageHandle {
	fn new(i: u64) -> Self {
		BaseImageHandle(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

impl From<BaseImageHandle> for Handles {
	fn from(value: BaseImageHandle) -> Self {
		Handles::Image(ImageHandle(value))
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageHandle(pub(crate) BaseImageHandle);

impl From<ImageHandle> for BaseImageHandle {
	fn from(value: ImageHandle) -> Self {
		value.0
	}
}

/// The `DynamicImageHandle` struct addresses a frame-local image that can be written independently for each frame in flight.
#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct DynamicImageHandle(pub(crate) BaseImageHandle);

impl From<DynamicImageHandle> for BaseImageHandle {
	fn from(value: DynamicImageHandle) -> Self {
		value.0
	}
}

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct TopLevelAccelerationStructureHandle(pub(crate) u64);

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
pub struct BottomLevelAccelerationStructureHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CommandBufferHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ShaderHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PipelineHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MeshHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SynchronizerHandle(pub(crate) u64);

impl MasterHandle for SynchronizerHandle {
	fn new(i: u64) -> Self {
		Self(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// The `DescriptorSetHandle` struct identifies a retained group of flat shader resource writes.
pub struct DescriptorSetHandle(pub(crate) u64);

/// The `PipelineLayoutHandle` struct identifies a pipeline resource layout.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineLayoutHandle(pub(crate) u64);

/// The `SamplerHandle` struct identifies an image sampler.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SamplerHandle(pub(crate) u64);

/// The `SwapchainHandle` struct identifies a presentation swapchain.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwapchainHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AllocationHandle(pub(crate) u64);

/// The `TextureCopyHandle` struct identifies one texture-transfer invocation within its creating context.
///
/// Handle values can overlap across contexts, so pass a handle only to the [`crate::Context`] that created it.
/// Submit the command that returned the handle, then pass it once to [`crate::Context::get_image_data`]. Successful
/// mapping consumes the handle value and releases backend staging; later mapping attempts return
/// [`crate::TextureTransferError::InvalidHandle`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextureCopyHandle(pub(crate) u64);

impl<T: bytemuck::Pod> From<BufferHandle<T>> for BaseBufferHandle {
	fn from(val: BufferHandle<T>) -> Self {
		val.0
	}
}

impl<T: bytemuck::Pod> From<DynamicBufferHandle<T>> for BaseBufferHandle {
	fn from(val: DynamicBufferHandle<T>) -> Self {
		val.0
	}
}

impl From<DynamicImageHandle> for Handles {
	fn from(val: DynamicImageHandle) -> Self {
		val.0.into()
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Handles {
	Buffer(BaseBufferHandle),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	CommandBuffer(CommandBufferHandle),
	Shader(ShaderHandle),
	Pipeline(PipelineHandle),
	Image(ImageHandle),
	Mesh(MeshHandle),
	Synchronizer(SynchronizerHandle),

	DescriptorSet(DescriptorSetHandle),
	PipelineLayout(PipelineLayoutHandle),
	Sampler(SamplerHandle),
	Swapchain(SwapchainHandle),
	Allocation(AllocationHandle),
	TextureCopy(TextureCopyHandle),
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
}

impl From<BaseBufferHandle> for Handles {
	fn from(val: BaseBufferHandle) -> Self {
		Handles::Buffer(val)
	}
}

impl From<ImageHandle> for Handles {
	fn from(val: ImageHandle) -> Self {
		Handles::Image(val)
	}
}

impl From<SynchronizerHandle> for Handles {
	fn from(val: SynchronizerHandle) -> Self {
		Handles::Synchronizer(val)
	}
}

pub(crate) trait MasterHandle: Sized + Copy {
	fn new(i: u64) -> Self;
	fn index(&self) -> u64;
}

impl<T: bytemuck::Pod> MasterHandle for BufferHandle<T> {
	fn new(i: u64) -> Self {
		Self(BaseBufferHandle(i), std::marker::PhantomData)
	}

	fn index(&self) -> u64 {
		self.0.0
	}
}

pub(crate) trait PrivateHandle: Copy {
	fn new(i: u64) -> Self;
	fn index(&self) -> u64;
}
