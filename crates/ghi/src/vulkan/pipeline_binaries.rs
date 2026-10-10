//! The `pipeline_binaries` module creates pipelines through `VK_KHR_pipeline_binary`, so a pipeline compiled once is
//! created from stored binaries afterwards instead of from its shaders.
//!
//! [`create_pipeline`] is the entry point every pipeline kind goes through. The binaries live in a [`store::Store`],
//! which [`PipelineBinaries::load`] adopts from an earlier run and [`PipelineBinaries::data`] lends out to be saved.

use std::sync::RwLock;

use ash::vk;
use smallvec::SmallVec;

use self::store::{Key, Store};

mod store;

/// The `PipelineBinaries` struct is the application-owned pipeline cache that a device, its contexts, and its
/// factories share.
///
/// It exists only on devices created with [`crate::device::Features::pipeline_cache`] whose driver supports
/// `VK_KHR_pipeline_binary`. Pass it to [`create_pipeline`] to create pipelines through it.
pub(crate) struct PipelineBinaries {
	functions: ash::khr::pipeline_binary::Device,
	/// Names the driver configuration the binaries belong to. Binaries of another configuration are unusable.
	global_key: Key,
	/// Compiler threads create pipelines concurrently, and most creations only read.
	store: RwLock<Store>,
}

impl PipelineBinaries {
	/// Starts an empty cache for `device`, or returns `None` when the driver cannot name its configuration.
	pub(crate) fn new(instance: &ash::Instance, device: &ash::Device) -> Option<Self> {
		let functions = ash::khr::pipeline_binary::Device::load(instance, device);
		// A null create info asks for the global key.
		let global_key = pipeline_key(&functions, std::ptr::null())?;
		Some(Self {
			functions,
			global_key,
			store: RwLock::new(Store::new(&global_key)),
		})
	}

	/// Replaces the cache with a blob [`Self::data`] lent out, and keeps the blob as the cache's storage.
	///
	/// A blob of another driver configuration is dropped.
	pub(crate) fn load(&self, blob: Vec<u8>) {
		*self.write() = Store::load(&self.global_key, blob);
	}

	/// Lends the cache's bytes to `read` without copying them. See [`crate::pipelines::CacheData`].
	pub(crate) fn data<R>(&self, read: impl FnOnce(crate::pipelines::CacheData<'_>) -> R) -> R {
		let store = self.read();
		let (loaded, added) = store.bytes();
		read(crate::pipelines::CacheData { loaded, added })
	}

	fn read(&self) -> std::sync::RwLockReadGuard<'_, Store> {
		// The store is only ever replaced or extended whole, so it is still consistent after a panic elsewhere.
		self.store.read().unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	fn write(&self) -> std::sync::RwLockWriteGuard<'_, Store> {
		self.store.write().unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	/// Creates the native binary objects of a stored pipeline, or returns `None` when the pipeline is not stored or the
	/// driver rejects its binaries.
	fn stored_binaries(&self, pipeline: &Key) -> Option<Binaries<'_>> {
		// The driver reads the binaries straight from the store, which stays locked until it has.
		let store = self.read();
		let (keys, data): (SmallVec<[_; 4]>, SmallVec<[_; 4]>) = store
			.binaries(pipeline)?
			.map(|(key, bytes)| {
				// The driver only reads the data. The pointer is mutable because the structure also returns data elsewhere.
				let data = vk::PipelineBinaryDataKHR {
					data_size: bytes.len(),
					p_data: bytes.as_ptr().cast_mut().cast(),
					..Default::default()
				};
				(to_vk_key(&key), data)
			})
			.unzip();
		let keys_and_data = vk::PipelineBinaryKeysAndDataKHR::default()
			.pipeline_binary_keys(&keys)
			.pipeline_binary_data(&data);
		let create_info = vk::PipelineBinaryCreateInfoKHR::default().keys_and_data_info(&keys_and_data);
		self.create_binaries(&create_info, keys.len())
	}

	/// Stores the binaries of a pipeline created with `VK_PIPELINE_CREATE_2_CAPTURE_DATA_BIT_KHR`, then lets the
	/// pipeline drop the data it kept for this.
	///
	/// The driver writes each binary straight into the store, and a binary the store already holds, such as a shader
	/// another pipeline shares, is not read at all. When the driver fails any step, the pipeline stays out of the
	/// store and is compiled again in the next run.
	fn capture(&self, pipeline: vk::Pipeline, key: Key) {
		let create_info = vk::PipelineBinaryCreateInfoKHR::default().pipeline(pipeline);
		// Without storage, the driver only reports how many binaries the pipeline has.
		let mut count_info = vk::PipelineBinaryHandlesInfoKHR::default();
		// SAFETY: The create info names a live pipeline, and a handles info without storage only receives the count.
		let counted = unsafe {
			(self.functions.fp().create_pipeline_binaries_khr)(
				self.functions.device(),
				&create_info,
				std::ptr::null(),
				&mut count_info,
			)
		};

		if counted == vk::Result::SUCCESS
			&& let Some(binaries) = self.create_binaries(&create_info, count_info.pipeline_binary_count as usize)
		{
			// Without storage, the driver reports each binary's key and the size of its data.
			let sized = binaries
				.handles
				.iter()
				.map(|&binary| {
					let mut size = 0;
					Some((self.binary_data(binary, &mut size, std::ptr::null_mut())?, size))
				})
				.collect::<Option<SmallVec<[_; 4]>>>();
			if let Some(sized) = sized {
				// SAFETY: A successful call wrote `size` bytes, which is the length of the slice the store passes.
				unsafe {
					self.write().insert(key, &sized, |position, data| {
						let mut size = data.len();
						self.binary_data(binaries.handles[position], &mut size, data.as_mut_ptr().cast())
							.is_some() && size == data.len()
					});
				}
			}
		}

		let release_info = vk::ReleaseCapturedPipelineDataInfoKHR::default().pipeline(pipeline);
		// Releasing only frees memory early, so a failure costs nothing but that memory.
		// SAFETY: The pipeline is live and no other thread uses it before this function returns.
		let _ = unsafe {
			(self.functions.fp().release_captured_pipeline_data_khr)(self.functions.device(), &release_info, std::ptr::null())
		};
	}

	/// Creates the `count` native binary objects `create_info` describes: those of a pipeline that captured its data,
	/// or those of stored keys and data.
	fn create_binaries(&self, create_info: &vk::PipelineBinaryCreateInfoKHR<'_>, count: usize) -> Option<Binaries<'_>> {
		// Handles the driver does not create stay null, which `Binaries` may destroy.
		let mut binaries = Binaries {
			functions: &self.functions,
			handles: SmallVec::from_elem(vk::PipelineBinaryKHR::null(), count),
		};
		let mut handles_info = vk::PipelineBinaryHandlesInfoKHR::default().pipeline_binaries(&mut binaries.handles);
		// SAFETY: The caller's create info is valid, and the handles info points at storage for `count` handles.
		let result = unsafe {
			(self.functions.fp().create_pipeline_binaries_khr)(
				self.functions.device(),
				create_info,
				std::ptr::null(),
				&mut handles_info,
			)
		};
		(result == vk::Result::SUCCESS).then_some(binaries)
	}

	/// Reads the key of a binary, and its data into `data` when that is not null.
	///
	/// `size` is the capacity of `data` going in and the size of the binary's data coming out.
	fn binary_data(&self, binary: vk::PipelineBinaryKHR, size: &mut usize, data: *mut std::ffi::c_void) -> Option<Key> {
		let info = vk::PipelineBinaryDataInfoKHR::default().pipeline_binary(binary);
		let mut key = vk::PipelineBinaryKeyKHR::default();
		// SAFETY: The callers pass a null `data`, or one that points at `size` writable bytes.
		let result =
			unsafe { (self.functions.fp().get_pipeline_binary_data_khr)(self.functions.device(), &info, &mut key, size, data) };
		(result == vk::Result::SUCCESS).then(|| from_vk_key(&key)).flatten()
	}
}

/// The `Binaries` struct owns the native binary objects of one pipeline for as long as creating or reading that
/// pipeline needs them.
///
/// A pipeline keeps nothing of the objects it was created from, so they are destroyed as soon as this value drops.
struct Binaries<'a> {
	functions: &'a ash::khr::pipeline_binary::Device,
	handles: SmallVec<[vk::PipelineBinaryKHR; 4]>,
}

impl Drop for Binaries<'_> {
	fn drop(&mut self) {
		for &handle in &self.handles {
			// SAFETY: Each handle is null or a binary this value created, and nothing else holds it.
			unsafe { (self.functions.fp().destroy_pipeline_binary_khr)(self.functions.device(), handle, std::ptr::null()) };
		}
	}
}

/// Creates a descriptor-heap pipeline, from stored binaries when `binaries` holds the pipeline and from its shaders
/// otherwise.
///
/// `create_info` must not carry a `VkPipelineCreateFlags2CreateInfo`; this function adds one with the descriptor-heap
/// flag every pipeline of this backend needs. `create` calls the creation function of the pipeline's kind and may run
/// twice, because a pipeline whose stored binaries the driver rejects is compiled from its shaders instead. A
/// pipeline compiled from shaders has its binaries stored for the next creation and the next run.
pub(crate) fn create_pipeline<T>(
	binaries: Option<&PipelineBinaries>,
	create_info: &T,
	create: impl Fn(&T) -> Result<vk::Pipeline, vk::Result>,
) -> Result<vk::Pipeline, vk::Result>
where
	T: Copy,
	for<'a> vk::PipelineCreateFlags2CreateInfo<'a>: vk::Extends<T>,
	for<'a> vk::PipelineBinaryInfoKHR<'a>: vk::Extends<T>,
{
	let heap_flags = vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT;
	let mut flags = vk::PipelineCreateFlags2CreateInfo::default().flags(heap_flags);
	// SAFETY: The bounds make the flags a valid extension of `T`, and `flags` outlives every use of `keyed`.
	let keyed = unsafe { chained(create_info, &mut flags) };
	let Some(binaries) = binaries else {
		return create(&keyed);
	};

	// The driver derives the key from the state that affects the binaries, so equal keys mean reusable binaries.
	let key_info = vk::PipelineCreateInfoKHR {
		p_next: (&raw const keyed).cast_mut().cast(),
		..Default::default()
	};
	let Some(key) = pipeline_key(&binaries.functions, &key_info) else {
		return create(&keyed);
	};

	if let Some(stored) = binaries.stored_binaries(&key) {
		let mut binary_info = vk::PipelineBinaryInfoKHR::default().pipeline_binaries(&stored.handles);
		// SAFETY: As above, and `binary_info` and the handles it points at outlive the creation call.
		if let Ok(pipeline) = create(&unsafe { chained(&keyed, &mut binary_info) }) {
			return Ok(pipeline);
		}
	}

	let mut capture_flags =
		vk::PipelineCreateFlags2CreateInfo::default().flags(heap_flags | vk::PipelineCreateFlags2::CAPTURE_DATA_KHR);
	// SAFETY: As above, and `capture_flags` outlives the creation call.
	let pipeline = create(&unsafe { chained(create_info, &mut capture_flags) })?;
	binaries.capture(pipeline, key);
	Ok(pipeline)
}

/// Returns a copy of `root` with `next` linked at the front of its extension chain.
///
/// This is `TaggedStructure::push` for callers that cannot name the lifetime `push` wants `next` borrowed for.
///
/// # Safety
///
/// `T` and `N` must be Vulkan structures that start with a structure type and a chain pointer, `N` must be a valid
/// extension of `T`, and `next` must stay alive and in place for as long as the returned copy is used.
unsafe fn chained<T: Copy, N>(root: &T, next: &mut N) -> T {
	let mut root = *root;
	let root_base = (&raw mut root).cast::<vk::BaseOutStructure<'_>>();
	let next_base = (&raw mut *next).cast::<vk::BaseOutStructure<'_>>();
	// SAFETY: The caller guarantees both structures have the base layout.
	unsafe {
		(*next_base).p_next = (*root_base).p_next;
		(*root_base).p_next = next_base;
	}
	root
}

/// Asks the driver for the key of a pipeline create info, or for the global key when `create_info` is null.
fn pipeline_key(
	functions: &ash::khr::pipeline_binary::Device,
	create_info: *const vk::PipelineCreateInfoKHR<'_>,
) -> Option<Key> {
	let mut key = vk::PipelineBinaryKeyKHR::default();
	// SAFETY: `create_info` is null or points at a live create info whose chain holds a valid pipeline create info.
	let result = unsafe { (functions.fp().get_pipeline_key_khr)(functions.device(), create_info, &mut key) };
	(result == vk::Result::SUCCESS).then(|| from_vk_key(&key)).flatten()
}

fn from_vk_key(key: &vk::PipelineBinaryKeyKHR<'_>) -> Option<Key> {
	Key::new(key.key.get(..key.key_size as usize)?)
}

fn to_vk_key(key: &Key) -> vk::PipelineBinaryKeyKHR<'static> {
	let bytes = key.bytes();
	let mut vk_key = vk::PipelineBinaryKeyKHR::default().key_size(bytes.len() as u32);
	vk_key.key[..bytes.len()].copy_from_slice(bytes);
	vk_key
}
