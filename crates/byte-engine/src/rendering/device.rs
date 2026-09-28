//! The GHI device that the rendering and loading timelines both come from.

use ghi::Device as _;

use crate::application::parameters::Parameters;

/// The `GraphicsDevice` struct is the shared root of the rendering and loading timelines.
///
/// Rendering and loading each own a context of this device and submit on a queue of their own: the renderer on
/// the graphics queue and the [`Loader`](crate::rendering::loading::Loader) on the copy queue. Neither timeline
/// waits on the other. Loaded resources reach the renderer only through the loader's notifications. Keep this
/// value alive until both contexts have been dropped. The [loading timeline design](/docs/develop/rendering/loading)
/// explains why the timelines stay independent.
///
/// Next, create the renderer with [`Renderer::new`](crate::rendering::Renderer::new) and the loader with
/// [`Loader::new`](crate::rendering::loading::Loader::new).
pub struct GraphicsDevice {
	/// Declared before the instance so the device drops first.
	device: ghi::implementation::Device,
	_instance: ghi::implementation::Instance,
	graphics_queue: ghi::QueueHandle,
	copy_queue: ghi::QueueHandle,
}

impl GraphicsDevice {
	/// Creates the device, its graphics queue, and its copy queue from application parameters.
	///
	/// # Parameters
	/// - `render.debug`: Enables validation layers for debugging. Defaults to true on debug builds.
	/// - `render.debug.dump`: Enables API dump for debugging. Defaults to false.
	/// - `render.debug.extended`: Enables extended validation for debugging. Defaults to false.
	/// - `render.debug.labels`: Enables graphics API object labels and command debug groups. Defaults to `render.debug`.
	/// - `render.ghi.features.mesh-shading`: Enables mesh shading features on the device. Defaults to true.
	pub fn new(parameters: &dyn Parameters) -> Self {
		let flag = |name: &str, default: bool| {
			parameters
				.get_parameter(name)
				.map_or(default, |parameter| parameter.as_bool_simple())
		};
		let validation = flag("render.debug", cfg!(debug_assertions));

		let mut features = ghi::device::Features::new()
			.validation(validation)
			.api_dump(flag("render.debug.dump", false))
			.gpu_validation(flag("render.debug.extended", false))
			.debug_labels(flag("render.debug.labels", validation))
			.debug_log_function(log_graphics_error)
			.geometry_shader(false)
			.mesh_shading(flag("render.ghi.features.mesh-shading", true));

		let mut instance = match ghi::implementation::Instance::new(features) {
			Ok(instance) => instance,
			Err(error) if validation => {
				log::warn!(
					"Renderer validation was requested but could not be enabled: {error} Falling back to renderer validation disabled. The most likely cause is missing or unsupported platform graphics tooling. See {}.",
					crate::online_docs_url("use/setup/environment")
				);
				features = features
					.validation(false)
					.gpu_validation(false)
					.api_dump(false)
					.debug_labels(false);
				ghi::implementation::Instance::new(features).unwrap()
			}
			Err(error) => panic!("Failed to create GHI instance: {error}"),
		};

		let mut graphics_queue = None;
		let mut copy_queue = None;
		let device = instance
			.create_device(
				features,
				&mut [
					(
						ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER),
						&mut graphics_queue,
					),
					(ghi::QueueSelection::new(ghi::types::WorkloadTypes::TRANSFER), &mut copy_queue),
				],
			)
			.unwrap();

		Self {
			device,
			_instance: instance,
			graphics_queue: graphics_queue.unwrap(),
			copy_queue: copy_queue.unwrap(),
		}
	}

	/// Creates a context on this device for a timeline of its own.
	///
	/// Each timeline owns one context and submits only on its own queue. Resources move between the contexts of one
	/// device with [`export_image`](ghi::implementation::Context::export_image) and
	/// [`share_buffer`](ghi::implementation::Context::share_buffer).
	pub fn create_context(&self) -> ghi::implementation::Context {
		self.device.create_context().expect(
			"Failed to create a GHI context. The most likely cause is that the graphics device was lost or ran out of memory.",
		)
	}

	/// Returns the queue the renderer records and presents frames on.
	#[must_use]
	pub fn graphics_queue(&self) -> ghi::QueueHandle {
		self.graphics_queue
	}

	/// Returns the queue the loader submits its upload batches on.
	#[must_use]
	pub fn copy_queue(&self) -> ghi::QueueHandle {
		self.copy_queue
	}
}

/// Logs a graphics API error with the backtrace frames that belong to this workspace.
fn log_graphics_error(message: &str) {
	let backtrace = std::backtrace::Backtrace::force_capture().to_string();
	let manifest_dir = env!("CARGO_MANIFEST_DIR");
	let workspace_root = manifest_dir
		.rsplit_once("/crates/")
		.map(|(root, _)| root)
		.unwrap_or(manifest_dir);

	let mut filtered = String::new();
	for line in backtrace.lines() {
		if line.contains(workspace_root) {
			filtered.push_str(line);
			filtered.push('\n');
		}
	}

	if filtered.trim().is_empty() {
		log::error!("{}\n{}", message, backtrace);
	} else {
		log::error!("{}\n{}", message, filtered.trim_end());
	}
}
