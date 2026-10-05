pub mod command_buffer;
pub mod context;
pub mod device;
pub mod factory;
pub mod frame;
pub mod instance;
mod io;
pub mod queue;

pub use self::command_buffer::*;
pub use self::context::*;
pub use self::factory::*;
pub use self::frame::*;
pub use self::instance::*;
pub(crate) use self::io::write_compressed_file;
pub use self::io::{ResourceIoQueue, ResourceIoTicket};
pub use self::queue::*;
mod utils;

/// The `Context` type alias exposes the live DX12 device through the cross-backend context name.
pub type Context = self::context::Device;

#[cfg(test)]
#[allow(
	clippy::drop_non_drop,
	reason = "DX12 tests explicitly end frame borrows before inspecting their devices."
)]
mod tests {
	use windows::Win32::Graphics::Direct3D12::D3D12_BARRIER_SYNC_COMPUTE_SHADING;

	use super::*;
	use crate::command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _,
		RasterizationRenderPassMode as _,
	};
	use crate::context::Context as _;
	use crate::frame::Frame as _;
	use crate::queue::{Queue as _, QueueExecution as _};

	/// Reports the test message by panicking with it, since a plain `fn(&str)` log callback has no state to record into.
	/// The test catches the unwind and reads the message from the panic payload.
	fn panic_on_dx12_debug_test_message(message: &str) {
		if message.contains("ghi dx12 test application message") {
			panic!("{message}");
		}
	}

	fn create_default_device_setup() -> Option<(Instance, Device, crate::QueueHandle)> {
		let features = crate::device::Features::new().validation(false);
		create_device_setup_with_features(features)
	}

	fn create_validated_device_setup() -> Option<(Instance, Device, crate::QueueHandle)> {
		create_device_setup_with_features(crate::device::Features::new().validation(true))
	}

	fn create_device_setup_with_features(features: crate::device::Features) -> Option<(Instance, Device, crate::QueueHandle)> {
		let mut instance = Instance::new(features).ok()?;
		let mut queue_handle = None;
		let device = instance
			.create_device(
				features,
				&mut [(
					crate::QueueSelection::new(crate::types::WorkloadTypes::RASTER),
					&mut queue_handle,
				)],
			)
			.ok()?;
		Some((instance, device, queue_handle?))
	}

	/// Times `operation` while `blocker` stalls the queue, then releases the queue.
	///
	/// A watchdog releases the queue after five seconds, so a regression that waits on the blocked work fails instead of
	/// hanging.
	fn time_with_blocked_queue(
		blocker: &windows::Win32::Graphics::Direct3D12::ID3D12Fence,
		operation: impl FnOnce(),
	) -> std::time::Duration {
		let watchdog_fence = blocker.clone();
		let (cancel_watchdog, watchdog_cancelled) = std::sync::mpsc::channel();
		let watchdog = std::thread::spawn(move || {
			if watchdog_cancelled.recv_timeout(std::time::Duration::from_secs(5)).is_err() {
				unsafe { watchdog_fence.Signal(1) }.expect(
					"Failed to release the DX12 test queue. The most likely cause is that the test device was removed.",
				);
			}
		});
		let start = std::time::Instant::now();
		operation();
		let elapsed = start.elapsed();

		unsafe { blocker.Signal(1) }
			.expect("Failed to release the DX12 test queue. The most likely cause is that the test device was removed.");
		let _ = cancel_watchdog.send(());
		watchdog.join().expect(
			"DX12 queue watchdog panicked. The most likely cause is that the test fence could not release blocked work.",
		);
		elapsed
	}

	/// Creates a device with one compute queue and one transfer queue, or returns `None` where DX12 is unavailable.
	fn create_compute_transfer_device_setup(
		features: crate::device::Features,
	) -> Option<(Instance, Device, crate::QueueHandle, crate::QueueHandle)> {
		let mut instance = Instance::new(features).ok()?;
		let mut compute_queue = None;
		let mut transfer_queue = None;
		let device = instance
			.create_device(
				features,
				&mut [
					(crate::QueueSelection::new(crate::WorkloadTypes::COMPUTE), &mut compute_queue),
					(
						crate::QueueSelection::new(crate::WorkloadTypes::TRANSFER),
						&mut transfer_queue,
					),
				],
			)
			.ok()?;
		Some((
			instance,
			device,
			compute_queue.expect("DX12 compute queue creation must return its GHI handle."),
			transfer_queue.expect("DX12 transfer queue creation must return its GHI handle."),
		))
	}

	/// Builds a compute pipeline from SPIR-V-less shader metadata, so descriptor tests exercise only the binding layout.
	fn create_metadata_compute_pipeline(
		device: &mut Device,
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
		resources: impl IntoIterator<Item = crate::ShaderResourceDescriptor>,
	) -> crate::PipelineHandle {
		let shader = device
			.create_shader(
				None,
				crate::shader::Sources::SPIRV(&[]),
				crate::ShaderTypes::Compute,
				resources,
			)
			.expect("Failed to create DX12 shader metadata.");
		device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			push_constant_ranges,
			crate::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		))
	}

	#[test]
	fn debug_info_queue_messages_use_device_log_function() {
		let features = crate::device::Features::new()
			.validation(true)
			.debug_log_function(panic_on_dx12_debug_test_message);
		let Some((_instance, device, _queue_handle)) = create_device_setup_with_features(features) else {
			return;
		};

		let logged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.add_debug_message_for_test("ghi dx12 test application message");
		}));

		let payload = logged.expect_err("The DX12 info-queue message should reach the device log function.");
		let message = payload
			.downcast_ref::<String>()
			.expect("The log function should panic with the formatted message.");
		assert!(message.contains("ghi dx12 test application message"));
	}

	#[test]
	fn dropped_frame_recording_preserves_implicit_dynamic_image_upload() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1))
				.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		{
			let mut frame = device.start_frame(0, synchronizer);
			let recording = frame.create_command_buffer_recording(command_buffer);
			drop(recording);
		}

		let copy = {
			let mut frame = device.start_frame(0, synchronizer);
			let mut recording = frame.create_command_buffer_recording(command_buffer);
			let copy = crate::command_buffer::CommandBufferRecording::transfer_texture(&mut recording, image.into()).expect(
				"Texture transfer failed. The most likely cause is that the DX12 test image is not a valid transfer source.",
			);
			crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
			copy
		};
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(
			device
				.get_image_data(copy)
				.expect(
					"Texture mapping failed. The most likely cause is that the dropped recording lost the dynamic image upload."
				)
				.bytes,
			[0, 0, 0, 0]
		);
		assert!(!device.has_errors());
	}

	#[test]
	fn dropped_render_target_recording_restores_the_next_barrier_origin() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		let extent = ::utils::Extent::rectangle(1, 1);
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::RenderTarget | crate::Uses::TransferSource,
			)
			.extent(extent)
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		let mut recording = device.create_command_buffer_recording(command_buffer);
		let discarded_attachment = crate::AttachmentInformation::new(
			image,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Clear(crate::ClearValue::Color(::utils::RGBA::new(1.0, 0.0, 0.0, 1.0))),
			crate::StoreOp::Store,
		);
		crate::command_buffer::CommandBufferRecording::start_render_pass(&mut recording, extent, &[discarded_attachment])
			.end_render_pass();
		drop(recording);

		let mut recording = device.create_command_buffer_recording(command_buffer);
		let submitted_attachment = crate::AttachmentInformation::new(
			image,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Clear(crate::ClearValue::Color(::utils::RGBA::new(0.0, 1.0, 0.0, 1.0))),
			crate::StoreOp::Store,
		);
		crate::command_buffer::CommandBufferRecording::start_render_pass(&mut recording, extent, &[submitted_attachment])
			.end_render_pass();
		let copy = crate::command_buffer::CommandBufferRecording::transfer_texture(&mut recording, image.into()).expect(
			"Texture transfer failed. The most likely cause is that the DX12 test image is not a valid transfer source.",
		);
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(
			device
				.get_image_data(copy)
				.expect(
					"Texture mapping failed. The most likely cause is that the submitted render-target clear did not execute."
				)
				.bytes,
			[0, 255, 0, 255]
		);
		assert!(!device.has_errors());
	}

	#[test]
	fn frame_recording_flushes_only_current_sequence_texture_uploads() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer_0 = device.create_command_buffer(None, queue_handle);
		let command_buffer_1 = device.create_command_buffer(None, queue_handle);

		device
			.texture_slice_mut_for_sequence(image.into(), 0)
			.copy_from_slice(&[1, 2, 3, 4]);
		device
			.texture_slice_mut_for_sequence(image.into(), 1)
			.copy_from_slice(&[5, 6, 7, 8]);
		device.queue_texture_sync_for_sequence(image.into(), 0);
		device.queue_texture_sync_for_sequence(image.into(), 1);

		{
			let mut frame = device.start_frame(1, synchronizer);
			let recording = frame.create_command_buffer_recording(command_buffer_1);
			drop(recording);
			drop(frame);
		}

		assert_eq!(device.upload_resource_count(), 1);

		{
			let mut frame = device.start_frame(0, synchronizer);
			let recording = frame.create_command_buffer_recording(command_buffer_0);
			drop(recording);
			drop(frame);
		}

		assert_eq!(device.upload_resource_count(), 2);
	}

	#[test]
	fn frame_recording_without_implicit_sync_leaves_pending_texture_uploads_queued() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let transfer_command_buffer = device.create_command_buffer(None, queue_handle);
		let render_command_buffer = device.create_command_buffer(None, queue_handle);

		device
			.texture_slice_mut_for_sequence(image.into(), 0)
			.copy_from_slice(&[9, 10, 11, 12]);
		device.queue_texture_sync_for_sequence(image.into(), 0);

		{
			let mut frame = device.start_frame(0, synchronizer);
			let recording = frame.create_command_buffer_recording_without_implicit_sync(transfer_command_buffer);
			drop(recording);
			drop(frame);
		}

		assert_eq!(device.upload_resource_count(), 0);

		{
			let mut frame = device.start_frame(0, synchronizer);
			let recording = frame.create_command_buffer_recording(render_command_buffer);
			drop(recording);
			drop(frame);
		}

		assert_eq!(device.upload_resource_count(), 1);
	}

	#[test]
	fn shrinking_frames_drops_retired_pending_texture_syncs() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		device.set_frames_in_flight(3);
		device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);

		assert_eq!(device.pending_texture_sync_count(), 3);

		device.set_frames_in_flight(2);

		assert_eq!(device.pending_texture_sync_count(), 2);
	}

	#[test]
	fn combined_image_sampler_writes_preserve_frame_offset() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let set = device.create_descriptor_set(None);
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		let sampler = device.build_sampler(crate::sampler::Builder::new());
		device.write(&[crate::DescriptorWrite::combined_image_sampler_with_frame(
			set,
			slot,
			image,
			sampler,
			crate::Layouts::Read,
			-1,
		)]);

		assert_eq!(device.descriptor_sequence_index(set, 0, slot), Some(1));
		assert_eq!(device.descriptor_sequence_index(set, 1, slot), Some(0));
	}

	#[test]
	fn growing_frames_extends_existing_descriptor_sets() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let set = device.create_descriptor_set(None);
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		let sampler = device.build_sampler(crate::sampler::Builder::new());
		device.write(&[crate::DescriptorWrite::combined_image_sampler_with_frame(
			set,
			slot,
			image,
			sampler,
			crate::Layouts::Read,
			-1,
		)]);

		device.set_frames_in_flight(3);

		assert_eq!(device.descriptor_sequence_index(set, 2, slot), Some(1));
		device.write(&[crate::DescriptorWrite::combined_image_sampler_with_frame(
			set,
			slot,
			image,
			sampler,
			crate::Layouts::Read,
			1,
		)]);
		assert_eq!(device.descriptor_sequence_index(set, 2, slot), Some(0));
	}

	#[test]
	fn descriptor_arrays_keep_declared_dx12_slot_count() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let resource = crate::ShaderResourceDescriptor::new(
			slot,
			crate::ResourceKind::CombinedImageSampler,
			1024,
			crate::AccessPolicies::READ,
		);
		let pipeline = create_metadata_compute_pipeline(&mut device, &[], [resource]);

		assert_eq!(device.pipeline_descriptor_counts(pipeline), Some((1024, 1024)));
		assert_eq!(device.pipeline_descriptor_slot(pipeline, slot, 1023, false), Some(1023));
		assert_eq!(device.pipeline_descriptor_slot(pipeline, slot, 1024, false), None);
		assert_eq!(device.pipeline_descriptor_slot(pipeline, slot, 1023, true), Some(1023));
		assert_eq!(device.pipeline_descriptor_slot(pipeline, slot, 1024, true), None);
	}

	#[test]
	fn descriptor_texture_syncs_and_states_are_sequence_local() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let resource =
			crate::ShaderResourceDescriptor::single(slot, crate::ResourceKind::SampledImage, crate::AccessPolicies::READ);
		let set = device.create_descriptor_set(None);
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		device.write(&[crate::DescriptorWrite::image(set, slot, image, crate::Layouts::Read)]);
		device.queue_texture_sync_for_sequence(image.into(), 0);
		device.queue_texture_sync_for_sequence(image.into(), 1);
		let pipeline = create_metadata_compute_pipeline(&mut device, &[], [resource]);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let expected_state = TextureBarrierState::shader_resource(D3D12_BARRIER_SYNC_COMPUTE_SHADING);
		let synchronizer = device.create_synchronizer(None, false);

		{
			let mut recording = device.create_command_buffer_recording(command_buffer);
			recording.bind_compute_pipeline(pipeline).bind_descriptor_sets(&[set]);
			crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		}

		assert_eq!(device.upload_resource_count(), 1);
		assert_eq!(device.pending_texture_sync_count(), 1);
		assert_eq!(
			device.tracked_image_resource_state_for_sequence(crate::ImageHandle(image.into()), 0),
			Some(expected_state)
		);
		assert_eq!(
			device.tracked_image_resource_state_for_sequence(crate::ImageHandle(image.into()), 1),
			None
		);

		device.begin_command_buffer(command_buffer, 1);
		{
			let mut recording = super::command_buffer::CommandBufferRecording::new(
				&mut device,
				command_buffer,
				Some(crate::FrameKey {
					frame_index: 1,
					sequence_index: 1,
				}),
			);
			recording.bind_compute_pipeline(pipeline).bind_descriptor_sets(&[set]);
			crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		}

		assert_eq!(device.upload_resource_count(), 2);
		assert_eq!(device.pending_texture_sync_count(), 0);
		assert_eq!(
			device.tracked_image_resource_state_for_sequence(crate::ImageHandle(image.into()), 1),
			Some(expected_state)
		);
	}

	#[test]
	fn pipeline_switches_revalidate_retained_descriptor_sets() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let first_slot = crate::ResourceSlot::new(0);
		let second_slot = crate::ResourceSlot::new(1);
		let first_resource = crate::ShaderResourceDescriptor::single(
			first_slot,
			crate::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let second_resource = crate::ShaderResourceDescriptor::single(
			second_slot,
			crate::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let set = device.create_descriptor_set(None);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Storage)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		device.write(&[crate::DescriptorWrite::image(set, first_slot, image, crate::Layouts::General)]);
		let first_pipeline = create_metadata_compute_pipeline(&mut device, &[], [first_resource]);
		let second_pipeline = create_metadata_compute_pipeline(&mut device, &[], [second_resource]);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			let mut recording = device.create_command_buffer_recording(command_buffer);
			recording.bind_compute_pipeline(first_pipeline).bind_descriptor_sets(&[set]);
			recording
				.bind_compute_pipeline(second_pipeline)
				.dispatch(crate::DispatchExtent::new(
					::utils::Extent::rectangle(1, 1),
					::utils::Extent::rectangle(1, 1),
				));
		}));

		assert!(result.is_err());
	}

	#[test]
	fn shared_descriptor_sets_ignore_resources_outside_the_active_pipeline() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let active_slot = crate::ResourceSlot::new(0);
		let inactive_slot = crate::ResourceSlot::new(5);
		let active_resource = crate::ShaderResourceDescriptor::single(
			active_slot,
			crate::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let set = device.create_descriptor_set(None);
		let active_image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Storage)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		let inactive_image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Storage)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		device.write(&[
			crate::DescriptorWrite::image(set, active_slot, active_image, crate::Layouts::General),
			crate::DescriptorWrite::image(set, inactive_slot, inactive_image, crate::Layouts::General),
		]);
		let pipeline = create_metadata_compute_pipeline(&mut device, &[], [active_resource]);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		{
			let mut recording = device.create_command_buffer_recording(command_buffer);
			recording.bind_compute_pipeline(pipeline).bind_descriptor_sets(&[set]);
		}

		assert_eq!(device.image_uav_descriptor_write_count(), 1);
	}

	/// Verifies that frame rotation reuses stable native descriptor heaps instead of consuming transient arena slots.
	#[test]
	fn descriptor_materializations_are_reused_across_frame_sequences() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let resource =
			crate::ShaderResourceDescriptor::single(slot, crate::ResourceKind::UniformBuffer, crate::AccessPolicies::READ);
		let set = device.create_descriptor_set(None);
		let camera = device.build_dynamic_buffer::<[f32; 16]>(
			crate::buffer::Builder::new(crate::Uses::Uniform).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		device.write(&[crate::DescriptorWrite::buffer(set, slot, camera.into())]);
		let pipeline = create_metadata_compute_pipeline(&mut device, &[], [resource]);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		for frame_index in 0..512 {
			let sequence_index = (frame_index % device.frames as usize) as u8;
			device.begin_command_buffer(command_buffer, sequence_index);
			device.bind_pipeline_native_state(command_buffer, pipeline);
			device.validate_descriptor_sets(pipeline, &[set], sequence_index);
			device.bind_descriptor_heaps_and_tables(command_buffer, Some(pipeline), &[set], sequence_index);
		}

		assert_eq!(device.descriptor_write_count(), device.frames as usize);
		assert_eq!(device.descriptor_materialization_count(), device.frames as usize);
		assert!(
			device
				.descriptor_table_bind_records()
				.iter()
				.all(|record| record.heap_slot == 0),
			"Retained DX12 descriptor tables moved within a transient heap. The most likely cause is that bind-time materialization still consumes an accumulating arena offset.",
		);
	}

	/// Verifies that retained heaps refresh once after logical writes or descriptor-visible resource replacement.
	#[test]
	fn descriptor_materializations_refresh_after_retained_changes() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let resource =
			crate::ShaderResourceDescriptor::single(slot, crate::ResourceKind::UniformBuffer, crate::AccessPolicies::READ);
		let set = device.create_descriptor_set(None);
		let first_buffer = device.build_dynamic_buffer::<[f32; 16]>(
			crate::buffer::Builder::new(crate::Uses::Uniform).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		device.write(&[crate::DescriptorWrite::buffer(set, slot, first_buffer.into())]);
		let pipeline = create_metadata_compute_pipeline(&mut device, &[], [resource]);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let bind = |device: &mut crate::dx12::Device| {
			device.begin_command_buffer(command_buffer, 0);
			device.bind_pipeline_native_state(command_buffer, pipeline);
			device.validate_descriptor_sets(pipeline, &[set], 0);
			device.bind_descriptor_heaps_and_tables(command_buffer, Some(pipeline), &[set], 0);
		};

		bind(&mut device);

		assert_eq!(device.descriptor_write_count(), 1);
		assert_eq!(device.descriptor_materialization_count(), 1);

		device.write(&[crate::DescriptorWrite::buffer(set, slot, first_buffer.into())]);
		bind(&mut device);

		assert_eq!(device.descriptor_write_count(), 1);

		let replacement = device.build_dynamic_buffer::<[f32; 16]>(
			crate::buffer::Builder::new(crate::Uses::Uniform).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		device.write(&[crate::DescriptorWrite::buffer(set, slot, replacement.into())]);
		bind(&mut device);

		assert_eq!(device.descriptor_write_count(), 2);
		bind(&mut device);

		assert_eq!(device.descriptor_write_count(), 2);

		device.resize_buffer(replacement, std::mem::size_of::<[f32; 32]>());

		assert_eq!(device.descriptor_materialization_count(), 0);
		bind(&mut device);

		assert_eq!(device.descriptor_write_count(), 3);
	}

	#[test]
	fn hlsl_structured_buffer_stride_inference_matches_shader_struct_layout() {
		let source = r#"
struct View {
	float4x4 view;
	float4x4 projection;
	float4x4 view_projection;
	float4x4 inverse_view;
	float4x4 inverse_projection;
	float4x4 inverse_view_projection;
	float2 fov;
	float near;
	float far;
};
struct SkinInfluences {
	uint16_t4 joints;
};
StructuredBuffer<View> views : register(t0, space0);
StructuredBuffer<uint> indices : register(t6, space0);
RWStructuredBuffer<uint4> dispatches : register(u3, space1);
StructuredBuffer<SkinInfluences> skin_influences : register(t4, space2);
StructuredBuffer<uint16_t2> packed_pairs : register(t5, space2);
"#;

		let strides = Device::hlsl_structured_buffer_strides(source);

		assert_eq!(strides.get(&(0, 0)), Some(&400));
		assert_eq!(strides.get(&(0, 6)), Some(&4));
		assert_eq!(strides.get(&(1, 3)), Some(&16));
		assert_eq!(strides.get(&(2, 4)), Some(&8));
		assert_eq!(strides.get(&(2, 5)), Some(&4));
	}

	#[test]
	fn dxil_cache_path_changes_with_the_loaded_dxc_commit() {
		let first = Device::hlsl_dxil_cache_path("dxc-1.9-5402-first", "source", "main", "cs_6_9", &[])
			.expect("Expected a DXIL cache path for the first compiler identity");
		let second = Device::hlsl_dxil_cache_path("dxc-1.9-5403-second", "source", "main", "cs_6_9", &[])
			.expect("Expected a DXIL cache path for the second compiler identity");

		assert_ne!(first.file_name(), second.file_name());
	}

	#[test]
	fn agility_sdk_path_uses_the_deployed_default_and_validates_overrides() {
		assert_eq!(Device::agility_sdk_path(None).unwrap().to_bytes(), b".\\D3D12\\");
		assert_eq!(
			Device::agility_sdk_path(Some("C:\\byte-engine\\agility")).unwrap().to_bytes(),
			b"C:\\byte-engine\\agility\\"
		);
		assert_eq!(
			Device::agility_sdk_path(Some("C:\\byte-engine\\agility\\"))
				.unwrap()
				.to_bytes(),
			b"C:\\byte-engine\\agility\\"
		);
		assert!(Device::agility_sdk_path(Some("")).is_err());
		assert!(Device::agility_sdk_path(Some("D3D12\0payload")).is_err());
	}

	#[test]
	fn hlsl_shader_creation_preserves_explicit_buffer_stride() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let resource =
			crate::ShaderResourceDescriptor::single(slot, crate::ResourceKind::StorageBuffer, crate::AccessPolicies::READ)
				.buffer_stride(400);
		let shader_source = r#"
StructuredBuffer<uint> views : register(t0, space0);
[numthreads(1, 1, 1)]
void main() {}
"#;
		let Ok(shader) = device.create_shader(
			Some("explicit structured stride"),
			crate::shader::Sources::HLSL {
				source: shader_source,
				entry_point: "main",
			},
			crate::ShaderTypes::Compute,
			[resource],
		) else {
			return;
		};
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			crate::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		assert_eq!(
			device
				.pipeline_resource_descriptor(pipeline, slot)
				.map(crate::ShaderResourceDescriptor::buffer_element_stride),
			Some(400),
		);
	}

	#[test]
	fn hlsl_shader_creation_infers_later_flat_buffer_stride() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let resources = [
			crate::ShaderResourceDescriptor::single(
				crate::ResourceSlot::new(0),
				crate::ResourceKind::CombinedImageSampler,
				crate::AccessPolicies::READ,
			),
			crate::ShaderResourceDescriptor::single(
				crate::ResourceSlot::new(1),
				crate::ResourceKind::StorageImage,
				crate::AccessPolicies::WRITE,
			),
			crate::ShaderResourceDescriptor::single(
				crate::ResourceSlot::new(2),
				crate::ResourceKind::StorageBuffer,
				crate::AccessPolicies::READ,
			),
		];
		let shader_source = r#"
struct _parameters {
	float4x4 inverse_view_projection;
	float4 camera_position;
	float4 sun_direction;
	float4 planet_center;
	float4 atmosphere;
	float4 misc;
};
StructuredBuffer<_parameters> parameters : register(t2, space0);
RWTexture2D<float4> main_texture : register(u1, space0);
Texture2D<float4> depth_texture : register(t0, space0);
SamplerState depth_texture_sampler : register(s0, space0);
[numthreads(1, 1, 1)]
void main() {
	main_texture[uint2(0, 0)] = parameters[0].camera_position;
}
"#;
		let Ok(shader) = device.create_shader(
			Some("sky structured stride inference"),
			crate::shader::Sources::HLSL {
				source: shader_source,
				entry_point: "main",
			},
			crate::ShaderTypes::Compute,
			resources,
		) else {
			return;
		};
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			crate::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		assert_eq!(
			device
				.pipeline_resource_descriptor(pipeline, crate::ResourceSlot::new(2))
				.map(crate::ShaderResourceDescriptor::buffer_element_stride),
			Some(144),
		);
	}

	#[test]
	fn pipelines_create_native_root_signatures() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let resource =
			crate::ShaderResourceDescriptor::single(slot, crate::ResourceKind::StorageImage, crate::AccessPolicies::WRITE);
		let set = device.create_descriptor_set(None);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Storage)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		device.write(&[crate::DescriptorWrite::image(set, slot, image, crate::Layouts::General)]);
		let pipeline =
			create_metadata_compute_pipeline(&mut device, &[crate::pipelines::PushConstantRange::new(0, 16)], [resource]);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording
			.bind_compute_pipeline(pipeline)
			.bind_descriptor_sets(&[set])
			.write_push_constant(4, 7u32);
		drop(recording);

		assert_eq!(device.root_signature_bind_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(false));
		assert_eq!(device.pipeline_state_bind_count(), 0);
		assert_eq!(device.compute_dispatch_encode_count(), 0);
		assert_eq!(device.descriptor_heap_bind_count(), 1);
		assert_eq!(device.descriptor_table_bind_count(), 1);
		assert_eq!(device.push_constant_write_count(), 1);
		assert_eq!(
			device.push_constant_write_records(),
			&[crate::dx12::device::PushConstantWriteRecord {
				root_parameter_index: 1,
				offset: 4,
				size: 4,
				compute_root: true,
			}]
		);
	}

	#[test]
	fn root_signatures_reject_more_than_sixty_four_dwords() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Compute, [])
			.expect("Failed to create DX12 shader metadata.");

		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
				&[crate::pipelines::PushConstantRange::new(0, 260)],
				crate::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
			));
		}));

		assert!(result.is_err());
	}

	#[test]
	fn hlsl_compute_pipeline_specializes_scalar_macro_types() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let shader = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "
						#ifndef SPEC_CONSTANT_0
						#define SPEC_CONSTANT_0 false
						#endif
						#ifndef SPEC_CONSTANT_1
						#define SPEC_CONSTANT_1 1u
						#endif
						#ifndef SPEC_CONSTANT_2
						#define SPEC_CONSTANT_2 -1
						#endif
						[numthreads(1, 1, 1)]
						void main(uint3 id : SV_DispatchThreadID) {
							bool enabled = SPEC_CONSTANT_0;
							uint count = SPEC_CONSTANT_1;
							int offset = SPEC_CONSTANT_2;
						}
					",
					entry_point: "main",
				},
				crate::ShaderTypes::Compute,
				[],
			)
			.expect("Failed to compile default DX12 HLSL compute shader.");
		let specialization = [
			crate::pipelines::SpecializationMapEntry::new(0, true),
			crate::pipelines::SpecializationMapEntry::new(1, 8u32),
			crate::pipelines::SpecializationMapEntry::new(2, -3i32),
		];
		let shader_parameter = crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute)
			.with_specialization_map(&specialization);
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(&[], shader_parameter));

		assert_eq!(device.compute_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.hlsl_specialization_compile_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(true));
	}

	#[test]
	fn factory_compute_pipeline_preserves_hlsl_specialization_map() {
		use crate::Device as _;

		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let mut factory = device.create_factory().expect("DX12 should expose a resource factory.");
		let shader = factory
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "
						#ifndef SPEC_CONSTANT_0
						#define SPEC_CONSTANT_0 1.0
						#endif
						[numthreads(1, 1, 1)]
						void main(uint3 id : SV_DispatchThreadID) {
							float value = SPEC_CONSTANT_0;
						}
					",
					entry_point: "main",
				},
				crate::ShaderTypes::Compute,
				[],
			)
			.expect("Failed to create detached DX12 HLSL shader.");
		let specialization = [crate::pipelines::SpecializationMapEntry::new(0, 8.0f32)];
		let detached_compute = factory.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute)
				.with_specialization_map(&specialization),
		));
		let synchronizer = device.create_synchronizer(None, false);

		let mut frame = device.start_frame(0, synchronizer);
		let pipeline = frame.intern_compute_pipeline(detached_compute);
		drop(frame);

		assert_eq!(device.compute_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.hlsl_specialization_compile_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(true));
	}

	#[test]
	fn hlsl_raster_shaders_compile_to_native_pipeline_state() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let vertex = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "
						float4 main(uint vertex_id : SV_VertexID) : SV_Position {
							float2 positions[3] = {
								float2(0.0, 0.5),
								float2(0.5, -0.5),
								float2(-0.5, -0.5)
							};
							return float4(positions[vertex_id], 0.0, 1.0);
						}
					",
					entry_point: "main",
				},
				crate::ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to compile DX12 HLSL vertex shader.");
		let fragment = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "
						float4 main() : SV_Target {
							return float4(1.0, 0.0, 0.0, 1.0);
						}
					",
					entry_point: "main",
				},
				crate::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to compile DX12 HLSL fragment shader.");
		let shaders = [
			crate::pipelines::ShaderParameter::new(&vertex, crate::ShaderTypes::Vertex),
			crate::pipelines::ShaderParameter::new(&fragment, crate::ShaderTypes::Fragment),
		];
		let render_targets = [crate::pipelines::raster::AttachmentDescriptor::new(
			crate::Formats::RGBA8UNORM,
		)];
		let pipeline = device.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[crate::pipelines::PushConstantRange::new(0, 4)],
			&[],
			&shaders,
			&render_targets,
		));
		let command_buffer = device.create_command_buffer(Some("graphics root constants"), queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording.bind_raster_pipeline(pipeline).write_push_constant(0, 9u32);
		drop(recording);

		assert_eq!(device.graphics_pipeline_state_create_attempt_count(), 1);
		assert_eq!(
			device.pipeline_has_native_state(pipeline),
			Some(true),
			"last graphics PSO error: {:?}",
			device.graphics_pipeline_state_last_error()
		);
		assert_eq!(
			device.push_constant_write_records(),
			&[crate::dx12::device::PushConstantWriteRecord {
				root_parameter_index: 0,
				offset: 0,
				size: 4,
				compute_root: false,
			}]
		);
	}

	#[test]
	fn swapchain_rejects_a_second_acquisition_before_present() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let extent = ::utils::Extent::rectangle(4, 4);
		let mut app =
			crate::window::App::new("DX12 Outstanding Acquisition Test").expect("Failed to create the DX12 test app.");
		let window = app
			.create_window("DX12 Outstanding Acquisition Test", extent, crate::window::Features::empty())
			.expect("Failed to create DX12 test window.");
		let swapchain = device.bind_to_window(&window.os_handles(), Default::default(), extent, crate::Uses::RenderTarget);
		let synchronizer = device.create_synchronizer(None, true);

		let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			let mut frame = device.start_frame(0, synchronizer);
			let _ = frame.acquire_swapchain_image(swapchain);
			let _ = frame.acquire_swapchain_image(swapchain);
		}));

		assert!(
			rejected.is_err(),
			"A DXGI swapchain must not expose a second outstanding present key for the same native backbuffer chain."
		);
	}

	#[test]
	fn present_storage_swapchain_copies_proxy_to_backbuffer() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let extent = ::utils::Extent::rectangle(4, 4);
		let mut app = crate::window::App::new("DX12 Storage Present Proxy Test").expect("Failed to create the DX12 test app.");
		let window = app
			.create_window("DX12 Storage Present Proxy Test", extent, crate::window::Features::empty())
			.expect("Failed to create DX12 test window.");
		let swapchain = device.bind_to_window(&window.os_handles(), Default::default(), extent, crate::Uses::Storage);
		let slot = crate::ResourceSlot::new(0);
		let resource =
			crate::ShaderResourceDescriptor::single(slot, crate::ResourceKind::StorageImage, crate::AccessPolicies::WRITE);
		let set = device.create_descriptor_set(None);
		device.write(&[crate::DescriptorWrite::swapchain(set, slot, swapchain)]);
		let shader = device
			.create_shader(
				Some("storage swapchain present"),
				crate::shader::Sources::HLSL {
					source: "
						RWTexture2D<float4> output_texture : register(u0, space0);
						[numthreads(1, 1, 1)]
						void main(uint3 dispatch_thread_id : SV_DispatchThreadID) {
							output_texture[dispatch_thread_id.xy] = float4(1.0, 0.25, 0.5, 1.0);
						}
					",
					entry_point: "main",
				},
				crate::ShaderTypes::Compute,
				[resource],
			)
			.expect("Failed to compile DX12 storage swapchain present shader.");
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			crate::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		assert_eq!(device.pipeline_has_native_state(pipeline), Some(true));
		let command_buffer = device.create_command_buffer(Some("storage swapchain present"), queue_handle);
		let synchronizer = device.create_synchronizer(None, true);
		let mut captured_present_key = None;

		device.queue(queue_handle).execute(
			Some(crate::queue::FrameRequest::new(0, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				let present_key = execution
					.frame()
					.unwrap()
					.acquire_swapchain_image(swapchain)
					.expect("acquire backbuffer")
					.present_key();
				captured_present_key = Some(present_key);
				let present_keys = [present_key];
				execution.record_with_present_keys(command_buffer, &present_keys, |command_buffer_recording| {
					command_buffer_recording
						.bind_compute_pipeline(pipeline)
						.bind_descriptor_sets(&[set])
						.dispatch(crate::DispatchExtent::new(extent, ::utils::Extent::square(1)));
				});
				present_keys
			},
		);

		device.wait_for_synchronizer(synchronizer);
		captured_present_key.expect("Missing acquired present key.");

		assert_eq!(device.texture_copy_count(), 1);
	}

	#[test]
	fn indirect_dispatch_uses_and_flushes_the_active_frame_buffer() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let pipeline = create_metadata_compute_pipeline(&mut device, &[], []);
		let indirect_buffer = device.build_dynamic_buffer::<[[u32; 3]; 2]>(
			crate::buffer::Builder::new(crate::Uses::Indirect).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let sequence_data = [[7, 8, 9], [10, 11, 12]];
		*device.dynamic_buffer_slice_mut(indirect_buffer, 1) = sequence_data;

		let command_buffer = device.create_command_buffer(None, queue_handle);
		device.begin_command_buffer(command_buffer, 1);
		let mut recording = super::command_buffer::CommandBufferRecording::new(
			&mut device,
			command_buffer,
			Some(crate::FrameKey {
				frame_index: 1,
				sequence_index: 1,
			}),
		);
		crate::command_buffer::CommonCommandBufferMode::bind_compute_pipeline(&mut recording, pipeline)
			.indirect_dispatch(indirect_buffer, 0);
		drop(recording);

		assert_eq!(
			device.buffer_mapped_bytes_for_sequence(indirect_buffer.into(), std::mem::size_of_val(&sequence_data), 1),
			Some(bytemuck::bytes_of(&sequence_data).to_vec()),
		);
		assert_eq!(
			device.buffer_mapped_bytes_for_sequence(indirect_buffer.into(), std::mem::size_of_val(&sequence_data), 0),
			Some(vec![0; std::mem::size_of_val(&sequence_data)]),
		);
	}

	/// Verifies that visibility-like render passes reuse retained attachment views across long frame runs.
	#[test]
	fn render_pass_attachment_view_allocations_remain_bounded() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let color = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::RenderTarget)
				.extent(::utils::Extent::rectangle(1, 1))
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let depth = device.build_image(
			crate::image::Builder::new(crate::Formats::Depth32, crate::Uses::DepthStencil)
				.extent(::utils::Extent::rectangle(1, 1))
				.array_layers(std::num::NonZeroU32::new(4))
				.device_accesses(crate::DeviceAccesses::DeviceOnly)
				.optimized_clear_value(crate::ClearValue::Depth(1.0)),
		);
		let command_buffer = device.create_command_buffer(Some("retained attachment views"), queue_handle);

		assert_eq!(device.render_target_view_count(), 1);
		assert_eq!(device.depth_stencil_view_count(), 1);
		assert_eq!(device.render_target_view_allocation_count(), 1);
		assert_eq!(device.depth_stencil_view_allocation_count(), 1);
		assert_eq!(device.depth_stencil_descriptor_count(), 8);

		for frame_index in 0..1024 {
			let cascade = frame_index % 4;
			let attachments = [
				crate::AttachmentInformation::new(
					color.0,
					crate::Layouts::RenderTarget,
					crate::LoadOp::Load,
					crate::StoreOp::Store,
				),
				crate::AttachmentInformation::new(
					depth.0,
					crate::Layouts::RenderTarget,
					crate::LoadOp::Load,
					crate::StoreOp::Store,
				)
				.layer(cascade),
			];
			device.begin_command_buffer(command_buffer, 0);
			device.bind_render_targets_native(command_buffer, &attachments, 0);
		}

		let layered_attachment = [crate::AttachmentInformation::new(
			depth.0,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Load,
			crate::StoreOp::Store,
		)
		.layers(4)];
		device.begin_command_buffer(command_buffer, 0);
		device.bind_render_targets_native(command_buffer, &layered_attachment, 0);

		assert_eq!(device.render_target_view_count(), 1);
		assert_eq!(device.depth_stencil_view_count(), 1);
		assert_eq!(device.render_target_view_allocation_count(), 1);
		assert_eq!(device.depth_stencil_view_allocation_count(), 1);
		assert_eq!(device.depth_stencil_descriptor_count(), 8);
		assert_eq!(Device::depth_stencil_view_array_range(4, None, 4), Some((0, 4)));
		assert_eq!(Device::depth_stencil_view_array_range(4, None, 3), Some((0, 3)));
		assert_eq!(Device::depth_stencil_view_array_range(4, Some(2), 1), Some((2, 1)));
	}

	#[test]
	fn descriptor_tables_stage_multiple_sets_into_one_native_heap() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let base_slot = crate::ResourceSlot::new(1);
		let visibility_buffer_slot = crate::ResourceSlot::new(0);
		let visibility_image_slot = crate::ResourceSlot::new(7);
		let resources = [
			crate::ShaderResourceDescriptor::single(base_slot, crate::ResourceKind::StorageBuffer, crate::AccessPolicies::READ),
			crate::ShaderResourceDescriptor::single(
				visibility_buffer_slot,
				crate::ResourceKind::StorageBuffer,
				crate::AccessPolicies::READ,
			),
			crate::ShaderResourceDescriptor::single(
				visibility_image_slot,
				crate::ResourceKind::StorageImage,
				crate::AccessPolicies::WRITE,
			),
		];
		let base_set = device.create_descriptor_set(None);
		let visibility_set = device.create_descriptor_set(None);
		let base_buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let visibility_buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let visibility_image = device.build_image(
			crate::image::Builder::new(crate::Formats::U32, crate::Uses::Storage).extent(::utils::Extent::rectangle(1, 1)),
		);
		device.write(&[
			crate::DescriptorWrite::buffer(base_set, base_slot, base_buffer.into()),
			crate::DescriptorWrite::buffer(visibility_set, visibility_buffer_slot, visibility_buffer.into()),
			crate::DescriptorWrite::image(
				visibility_set,
				visibility_image_slot,
				visibility_image,
				crate::Layouts::General,
			),
		]);

		let pipeline = create_metadata_compute_pipeline(&mut device, &[], resources);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording
			.bind_compute_pipeline(pipeline)
			.bind_descriptor_sets(&[base_set, visibility_set]);
		drop(recording);

		assert_eq!(
			device.pipeline_descriptor_slot(pipeline, visibility_buffer_slot, 0, false),
			Some(0)
		);
		assert_eq!(device.pipeline_descriptor_slot(pipeline, base_slot, 0, false), Some(1));
		assert_eq!(
			device.pipeline_descriptor_slot(pipeline, visibility_image_slot, 0, false),
			Some(2)
		);
		assert_eq!(device.descriptor_heap_bind_count(), 1);
		assert_eq!(
			device.descriptor_table_bind_records(),
			&[crate::dx12::context::DescriptorTableBindRecord {
				root_parameter_index: 0,
				set_index: 0,
				binding_index: 0,
				sampler_heap: false,
				heap_slot: 0,
			}]
		);
	}

	#[test]
	fn transfer_queue_consumes_compute_output_with_enhanced_barriers() {
		let features = crate::device::Features::new().validation(true).mesh_shading(false);
		let Some((_instance, mut device, compute_queue_handle, transfer_queue_handle)) =
			create_compute_transfer_device_setup(features)
		else {
			return;
		};
		let source_slot = crate::ResourceSlot::new(0);
		let output_slot = crate::ResourceSlot::new(1);
		let resources = [
			crate::ShaderResourceDescriptor::single(
				source_slot,
				crate::ResourceKind::CombinedImageSampler,
				crate::AccessPolicies::READ,
			),
			crate::ShaderResourceDescriptor::single(
				output_slot,
				crate::ResourceKind::StorageImage,
				crate::AccessPolicies::WRITE,
			),
		];
		let shader = device
			.create_shader(
				Some("compute queue sampled image"),
				crate::shader::Sources::HLSL {
					source: r#"
Texture2D<float4> source_image : register(t0, space0);
SamplerState source_sampler : register(s0, space0);
RWTexture2D<float4> output_image : register(u1, space0);
[numthreads(1, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
	output_image[id.xy] = source_image.SampleLevel(source_sampler, float2(0.5, 0.5), 0.0);
}
"#,
					entry_point: "main",
				},
				crate::ShaderTypes::Compute,
				resources,
			)
			.expect("Failed to compile the DX12 compute-queue sampled-image shader.");
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			crate::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));
		let source = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		let output = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Storage | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		let sampler = device.build_sampler(crate::sampler::Builder::new());
		let set = device.create_descriptor_set(Some("compute queue sampled image"));
		device.write(&[
			crate::DescriptorWrite::combined_image_sampler(set, source_slot, source, sampler, crate::Layouts::Read),
			crate::DescriptorWrite::image(set, output_slot, output, crate::Layouts::General),
		]);
		let synchronizer = device.create_synchronizer(Some("compute queue sampled image"), false);
		let command_buffer = device.create_command_buffer(Some("compute queue sampled image"), compute_queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording
			.bind_compute_pipeline(pipeline)
			.bind_descriptor_sets(&[set])
			.dispatch(crate::DispatchExtent::new(
				::utils::Extent::rectangle(1, 1),
				::utils::Extent::rectangle(1, 1),
			));
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		let destination = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1)),
		);
		let transfer_synchronizer = device.create_synchronizer(Some("compute output transfer"), false);
		let transfer_command_buffer = device.create_command_buffer(Some("compute output transfer"), transfer_queue_handle);
		let mut transfer_recording = device.create_command_buffer_recording(transfer_command_buffer);
		crate::command_buffer::CommandBufferRecording::blit_image(
			&mut transfer_recording,
			output.into(),
			crate::Layouts::Transfer,
			destination.into(),
			crate::Layouts::Transfer,
		);
		crate::command_buffer::CommandBufferRecording::execute(transfer_recording, transfer_synchronizer);
		device.wait_for_synchronizer(transfer_synchronizer);

		assert!(!device.has_errors());
	}

	#[test]
	fn storage_image_descriptor_binding_transitions_render_target_to_uav() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(7);
		let resource =
			crate::ShaderResourceDescriptor::single(slot, crate::ResourceKind::StorageImage, crate::AccessPolicies::WRITE);
		let set = device.create_descriptor_set(None);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::U32, crate::Uses::RenderTarget | crate::Uses::Storage)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		device.write(&[crate::DescriptorWrite::image(set, slot, image, crate::Layouts::General)]);
		let pipeline = create_metadata_compute_pipeline(&mut device, &[], [resource]);

		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		let attachment = crate::AttachmentInformation::new(
			image,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Clear(crate::ClearValue::Integer(u32::MAX, 0, 0, 0)),
			crate::StoreOp::Store,
		);
		crate::command_buffer::CommandBufferRecording::start_render_pass(
			&mut recording,
			::utils::Extent::rectangle(1, 1),
			&[attachment],
		)
		.end_render_pass();
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(
			device.tracked_image_resource_state(image),
			Some(TextureBarrierState::RENDER_TARGET)
		);

		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording.bind_compute_pipeline(pipeline).bind_descriptor_sets(&[set]);
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(
			device.tracked_image_resource_state(image),
			Some(TextureBarrierState::unordered_access(D3D12_BARRIER_SYNC_COMPUTE_SHADING))
		);
		assert!(!device.has_errors());
	}

	#[test]
	fn uav_buffer_rebind_without_state_change_emits_uav_barrier() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let resource =
			crate::ShaderResourceDescriptor::single(slot, crate::ResourceKind::StorageBuffer, crate::AccessPolicies::WRITE);
		let set = device.create_descriptor_set(None);
		let buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		device.write(&[crate::DescriptorWrite::buffer(set, slot, buffer.into())]);
		let pipeline = create_metadata_compute_pipeline(&mut device, &[], [resource]);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording.bind_compute_pipeline(pipeline).bind_descriptor_sets(&[set]);
		recording.bind_descriptor_sets(&[set]);
		drop(recording);

		assert_eq!(device.uav_barrier_count(), 1);
	}

	#[test]
	fn render_pass_clears_u32_render_targets_with_integer_values() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::U32,
				crate::Uses::RenderTarget | crate::Uses::Storage | crate::Uses::TransferSource,
			)
			.extent(::utils::Extent::rectangle(1, 1))
			.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		let attachment = crate::AttachmentInformation::new(
			image,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Clear(crate::ClearValue::Integer(u32::MAX, 0, 0, 0)),
			crate::StoreOp::Store,
		);
		crate::command_buffer::CommandBufferRecording::start_render_pass(
			&mut recording,
			::utils::Extent::rectangle(1, 1),
			&[attachment],
		)
		.end_render_pass();
		let copies = [
			crate::command_buffer::CommandBufferRecording::transfer_texture(&mut recording, image.into()).expect(
				"Texture transfer failed. The most likely cause is that the DX12 test image is not a valid transfer source.",
			),
		];
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(
			device
				.get_image_data(copies[0])
				.expect(
					"Texture mapping failed. The most likely cause is that the DX12 test handle was not created by this device."
				)
				.bytes,
			&[0xff, 0xff, 0xff, 0xff]
		);
		assert_eq!(device.render_target_clear_count(), 1);
		assert!(!device.has_errors());
	}

	#[test]
	fn copy_buffers_updates_shadow_storage() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let source = device.build_buffer::<[u8; 8]>(
			crate::buffer::Builder::new(crate::Uses::TransferSource).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let destination = device.build_buffer::<[u8; 8]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::HostToDevice),
		);

		*device.get_mut_buffer_slice(source) = [1, 2, 3, 4, 5, 6, 7, 8];
		device.sync_buffer(source);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::copy_buffers(
			&mut recording,
			&[crate::BufferCopyDescriptor::new(source.into(), 2, destination.into(), 1, 4)],
		);
		drop(recording);

		assert_eq!(*device.get_buffer_slice(destination), [0, 3, 4, 5, 6, 0, 0, 0]);
	}

	#[test]
	fn command_recording_sync_buffer_flushes_static_resource_for_nonzero_sequence() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		device.set_frames_in_flight(2);
		let synchronizer = device.create_synchronizer(None, false);
		let buffer = device.build_buffer::<[u8; 8]>(
			crate::buffer::Builder::new(crate::Uses::TransferSource).device_accesses(crate::DeviceAccesses::HostOnly),
		);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		{
			let mut frame = device.start_frame(1, synchronizer);
			let mut recording = frame.create_command_buffer_recording_without_implicit_sync(command_buffer);
			*recording.get_mut_buffer_slice(buffer) = [2, 4, 6, 8, 10, 12, 14, 16];
			crate::command_buffer::CommandBufferRecording::sync_buffer(&mut recording, buffer);
			drop(recording);
		}

		assert_eq!(
			device.buffer_mapped_bytes_for_sequence(buffer.into(), 8, 1).unwrap(),
			vec![2, 4, 6, 8, 10, 12, 14, 16]
		);
	}

	#[test]
	fn copy_to_static_host_visible_buffer_flushes_destination_for_nonzero_sequence() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		device.set_frames_in_flight(2);
		let synchronizer = device.create_synchronizer(None, false);
		let source = device.build_buffer::<[u8; 8]>(
			crate::buffer::Builder::new(crate::Uses::TransferSource).device_accesses(crate::DeviceAccesses::HostOnly),
		);
		let destination = device.build_buffer::<[u8; 8]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		{
			let mut frame = device.start_frame(1, synchronizer);
			let mut recording = frame.create_command_buffer_recording_without_implicit_sync(command_buffer);
			*recording.get_mut_buffer_slice(source) = [21, 22, 23, 24, 25, 26, 27, 28];
			crate::command_buffer::CommandBufferRecording::copy_buffers(
				&mut recording,
				&[crate::BufferCopyDescriptor::new(source.into(), 1, destination.into(), 2, 5)],
			);
			drop(recording);
		}

		assert_eq!(
			device.buffer_mapped_bytes_for_sequence(destination.into(), 8, 1).unwrap(),
			vec![0, 0, 22, 23, 24, 25, 26, 0]
		);
	}

	/// Verifies that command-buffer reuse releases completed upload staging resources before recording again.
	#[test]
	fn repeated_image_uploads_keep_live_staging_resources_bounded() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1))
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let command_buffer = device.create_command_buffer(Some("bounded image uploads"), queue_handle);
		let synchronizer = device.create_synchronizer(Some("bounded image uploads"), false);

		for value in 0..512 {
			let mut recording = device.create_command_buffer_recording(command_buffer);
			crate::command_buffer::CommandBufferRecording::clear_images(
				&mut recording,
				&[(image.into(), crate::ClearValue::Integer(value, 0, 0, 0))],
			);
			crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);

			assert_eq!(device.upload_resource_count(), 1);
		}
		let recording = device.create_command_buffer_recording(command_buffer);
		drop(recording);

		assert_eq!(device.upload_resource_count(), 0);
	}

	#[test]
	fn transfer_texture_resolves_submitted_readback_copy() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferSource | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1))
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let pixel = crate::RGBAu8 {
			r: 21,
			g: 22,
			b: 23,
			a: 24,
		};
		let data = [pixel];

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::write_image_data(&mut recording, image.into(), &data);
		let copies = [
			crate::command_buffer::CommandBufferRecording::transfer_texture(&mut recording, image.into()).expect(
				"Texture transfer failed. The most likely cause is that the DX12 test image is not a valid transfer source.",
			),
		];
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(
			device
				.get_image_data(copies[0])
				.expect(
					"Texture mapping failed. The most likely cause is that the DX12 test handle was not created by this device."
				)
				.bytes,
			bytemuck::bytes_of(&pixel)
		);
		assert_eq!(
			device.get_image_data(copies[0]),
			Err(crate::TextureTransferError::InvalidHandle(copies[0]))
		);
		assert_eq!(device.readback_resource_count(), 0);
		assert_eq!(device.texture_readback_resolve_count(), 1);
	}

	#[test]
	fn queue_batch_signals_once_after_nonempty_and_empty_lists_and_allows_immediate_reuse() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		device.set_frames_in_flight(1);
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferSource | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1))
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let pixel = crate::RGBAu8 {
			r: 31,
			g: 32,
			b: 33,
			a: 34,
		};
		let synchronizer = device.create_synchronizer(None, false);
		let nonempty = device.create_command_buffer(Some("nonempty batch list"), queue_handle);
		let empty = device.create_command_buffer(Some("empty batch list"), queue_handle);
		let mut copy = None;

		device.queue(queue_handle).execute(
			Some(crate::queue::FrameRequest::new(0, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(nonempty, |recording| {
					crate::command_buffer::CommandBufferRecording::write_image_data(recording, image.into(), &[pixel]);
					copy = Some(
						crate::command_buffer::CommandBufferRecording::transfer_texture(recording, image.into()).expect(
							"Texture transfer failed. The most likely cause is that the DX12 test image is not a valid transfer source.",
						),
					);
				});
				execution.record(empty, |_| {});
				[]
			},
		);

		assert_eq!(device.synchronizer_value(synchronizer), Some(1));
		assert_eq!(device.native_command_list_execute_count(), 1);
		assert_eq!(device.empty_command_list_skip_count(), 1);

		// Reusing the same sequence and both allocators immediately must wait for the batch's one terminal fence.
		device.queue(queue_handle).execute(
			Some(crate::queue::FrameRequest::new(1, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(nonempty, |_| {});
				execution.record(empty, |_| {});
				[]
			},
		);

		let copy = copy.expect(
			"Missing DX12 readback handle. The most likely cause is that the nonempty command list did not record its transfer.",
		);
		assert_eq!(
			device
				.get_image_data(copy)
				.expect(
					"Texture mapping failed. The most likely cause is that the terminal batch fence did not complete the readback."
				)
				.bytes,
			bytemuck::bytes_of(&pixel),
		);
		assert_eq!(device.synchronizer_value(synchronizer), Some(2));
		assert!(!device.has_errors());
	}

	#[test]
	fn queue_execution_rejects_a_foreign_command_buffer_before_mutation() {
		let features = crate::device::Features::new().validation(false).mesh_shading(false);
		let Some((_instance, mut device, compute_queue, transfer_queue)) = create_compute_transfer_device_setup(features)
		else {
			return;
		};
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(Some("compute-owned list"), compute_queue);

		let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.queue(transfer_queue).execute(
				Some(crate::queue::FrameRequest::new(0, synchronizer)),
				&[],
				synchronizer,
				|execution| {
					execution.record(command_buffer, |_| {});
					[]
				},
			);
		}));

		assert!(rejected.is_err());
		device.queue(compute_queue).execute(
			Some(crate::queue::FrameRequest::new(0, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |_| {});
				[]
			},
		);
		device.wait_for_synchronizer(synchronizer);
		assert_eq!(device.synchronizer_value(synchronizer), Some(1));
		assert!(!device.has_errors());
	}

	#[test]
	fn queue_execution_rejects_duplicate_command_buffers_before_submission_and_remains_reusable() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let synchronizer = device.create_synchronizer(None, false);
		let first_command_buffer = device.create_command_buffer(None, queue_handle);
		let second_command_buffer = device.create_command_buffer(None, queue_handle);
		let duplicate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.queue(queue_handle).execute(
				Some(crate::queue::FrameRequest::new(0, synchronizer)),
				&[],
				synchronizer,
				|execution| {
					execution.record(first_command_buffer, |_| {});
					execution.record(second_command_buffer, |_| {});
					execution.record(first_command_buffer, |_| {});
					[]
				},
			);
		}));

		assert!(duplicate.is_err());
		device.queue(queue_handle).execute(
			Some(crate::queue::FrameRequest::new(0, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(first_command_buffer, |_| {});
				execution.record(second_command_buffer, |_| {});
				[]
			},
		);
		device.wait_for_synchronizer(synchronizer);
		assert!(!device.has_errors());
	}

	#[test]
	fn frame_sequence_waits_its_previous_synchronizer_when_the_master_changes() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let first = device.create_synchronizer(None, false);
		let replacement = device.create_synchronizer(None, false);

		device
			.queue(queue_handle)
			.execute(Some(crate::queue::FrameRequest::new(0, first)), &[], first, |_| []);
		device.queue(queue_handle).execute(
			Some(crate::queue::FrameRequest::new(2, replacement)),
			&[],
			replacement,
			|_| [],
		);
		device.wait_for_synchronizer(replacement);

		assert_eq!(device.synchronizer_value(first), Some(1));
		assert_eq!(device.synchronizer_value(replacement), Some(1));
		assert!(!device.has_errors());
	}

	#[test]
	fn frame_recording_rejects_a_different_completion_synchronizer_before_submission() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		device.set_frames_in_flight(1);
		let frame_synchronizer = device.create_synchronizer(None, false);
		let different_synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			let mut frame = device.start_frame(0, frame_synchronizer);
			let recording = frame.create_command_buffer_recording(command_buffer);
			crate::command_buffer::CommandBufferRecording::execute(recording, different_synchronizer);
		}));

		assert!(rejected.is_err());
		assert_eq!(device.synchronizer_value(frame_synchronizer), Some(0));
		assert_eq!(device.synchronizer_value(different_synchronizer), Some(0));

		let mut frame = device.start_frame(0, frame_synchronizer);
		let recording = frame.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::execute(recording, frame_synchronizer);
		drop(frame);
		device.wait_for_synchronizer(frame_synchronizer);
		assert_eq!(device.synchronizer_value(frame_synchronizer), Some(1));
		assert!(!device.has_errors());
	}

	#[test]
	fn storage_present_requires_recorded_preparation_before_submission() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let extent = ::utils::Extent::rectangle(4, 4);
		let mut app =
			crate::window::App::new("DX12 Missing Present Preparation Test").expect("Failed to create the DX12 test app.");
		let window = app
			.create_window(
				"DX12 Missing Present Preparation Test",
				extent,
				crate::window::Features::empty(),
			)
			.expect(
				"Failed to create the DX12 present-validation test window. The most likely cause is that WSI is unavailable.",
			);
		let swapchain = device.bind_to_window(&window.os_handles(), Default::default(), extent, crate::Uses::Storage);
		device.get_swapchain_image(swapchain, crate::Uses::Storage);
		let synchronizer = device.create_synchronizer(None, false);
		let missing_preparation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.queue(queue_handle).execute(
				Some(crate::queue::FrameRequest::new(0, synchronizer)),
				&[],
				synchronizer,
				|execution| {
					let present_key = execution
						.frame()
						.unwrap()
						.acquire_swapchain_image(swapchain)
						.expect("acquire backbuffer")
						.present_key();
					[present_key]
				},
			);
		}));

		assert!(missing_preparation.is_err());
	}

	#[test]
	fn clear_buffers_updates_shadow_storage() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::HostToDevice),
		);

		*device.get_mut_buffer_slice(buffer) = [1, 2, 3, 4];
		device.sync_buffer(buffer);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_buffers(&mut recording, &[buffer.into()]);
		drop(recording);

		assert_eq!(*device.get_buffer_slice(buffer), [0, 0, 0, 0]);
	}

	#[test]
	fn clear_device_only_buffer_records_native_uav_clear() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		let buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage | crate::Uses::TransferDestination)
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let upload_resource_count = device.upload_resource_count();
		*device.get_mut_buffer_slice(buffer) = [1, 2, 3, 4];
		let synchronizer = device.create_synchronizer(None, false);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_buffers(&mut recording, &[buffer.into()]);
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(*device.get_buffer_slice(buffer), [1, 2, 3, 4]);
		assert_eq!(device.buffer_clear_count(), 1);
		assert_eq!(device.upload_resource_count(), upload_resource_count);
		assert_eq!(device.buffer_is_in_common_state(buffer.into()), Some(false));
		assert!(!device.has_errors());
	}

	#[test]
	fn uav_clears_reuse_retained_cpu_descriptors() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		let first_buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage | crate::Uses::TransferDestination)
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let second_buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage | crate::Uses::TransferDestination)
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let copy_call_count = device.clear_descriptor_copy_call_count();

		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_buffers(
			&mut recording,
			&[first_buffer.into(), second_buffer.into()],
		);
		recording.finish_for_submission();
		drop(recording);

		assert_eq!(device.retained_clear_uav_descriptor_pool_state(), (2, 1, 2, 0));
		assert_eq!(device.pending_clear_descriptor_copy_count(command_buffer), 0);
		assert_eq!(device.clear_descriptor_copy_call_count(), copy_call_count + 1);
		device.submit_command_buffer(command_buffer, synchronizer);

		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_buffers(&mut recording, &[first_buffer.into()]);
		recording.finish_for_submission();
		drop(recording);

		assert_eq!(device.retained_clear_uav_descriptor_pool_state(), (2, 1, 2, 0));
		device.submit_command_buffer(command_buffer, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert!(!device.has_errors());
	}

	#[test]
	fn resized_buffers_recycle_retained_clear_descriptors() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		let buffer = device.build_dynamic_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage | crate::Uses::TransferDestination)
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_buffers(&mut recording, &[buffer.into()]);
		recording.finish_for_submission();
		drop(recording);
		device.submit_command_buffer(command_buffer, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(device.retained_clear_uav_descriptor_pool_state(), (1, 1, 1, 0));

		device.resize_buffer(buffer, std::mem::size_of::<[u32; 8]>());

		assert_eq!(device.retained_clear_uav_descriptor_pool_state(), (0, 1, 1, 1));

		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_buffers(&mut recording, &[buffer.into()]);
		recording.finish_for_submission();
		drop(recording);
		device.submit_command_buffer(command_buffer, synchronizer);
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(device.retained_clear_uav_descriptor_pool_state(), (1, 1, 1, 0));
		assert!(!device.has_errors());
	}

	#[test]
	fn reusing_command_buffer_waits_for_previous_submission_before_allocator_reset() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		let buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage | crate::Uses::TransferDestination)
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		for _ in 0..2 {
			let mut recording = device.create_command_buffer_recording(command_buffer);
			crate::command_buffer::CommandBufferRecording::clear_buffers(&mut recording, &[buffer.into()]);
			recording.finish_for_submission();
			drop(recording);
			device.submit_command_buffer(command_buffer, synchronizer);
		}
		device.wait_for_synchronizer(synchronizer);

		assert_eq!(device.buffer_clear_count(), 2);
		assert!(!device.has_errors());
	}

	#[test]
	fn reusing_logical_command_buffer_on_the_next_sequence_does_not_wait_for_the_previous_sequence() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		device.set_frames_in_flight(2);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let blocker = device
			.block_queue_until_test_fence(queue_handle)
			.expect("Failed to block the DX12 test queue. The most likely cause is that the test device was removed.");

		device.queue(queue_handle).execute(
			Some(crate::queue::FrameRequest::new(0, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |_| {});
				[]
			},
		);

		// A broken single-allocator implementation waits on the blocked first sequence.
		let elapsed = time_with_blocked_queue(&blocker, || {
			device.queue(queue_handle).execute(
				Some(crate::queue::FrameRequest::new(1, synchronizer)),
				&[],
				synchronizer,
				|execution| {
					execution.record(command_buffer, |_| {});
					[]
				},
			);
		});
		device.wait_for_synchronizer(synchronizer);

		assert!(
			elapsed < std::time::Duration::from_secs(2),
			"The next DX12 frame sequence waited for the previous sequence's allocator. elapsed={elapsed:?}"
		);
		assert!(!device.has_errors());
	}

	#[test]
	fn later_sequence_submission_does_not_reassign_a_completed_texture_readback() {
		let Some((_instance, mut device, queue_handle)) = create_validated_device_setup() else {
			return;
		};
		device.set_frames_in_flight(2);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1))
				.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut first_copy = None;

		device.queue(queue_handle).execute(
			Some(crate::queue::FrameRequest::new(0, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |recording| {
					first_copy = Some(
						crate::command_buffer::CommandBufferRecording::transfer_texture(recording, image.into()).expect(
							"Texture transfer failed. The most likely cause is that the DX12 test image is not a valid transfer source.",
						),
					);
				});
				[]
			},
		);
		let first_copy = first_copy.expect(
			"Missing DX12 readback handle. The most likely cause is that the first sequence did not record its transfer.",
		);
		device.wait_for_texture_copy_readback(first_copy);

		let blocker = device
			.block_queue_until_test_fence(queue_handle)
			.expect("Failed to block the DX12 test queue. The most likely cause is that the test device was removed.");
		let mut second_copy = None;
		device.queue(queue_handle).execute(
			Some(crate::queue::FrameRequest::new(1, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |recording| {
					second_copy = Some(
						crate::command_buffer::CommandBufferRecording::transfer_texture(recording, image.into()).expect(
							"Texture transfer failed. The most likely cause is that the DX12 test image is not a valid transfer source.",
						),
					);
				});
				[]
			},
		);

		// A bad completion reassignment waits on the blocked second sequence.
		let elapsed = time_with_blocked_queue(&blocker, || device.wait_for_texture_copy_readback(first_copy));
		device.wait_for_synchronizer(synchronizer);
		let _ = device
			.get_image_data(first_copy)
			.expect("Texture mapping failed. The most likely cause is that the first sequence lost its completion ownership.");
		let _ = device
			.get_image_data(second_copy.expect(
				"Missing DX12 readback handle. The most likely cause is that the second sequence did not record its transfer.",
			))
			.expect("Texture mapping failed. The most likely cause is that the second sequence did not complete.");

		assert!(
			elapsed < std::time::Duration::from_secs(2),
			"A later DX12 frame sequence reassigned an earlier readback's completion fence. elapsed={elapsed:?}"
		);
		assert!(!device.has_errors());
	}

	#[test]
	fn dynamic_buffer_handles_do_not_alias_static_buffers() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let static_buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Uniform).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		let dynamic_buffer = device.build_dynamic_buffer::<[u32; 8]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::DeviceToHost),
		);

		assert_eq!(
			device.buffer_resource_state(static_buffer.into()),
			Some((crate::DeviceAccesses::CpuWrite, BufferHeapKind::Upload, true, true))
		);
		assert_eq!(
			device.buffer_resource_state(dynamic_buffer.into()),
			Some((crate::DeviceAccesses::DeviceToHost, BufferHeapKind::Readback, true, true))
		);

		device.resize_buffer(dynamic_buffer, std::mem::size_of::<[u32; 16]>());

		assert_eq!(
			device.buffer_resource_state(static_buffer.into()),
			Some((crate::DeviceAccesses::CpuWrite, BufferHeapKind::Upload, true, true))
		);
		assert_eq!(
			device
				.buffer_bytes(dynamic_buffer.into(), std::mem::size_of::<[u32; 16]>())
				.map(|bytes| bytes.len()),
			Some(64)
		);
	}

	#[test]
	fn narrow_storage_buffer_tails_have_complete_word_allocations() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let u16_tail = device.build_buffer::<[u16; 3]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let u8_tail = device.build_buffer::<[u8; 5]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);

		assert_eq!(device.buffer_native_size_for_sequence(u16_tail.into(), 0), Some(8));
		assert_eq!(device.buffer_native_size_for_sequence(u8_tail.into(), 0), Some(8));
	}

	#[test]
	fn dynamic_buffer_writes_are_sequence_local() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let buffer = device.build_dynamic_buffer::<[u32; 2]>(
			crate::buffer::Builder::new(crate::Uses::Uniform).device_accesses(crate::DeviceAccesses::CpuWrite),
		);

		*device.dynamic_buffer_slice_mut(buffer, 1) = [5, 9];
		device.sync_buffer_for_sequence(buffer, 1);

		assert_eq!(
			device.buffer_bytes_for_sequence(buffer.into(), std::mem::size_of::<[u32; 2]>(), 0),
			Some(vec![0, 0, 0, 0, 0, 0, 0, 0])
		);
		assert_eq!(
			device.buffer_bytes_for_sequence(buffer.into(), std::mem::size_of::<[u32; 2]>(), 1),
			Some(vec![5, 0, 0, 0, 9, 0, 0, 0])
		);
		assert_eq!(device.buffer_frame_resource_state(buffer.into(), 1), Some(true));
	}

	#[test]
	fn acceleration_structure_instances_write_dx12_layout() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let instance_buffer = device.create_acceleration_structure_instance_buffer(Some("instances"), 1);
		let bottom_level = device.create_bottom_level_acceleration_structure(&crate::BottomLevelAccelerationStructure {
			description: crate::BottomLevelAccelerationStructureDescriptions::AABB { transform_count: 1 },
		});
		let transform = [[1.0, 0.0, 0.0, 4.0], [0.0, 1.0, 0.0, 5.0], [0.0, 0.0, 1.0, 6.0]];

		device.write_instance(instance_buffer, usize::MAX, transform, 7, 0xff, 3, bottom_level);
		let unchanged = device
			.buffer_bytes(
				instance_buffer,
				std::mem::size_of::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_INSTANCE_DESC>(),
			)
			.expect("Instance buffer bytes should be available.");
		assert!(unchanged.iter().all(|byte| *byte == 0));
		assert_eq!(device.acceleration_structure_instance_write_count(), 0);

		device.write_instance(instance_buffer, 0, transform, 7, 0xff, 3, bottom_level);

		let bytes = device
			.buffer_bytes(
				instance_buffer,
				std::mem::size_of::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_INSTANCE_DESC>(),
			)
			.expect("Instance buffer bytes should be available.");
		// The buffer shadow is byte-aligned storage, so copy the descriptor without imposing native alignment on the slice.
		let instance = unsafe {
			std::ptr::read_unaligned(
				bytes
					.as_ptr()
					.cast::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_INSTANCE_DESC>(),
			)
		};

		assert_eq!(device.acceleration_structure_instance_write_count(), 1);
		assert_eq!(
			instance.Transform,
			[1.0, 0.0, 0.0, 4.0, 0.0, 1.0, 0.0, 5.0, 0.0, 0.0, 1.0, 6.0]
		);
		assert_eq!(instance._bitfield1, 0xff00_0007);
		assert_eq!(instance._bitfield2, 0x0400_0003);
		assert_ne!(instance.AccelerationStructure, 0);
	}

	#[test]
	fn texture3d_creation_uses_depth_instead_of_array_size() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::new(8, 4, 3)),
		);

		assert_eq!(
			device.image_native_dimension(image),
			Some((windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_DIMENSION_TEXTURE3D.0, 3,))
		);
	}

	#[test]
	fn texture3d_creation_rejects_array_metadata() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.build_image(
				crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image)
					.extent(::utils::Extent::new(8, 4, 3))
					.array_layers(std::num::NonZeroU32::new(2)),
			)
		}));

		assert!(result.is_err(), "DX12 accepted array metadata on a Texture3D resource.");
	}

	#[test]
	fn texture3d_uav_uses_the_selected_mip_depth() {
		let descriptor = Device::descriptor_texture_uav_desc(
			windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM,
			crate::TextureViewTypes::Texture3D,
			::utils::Extent::new(16, 8, 9),
			true,
			1,
			None,
			Some(2),
		);

		assert_eq!(
			descriptor.ViewDimension,
			windows::Win32::Graphics::Direct3D12::D3D12_UAV_DIMENSION_TEXTURE3D
		);
		let texture = unsafe { descriptor.Anonymous.Texture3D };
		assert_eq!(texture.MipSlice, 2);
		assert_eq!(texture.FirstWSlice, 0);
		assert_eq!(texture.WSize, 2);
	}

	#[test]
	fn unsupported_srgb_channel_layout_is_rejected_before_handle_publication() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};

		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.build_image(
				crate::image::Builder::new(crate::Formats::R8sRGB, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
			)
		}));

		assert!(
			result.is_err(),
			"DX12 accepted an sRGB format that has no exact DXGI representation."
		);
	}

	#[test]
	fn sampled_descriptor_rejects_image_without_sampled_usage() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Storage)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		let set = device.create_descriptor_set(None);
		device.write(&[crate::DescriptorWrite::image(set, slot, image, crate::Layouts::Read)]);
		let pipeline = create_metadata_compute_pipeline(
			&mut device,
			&[],
			[crate::ShaderResourceDescriptor::single(
				slot,
				crate::ResourceKind::SampledImage,
				crate::AccessPolicies::READ,
			)],
		);

		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.validate_descriptor_sets(pipeline, &[set], 0);
		}));
		assert!(
			result.is_err(),
			"DX12 accepted a sampled descriptor over a storage-only image."
		);
	}

	#[test]
	fn uniform_descriptor_rejects_buffer_without_uniform_usage() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let set = device.create_descriptor_set(None);
		let buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		device.write(&[crate::DescriptorWrite::buffer(set, slot, buffer.into())]);
		let pipeline = create_metadata_compute_pipeline(
			&mut device,
			&[],
			[crate::ShaderResourceDescriptor::single(
				slot,
				crate::ResourceKind::UniformBuffer,
				crate::AccessPolicies::READ,
			)],
		);

		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.validate_descriptor_sets(pipeline, &[set], 0);
		}));
		assert!(result.is_err(), "DX12 accepted a CBV over a buffer without uniform usage.");
	}

	#[test]
	fn uniform_descriptor_rejects_ranges_larger_than_64_kib() {
		let Some((_instance, mut device, _queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let set = device.create_descriptor_set(None);
		let buffer = device.build_buffer::<[u32; 16_385]>(
			crate::buffer::Builder::new(crate::Uses::Uniform).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		device.write(&[crate::DescriptorWrite::buffer(set, slot, buffer.into())]);
		let pipeline = create_metadata_compute_pipeline(
			&mut device,
			&[],
			[crate::ShaderResourceDescriptor::single(
				slot,
				crate::ResourceKind::UniformBuffer,
				crate::AccessPolicies::READ,
			)],
		);

		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.validate_descriptor_sets(pipeline, &[set], 0);
		}));
		assert!(result.is_err(), "DX12 accepted a shader-visible CBV larger than 64 KiB.");
	}

	#[test]
	fn readback_buffer_creation_rejects_every_gpu_read_or_binding_use() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let gpu_binding_uses = [
			crate::Uses::Uniform,
			crate::Uses::Storage,
			crate::Uses::Vertex,
			crate::Uses::Index,
			crate::Uses::Indirect,
			crate::Uses::AccelerationStructure,
			crate::Uses::AccelerationStructureBuild,
			crate::Uses::AccelerationStructureBuildScratch,
			crate::Uses::ShaderBindingTable,
			crate::Uses::TransferSource,
			crate::Uses::Clear,
		];

		for uses in gpu_binding_uses {
			let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				device.build_buffer::<[u32; 4]>(
					crate::buffer::Builder::new(uses | crate::Uses::TransferDestination)
						.device_accesses(crate::DeviceAccesses::DeviceToHost),
				)
			}));
			assert!(rejected.is_err(), "DX12 accepted a GPU binding use on a readback heap.");
		}

		let rejected_access = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			device.build_buffer::<[u32; 4]>(
				crate::buffer::Builder::new(crate::Uses::TransferDestination)
					.device_accesses(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::GpuRead),
			)
		}));
		assert!(
			rejected_access.is_err(),
			"DX12 accepted explicit GPU-read access on a readback heap."
		);

		let valid_readback = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let destination = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let rejected_copy_source = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			let mut recording = device.create_command_buffer_recording(command_buffer);
			crate::command_buffer::CommandBufferRecording::copy_buffers(
				&mut recording,
				&[crate::BufferCopyDescriptor::new(
					valid_readback.into(),
					0,
					destination.into(),
					0,
					std::mem::size_of::<[u32; 4]>(),
				)],
			);
		}));
		assert!(
			rejected_copy_source.is_err(),
			"DX12 accepted a readback resource as a GPU copy source."
		);
	}

	#[test]
	fn descriptor_binding_rejects_overlapping_buffer_read_write_aliases() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let read_slot = crate::ResourceSlot::new(0);
		let write_slot = crate::ResourceSlot::new(1);
		let set = device.create_descriptor_set(None);
		let buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		device.write(&[
			crate::DescriptorWrite::buffer(set, read_slot, buffer.into()),
			crate::DescriptorWrite::buffer(set, write_slot, buffer.into()),
		]);
		let pipeline = create_metadata_compute_pipeline(
			&mut device,
			&[],
			[
				crate::ShaderResourceDescriptor::single(
					read_slot,
					crate::ResourceKind::StorageBuffer,
					crate::AccessPolicies::READ,
				),
				crate::ShaderResourceDescriptor::single(
					write_slot,
					crate::ResourceKind::StorageBuffer,
					crate::AccessPolicies::WRITE,
				),
			],
		);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording.bind_compute_pipeline(pipeline);

		let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			recording.bind_descriptor_sets(&[set]);
		}));
		assert!(
			rejected.is_err(),
			"DX12 accepted overlapping whole-buffer SRV and UAV aliases."
		);
	}

	#[test]
	fn descriptor_binding_rejects_incompatible_image_layout_aliases() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let read_slot = crate::ResourceSlot::new(0);
		let write_slot = crate::ResourceSlot::new(1);
		let set = device.create_descriptor_set(None);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::Storage)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		device.write(&[
			crate::DescriptorWrite::image(set, read_slot, image, crate::Layouts::Read),
			crate::DescriptorWrite::image(set, write_slot, image, crate::Layouts::General),
		]);
		let pipeline = create_metadata_compute_pipeline(
			&mut device,
			&[],
			[
				crate::ShaderResourceDescriptor::single(
					read_slot,
					crate::ResourceKind::SampledImage,
					crate::AccessPolicies::READ,
				),
				crate::ShaderResourceDescriptor::single(
					write_slot,
					crate::ResourceKind::StorageImage,
					crate::AccessPolicies::WRITE,
				),
			],
		);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording.bind_compute_pipeline(pipeline);

		let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			recording.bind_descriptor_sets(&[set]);
		}));
		assert!(
			rejected.is_err(),
			"DX12 accepted simultaneous shader-resource and unordered-access image layouts."
		);
	}

	#[test]
	fn render_pass_rejects_an_image_still_bound_as_a_shader_resource() {
		let Some((_instance, mut device, queue_handle)) = create_default_device_setup() else {
			return;
		};
		let slot = crate::ResourceSlot::new(0);
		let set = device.create_descriptor_set(None);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::RenderTarget)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		device.write(&[crate::DescriptorWrite::image(set, slot, image, crate::Layouts::Read)]);
		let pipeline = create_metadata_compute_pipeline(
			&mut device,
			&[],
			[crate::ShaderResourceDescriptor::single(
				slot,
				crate::ResourceKind::SampledImage,
				crate::AccessPolicies::READ,
			)],
		);
		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording.bind_compute_pipeline(pipeline).bind_descriptor_sets(&[set]);
		let attachments = [crate::AttachmentInformation::new(
			image,
			crate::Layouts::RenderTarget,
			crate::LoadOp::Load,
			crate::StoreOp::Store,
		)];

		let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			crate::command_buffer::CommandBufferRecording::start_render_pass(
				&mut recording,
				::utils::Extent::rectangle(1, 1),
				&attachments,
			);
		}));
		assert!(
			rejected.is_err(),
			"DX12 accepted one whole image as both an attachment and shader resource."
		);
	}
}
