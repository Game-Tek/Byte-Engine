//! The `pipeline_binaries` module creates pipelines through `VK_KHR_pipeline_binary`, so a pipeline compiled once is
//! created from stored binaries afterwards instead of from its shaders.
//!
//! [`create_pipeline`] is the entry point every pipeline kind goes through. The binaries live in a [`store::Store`],
//! which [`PipelineBinaries::load`] adopts from an earlier run and [`PipelineBinaries::data`] lends out to be saved.

use std::sync::RwLock;

use ash::vk;
use smallvec::SmallVec;

pub(crate) use self::store::Key;
use self::store::Store;

mod store;

/// The `PipelineBinaries` struct is the application-owned pipeline cache that a device, its contexts, and its
/// factories share.
///
/// It exists only on devices created with [`crate::device::Features::pipeline_cache`] whose driver supports
/// `VK_KHR_pipeline_binary`. Pass it to [`create_pipeline`] to create pipelines through it.
pub(crate) struct PipelineBinaries {
	functions: ash::khr::pipeline_binary::Device,
	/// `VK_EXT_shader_module_identifier`, when the device has it. See [`Self::shader_identifier`].
	identifiers: Option<ash::ext::shader_module_identifier::Device>,
	/// Names the driver configuration the binaries belong to. Binaries of another configuration are unusable.
	global_key: Key,
	/// Compiler threads create pipelines concurrently, and most creations only read.
	store: RwLock<Store>,
}

impl PipelineBinaries {
	/// Starts an empty cache for `device`, or returns `None` when the driver cannot name its configuration.
	///
	/// `shader_identifiers` tells whether `VK_EXT_shader_module_identifier` is enabled on `device`.
	pub(crate) fn new(instance: &ash::Instance, device: &ash::Device, shader_identifiers: bool) -> Option<Self> {
		let functions = ash::khr::pipeline_binary::Device::load(instance, device);
		// A null create info asks for the global key.
		let global_key = pipeline_key(&functions, std::ptr::null())?;
		Some(Self {
			functions,
			identifiers: shader_identifiers.then(|| ash::ext::shader_module_identifier::Device::load(instance, device)),
			global_key,
			store: RwLock::new(Store::new(&global_key)),
		})
	}

	/// Returns the driver's identifier for a shader's SPIR-V, or `None` when the device cannot identify shaders.
	///
	/// A pipeline create info that names its shaders by identifier needs no code, so [`create_pipeline`] can look a
	/// pipeline up and create it from stored binaries without the driver reading SPIR-V again. Ask once per shader:
	/// the driver hashes the whole code to answer.
	pub(crate) fn shader_identifier(&self, spirv: &[u32]) -> Option<Key> {
		let identifiers = self.identifiers.as_ref()?;
		let create_info = vk::ShaderModuleCreateInfo::default().code(spirv);
		let mut identifier = vk::ShaderModuleIdentifierEXT::default();
		// SAFETY: The create info points at live SPIR-V, and the identifier is a writable local.
		unsafe {
			(identifiers.fp().get_shader_module_create_info_identifier_ext)(identifiers.device(), &create_info, &mut identifier)
		};
		let identifier = identifier.identifier.get(..identifier.identifier_size as usize)?;
		Key::new(identifier).filter(|identifier| !identifier.bytes().is_empty())
	}

	/// Returns the identifier of the shader whose code the caller names `key`, without the code when the cache
	/// remembers it.
	///
	/// `spirv` is only called for a key the cache has not seen: the driver then identifies the code, and the cache
	/// remembers the answer under `key` for this run and, once saved, for later ones. The identifier depends on the
	/// driver, which is why it lives in the cache, whose header names the driver, and not next to the shader.
	pub(crate) fn keyed_shader_identifier<'a>(
		&self,
		key: u64,
		spirv: impl FnOnce() -> Option<std::borrow::Cow<'a, [u32]>>,
	) -> Option<Key> {
		self.identifiers.as_ref()?;
		if let Some(identifier) = self.read().shader_identifier(key) {
			return Some(identifier);
		}
		let identifier = self.shader_identifier(&spirv()?)?;
		self.write().insert_shader(key, identifier);
		Some(identifier)
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
/// `compiled` carries the shaders' SPIR-V and is what a pipeline is compiled from. `identified` is the same pipeline
/// with its shaders named by identifier instead, when every shader has one. The pipeline is looked up and created
/// from stored binaries through `identified`, so the driver reads no SPIR-V unless it has to compile; without it,
/// `compiled` serves both purposes.
///
/// Pass `compiled` as `None` to try the cache alone. The result is then `VK_PIPELINE_COMPILE_REQUIRED` when the
/// pipeline has to be compiled, which tells the caller to come back with the code.
///
/// Neither create info may carry a `VkPipelineCreateFlags2CreateInfo`; this function adds one with the
/// descriptor-heap flag every pipeline of this backend needs, and `flags`, such as the flag that makes a ray tracing
/// library. `create` calls the creation function of the pipeline's kind and may run twice, because a pipeline whose
/// stored binaries the driver rejects is compiled from its shaders instead. A pipeline compiled from shaders has its
/// binaries stored for the next creation and the next run.
pub(crate) fn create_pipeline<T>(
	binaries: Option<&PipelineBinaries>,
	flags: vk::PipelineCreateFlags2,
	compiled: Option<&T>,
	identified: Option<&T>,
	create: impl Fn(&T) -> Result<vk::Pipeline, vk::Result>,
) -> Result<vk::Pipeline, vk::Result>
where
	T: Copy,
	for<'a> vk::PipelineCreateFlags2CreateInfo<'a>: vk::Extends<T>,
	for<'a> vk::PipelineBinaryInfoKHR<'a>: vk::Extends<T>,
{
	let heap_flags = vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT | flags;
	let compile = |extra_flags| {
		let compiled = compiled.ok_or(vk::Result::PIPELINE_COMPILE_REQUIRED)?;
		let mut flags = vk::PipelineCreateFlags2CreateInfo::default().flags(heap_flags | extra_flags);
		// SAFETY: The bounds make the flags a valid extension of `T`, and `flags` outlives the creation call.
		create(&unsafe { chained(compiled, &mut flags) })
	};
	let Some(binaries) = binaries else {
		return compile(vk::PipelineCreateFlags2::empty());
	};

	// Shaders named by identifier are only valid in a pipeline that must not compile, which a lookup never does.
	let (lookup, lookup_flags) = match (identified, compiled) {
		(Some(identified), _) => (
			identified,
			heap_flags | vk::PipelineCreateFlags2::FAIL_ON_PIPELINE_COMPILE_REQUIRED,
		),
		(None, Some(compiled)) => (compiled, heap_flags),
		(None, None) => return Err(vk::Result::PIPELINE_COMPILE_REQUIRED),
	};
	let mut flags = vk::PipelineCreateFlags2CreateInfo::default().flags(lookup_flags);
	// SAFETY: As above, and `flags` outlives every use of `keyed`.
	let keyed = unsafe { chained(lookup, &mut flags) };
	// The driver derives the key from the state that affects the binaries, so equal keys mean reusable binaries.
	let key_info = vk::PipelineCreateInfoKHR {
		p_next: (&raw const keyed).cast_mut().cast(),
		..Default::default()
	};
	let Some(key) = pipeline_key(&binaries.functions, &key_info) else {
		return compile(vk::PipelineCreateFlags2::empty());
	};

	if let Some(stored) = binaries.stored_binaries(&key) {
		// A pipeline created from binaries compiles nothing, and Vulkan rejects the lookup's flag that forbids compiling.
		let mut binary_flags = vk::PipelineCreateFlags2CreateInfo::default().flags(heap_flags);
		let mut binary_info = vk::PipelineBinaryInfoKHR::default().pipeline_binaries(&stored.handles);
		// SAFETY: As above, and both structures and the handles `binary_info` points at outlive the creation call.
		let from_binaries = unsafe { chained(&chained(lookup, &mut binary_flags), &mut binary_info) };
		if let Ok(pipeline) = create(&from_binaries) {
			return Ok(pipeline);
		}
	}

	let pipeline = compile(vk::PipelineCreateFlags2::CAPTURE_DATA_KHR)?;
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
