pub mod command_buffer;
pub mod context;
pub mod device;
pub mod factory;
pub mod frame;
pub mod instance;
pub mod queue;

pub use self::command_buffer::*;
pub use self::context::*;
pub use self::device::*;
pub use self::frame::*;
pub use self::instance::*;
pub use self::queue::*;
mod utils;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		RasterizationRenderPassMode as _,
	};

	fn create_default_device_setup() -> (Instance, Device, crate::QueueHandle) {
		let features = crate::device::Features::new().validation(false);
		let mut instance = Instance::new(features).expect("Failed to create a DX12 instance.");
		let mut queue_handle = None;
		let device = instance
			.create_device(
				features,
				&mut [(
					crate::QueueSelection::new(crate::types::WorkloadTypes::RASTER),
					&mut queue_handle,
				)],
			)
			.expect("Failed to create DX12 GHI.");
		(instance, device, queue_handle.unwrap())
	}

	#[test]
	fn render_triangle() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		crate::graphics_hardware_interface::tests::render_triangle(&mut device, queue_handle);
	}

	#[test]
	fn multiframe_rendering() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		crate::graphics_hardware_interface::tests::multiframe_rendering(&mut device, queue_handle);
	}

	#[test]
	fn change_frames() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		crate::graphics_hardware_interface::tests::change_frames(&mut device, queue_handle);
	}

	#[test]
	fn resize() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		crate::graphics_hardware_interface::tests::resize(&mut device, queue_handle);
	}

	#[test]
	fn dynamic_data() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		crate::graphics_hardware_interface::tests::dynamic_data(&mut device, queue_handle);
	}

	#[test]
	fn dynamic_textures() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		crate::graphics_hardware_interface::tests::dynamic_textures(&mut device, queue_handle);
	}

	#[test]
	fn texture_slice_mut_updates_static_image_storage() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);

		device.get_texture_slice_mut(image).copy_from_slice(&[3, 4, 5, 6]);

		let copy = device.copy_image_to_cpu(image);
		assert_eq!(device.get_image_data(copy), &[3, 4, 5, 6]);
	}

	#[test]
	fn frame_texture_slice_mut_updates_dynamic_image_frame_storage() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		let synchronizer = device.create_synchronizer(None, false);

		let frame = device.start_frame(1, synchronizer);
		frame.get_texture_slice_mut(image.into()).copy_from_slice(&[7, 8, 9, 10]);
		drop(frame);

		let copy = device.copy_image_to_cpu_for_sequence(crate::ImageHandle(image.into()), 1);
		assert_eq!(device.get_image_data(copy), &[7, 8, 9, 10]);
		let copy = device.copy_image_to_cpu_for_sequence(crate::ImageHandle(image.into()), 0);
		assert_eq!(device.get_image_data(copy), &[0, 0, 0, 0]);
	}

	#[test]
	fn sync_texture_records_pending_static_image_upload() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		let readback = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1)),
		);
		device.get_texture_slice_mut(image).copy_from_slice(&[1, 2, 3, 4]);
		crate::context::Context::sync_texture(&mut device, image);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::blit_image(
			&mut recording,
			image.into(),
			crate::Layouts::Transfer,
			readback.into(),
			crate::Layouts::Transfer,
		);
		drop(recording);

		assert_eq!(device.upload_resource_count(), 1);
		assert_eq!(device.texture_copy_count(), 1);
	}

	#[test]
	fn frame_sync_texture_records_pending_dynamic_image_upload() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let command_buffer = device.create_command_buffer(None, queue_handle);

		let mut frame = device.start_frame(1, synchronizer);
		frame.get_texture_slice_mut(image.into()).copy_from_slice(&[5, 6, 7, 8]);
		frame.sync_texture(image.into());
		let mut recording = frame.create_command_buffer_recording(command_buffer);
		let copies = crate::command_buffer::CommandBufferRecording::transfer_textures(&mut recording, &[image.into()]);
		drop(recording);
		drop(frame);

		assert_eq!(device.get_image_data(copies[0]), &[5, 6, 7, 8]);
		assert_eq!(device.upload_resource_count(), 1);
		assert_eq!(device.readback_resource_count(), 1);
	}

	#[test]
	fn bind_descriptor_sets_flushes_pending_sampled_texture_upload() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let binding = crate::DescriptorSetBindingTemplate::combined_image_sampler(0, crate::Stages::FRAGMENT);
		let template = device.create_descriptor_set_template(None, &[binding.clone()]);
		let set = device.create_descriptor_set(None, &template);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		let sampler = device.build_sampler(crate::sampler::Builder::new());
		device.create_descriptor_binding(
			set,
			crate::BindingConstructor::combined_image_sampler(&binding, image, sampler, crate::Layouts::Read),
		);
		device.get_texture_slice_mut(image).copy_from_slice(&[17, 18, 19, 20]);
		crate::context::Context::sync_texture(&mut device, image);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::BoundPipelineLayoutMode::bind_descriptor_sets(&mut recording, &[set]);
		drop(recording);

		assert_eq!(device.upload_resource_count(), 1);
	}

	#[test]
	fn combined_image_sampler_writes_preserve_frame_offset() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let binding = crate::DescriptorSetBindingTemplate::combined_image_sampler(0, crate::Stages::FRAGMENT);
		let template = device.create_descriptor_set_template(None, &[binding.clone()]);
		let set = device.create_descriptor_set(None, &template);
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		let sampler = device.build_sampler(crate::sampler::Builder::new());
		let binding_handle = device.create_descriptor_binding(
			set,
			crate::BindingConstructor::combined_image_sampler(&binding, image, sampler, crate::Layouts::Read),
		);

		device.write(&[crate::descriptors::Write::combined_image_sampler_with_frame(
			binding_handle,
			image,
			sampler,
			crate::Layouts::Read,
			-1,
		)]);

		assert_eq!(device.descriptor_sequence_index(set, 0, 0), Some(1));
		assert_eq!(device.descriptor_sequence_index(set, 1, 0), Some(0));
	}

	#[test]
	fn combined_image_sampler_array_writes_preserve_frame_offset() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let binding = crate::DescriptorSetBindingTemplate::combined_image_sampler_array(0, crate::Stages::FRAGMENT, 2);
		let template = device.create_descriptor_set_template(None, &[binding.clone()]);
		let set = device.create_descriptor_set(None, &template);
		let image = device.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		let sampler = device.build_sampler(crate::sampler::Builder::new());
		let binding_handle =
			device.create_descriptor_binding(set, crate::BindingConstructor::combined_image_sampler_array(&binding));
		let descriptor_write_count = device.descriptor_write_count();
		let image_srv_descriptor_write_count = device.image_srv_descriptor_write_count();
		let sampler_descriptor_write_count = device.sampler_descriptor_write_records().len();

		device.write(&[crate::descriptors::Write::combined_image_sampler_array_with_frame(
			binding_handle,
			image,
			sampler,
			crate::Layouts::Read,
			1,
			-1,
		)]);

		assert_eq!(device.descriptor_sequence_index(set, 0, 0), Some(1));
		assert_eq!(device.descriptor_sequence_index(set, 1, 0), Some(0));
		assert_eq!(device.descriptor_write_count(), descriptor_write_count + 4);
		assert_eq!(
			device.image_srv_descriptor_write_count(),
			image_srv_descriptor_write_count + 2
		);
		assert_eq!(
			device.sampler_descriptor_write_records().len(),
			sampler_descriptor_write_count + 2
		);
	}

	#[test]
	fn descriptor_sets() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		crate::graphics_hardware_interface::tests::descriptor_sets(&mut device, queue_handle);
	}

	#[test]
	fn debug_regions_encode_native_command_list_events() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let command_buffer = device.create_command_buffer(Some("debug regions"), queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);

		crate::command_buffer::CommonCommandBufferMode::start_region(&recording, "outer");
		crate::command_buffer::CommonCommandBufferMode::end_region(&recording);
		crate::command_buffer::CommonCommandBufferMode::region(&mut recording, "inner", |_| {});
		drop(recording);

		assert_eq!(device.debug_region_begin_count(), 2);
		assert_eq!(device.debug_region_end_count(), 2);
	}

	#[test]
	fn descriptor_sets_create_native_heaps() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let bindings = [
			crate::DescriptorSetBindingTemplate::storage_buffer(0, crate::Stages::COMPUTE),
			crate::DescriptorSetBindingTemplate::combined_image_sampler(1, crate::Stages::FRAGMENT),
		];
		let template = device.create_descriptor_set_template(None, &bindings);
		let set = device.create_descriptor_set(None, &template);
		let buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(1, 1)),
		);
		let sampler = device.build_sampler(crate::sampler::Builder::new());

		device.create_descriptor_binding(set, crate::BindingConstructor::buffer(&bindings[0], buffer.into()));
		device.create_descriptor_binding(
			set,
			crate::BindingConstructor::combined_image_sampler(&bindings[1], image, sampler, crate::Layouts::Read),
		);

		assert_eq!(device.descriptor_set_has_native_heaps(set), Some((true, true)));
		assert_eq!(device.descriptor_write_count(), 6);
		assert_eq!(device.image_srv_descriptor_write_count(), 2);
		assert_eq!(device.image_uav_descriptor_write_count(), 0);
	}

	#[test]
	fn pipelines_create_native_root_signatures() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let template = device.create_descriptor_set_template(
			None,
			&[crate::DescriptorSetBindingTemplate::storage_image(0, crate::Stages::COMPUTE)],
		);
		let set = device.create_descriptor_set(None, &template);
		let shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Compute, [])
			.expect("Failed to create DX12 shader metadata.");
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[template],
			&[crate::pipelines::PushConstantRange::new(0, 16)],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		assert_eq!(
			device.pipeline_layout_has_root_signature(device.pipelines[pipeline.0 as usize].layout),
			Some(true)
		);

		let command_buffer = device.create_command_buffer(None, _queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommonCommandBufferMode::bind_compute_pipeline(&mut recording, pipeline)
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
	fn detached_factory_resources_intern_into_device() {
		use crate::factory::Factory as _;

		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let mut factory = device
			.create_factory()
			.expect("DX12 should expose a detached resource factory.");
		let vertex = factory
			.create_shader(
				Some("factory vertex"),
				crate::shader::Sources::DXIL(&[0, 0, 0, 0]),
				crate::ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create detached DX12 vertex shader.");
		let fragment = factory
			.create_shader(
				Some("factory fragment"),
				crate::shader::Sources::DXIL(&[0, 0, 0, 0]),
				crate::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create detached DX12 fragment shader.");
		let compute = factory
			.create_shader(
				Some("factory compute"),
				crate::shader::Sources::DXIL(&[0, 0, 0, 0]),
				crate::ShaderTypes::Compute,
				[],
			)
			.expect("Failed to create detached DX12 compute shader.");
		let vertex_elements = [crate::pipelines::VertexElement::new("POSITION", crate::DataTypes::Float3, 0)];
		let raster_shaders = [
			crate::pipelines::ShaderParameter::new(&vertex, crate::ShaderTypes::Vertex),
			crate::pipelines::ShaderParameter::new(&fragment, crate::ShaderTypes::Fragment),
		];
		let render_targets = [crate::pipelines::raster::AttachmentDescriptor::new(
			crate::Formats::RGBA8UNORM,
		)];
		let detached_raster = factory.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_elements,
			&raster_shaders,
			&render_targets,
		));
		let detached_compute = factory.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			&[],
			crate::pipelines::ShaderParameter::new(&compute, crate::ShaderTypes::Compute),
		));
		let detached_image = factory.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image).extent(::utils::Extent::rectangle(2, 2)),
		);
		let detached_sampler = factory.build_sampler(crate::sampler::Builder::new().anisotropy(2.0));
		let synchronizer = device.create_synchronizer(None, false);

		let mut frame = device.start_frame(0, synchronizer);
		let image = frame.intern_image(detached_image);
		let sampler = frame.intern_sampler(detached_sampler);
		let raster = frame.intern_raster_pipeline(detached_raster);
		let compute = frame.intern_compute_pipeline(detached_compute);
		drop(frame);

		assert_eq!(
			device.image_resource_state(image),
			Some((::utils::Extent::rectangle(2, 2), true))
		);
		assert_eq!(sampler.0, 0);
		assert_eq!(device.pipeline_has_native_state(raster), Some(false));
		assert_eq!(device.pipeline_has_native_state(compute), Some(false));
		assert_eq!(device.graphics_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.compute_pipeline_state_create_attempt_count(), 1);
	}

	#[test]
	fn compute_pipelines_attempt_native_state_from_dxil() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let shader = device
			.create_shader(
				None,
				crate::shader::Sources::DXIL(&[0, 0, 0, 0]),
				crate::ShaderTypes::Compute,
				[],
			)
			.expect("Failed to create DX12 DXIL shader metadata.");
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			&[],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		assert_eq!(device.compute_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(false));
	}

	#[test]
	fn hlsl_compute_shader_compiles_to_native_pipeline_state() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let shader = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "[numthreads(1, 1, 1)] void main(uint3 id : SV_DispatchThreadID) {}",
					entry_point: "main",
				},
				crate::ShaderTypes::Compute,
				[],
			)
			.expect("Failed to compile DX12 HLSL compute shader.");
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			&[],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		assert_eq!(device.compute_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(true));
	}

	#[test]
	fn platform_native_shader_source_selects_hlsl_for_dx12_pipeline_state() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let compiled = crate::shader::compile(
			"dx12-platform-native-compute",
			crate::shader::ShaderSource::PlatformNative {
				glsl: "#version 450\nlayout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;\nvoid main() {}",
				msl: "kernel void main0() {}",
				msl_entry_point: "main0",
				hlsl: "[numthreads(1, 1, 1)] void main(uint3 id : SV_DispatchThreadID) {}",
				hlsl_entry_point: "main",
			},
		)
		.expect("Failed to select the DX12 platform-native shader source.");
		let shader = device
			.create_shader(None, compiled.as_source(), crate::ShaderTypes::Compute, [])
			.expect("Failed to compile DX12 platform-native HLSL compute shader.");
		let shader_parameter = crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute);
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(&[], &[], shader_parameter));

		assert_eq!(device.compute_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(true));
	}

	#[test]
	fn hlsl_compute_pipeline_recompiles_with_specialization_macros() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let shader = device
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
			.expect("Failed to compile default DX12 HLSL compute shader.");
		let specialization = [crate::pipelines::SpecializationMapEntry::new(0, "f32".to_string(), 4.0f32)];
		let shader_parameter = crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute)
			.with_specialization_map(&specialization);
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(&[], &[], shader_parameter));

		assert_eq!(device.compute_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.hlsl_specialization_compile_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(true));
	}

	#[test]
	fn hlsl_compute_pipeline_specializes_scalar_macro_types() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
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
			crate::pipelines::SpecializationMapEntry::new(0, "bool".to_string(), true),
			crate::pipelines::SpecializationMapEntry::new(1, "u32".to_string(), 8u32),
			crate::pipelines::SpecializationMapEntry::new(2, "i32".to_string(), -3i32),
		];
		let shader_parameter = crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute)
			.with_specialization_map(&specialization);
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(&[], &[], shader_parameter));

		assert_eq!(device.compute_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.hlsl_specialization_compile_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(true));
	}

	#[test]
	fn detached_factory_compute_pipeline_preserves_hlsl_specialization_map() {
		use crate::factory::Factory as _;

		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let mut factory = device
			.create_factory()
			.expect("DX12 should expose a detached resource factory.");
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
		let specialization = [crate::pipelines::SpecializationMapEntry::new(0, "f32".to_string(), 8.0f32)];
		let detached_compute = factory.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
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
	fn raster_pipelines_attempt_native_state_from_dxil() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let vertex = device
			.create_shader(
				None,
				crate::shader::Sources::DXIL(&[0, 0, 0, 0]),
				crate::ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to create DX12 vertex shader metadata.");
		let fragment = device
			.create_shader(
				None,
				crate::shader::Sources::DXIL(&[0, 0, 0, 0]),
				crate::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create DX12 fragment shader metadata.");
		let vertex_elements = [
			crate::pipelines::VertexElement::new("POSITION", crate::DataTypes::Float3, 0),
			crate::pipelines::VertexElement::new("COLOR", crate::DataTypes::Float4, 0),
		];
		let shaders = [
			crate::pipelines::ShaderParameter::new(&vertex, crate::ShaderTypes::Vertex),
			crate::pipelines::ShaderParameter::new(&fragment, crate::ShaderTypes::Fragment),
		];
		let render_targets = [crate::pipelines::raster::AttachmentDescriptor::new(
			crate::Formats::RGBA8UNORM,
		)];
		let pipeline = device.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_elements,
			&shaders,
			&render_targets,
		));

		assert_eq!(device.graphics_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(false));
	}

	#[test]
	fn hlsl_raster_shaders_compile_to_native_pipeline_state() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
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
			&[],
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
	fn present_rendering_updates_acquired_swapchain_proxy_image() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let extent = ::utils::Extent::rectangle(65, 33);
		let window = crate::window::Window::new("DX12 Present Proxy Test", extent).expect("Failed to create DX12 test window.");
		let swapchain = device.bind_to_window(&window.os_handles(), Default::default(), extent, crate::Uses::RenderTarget);
		let vertices: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];
		let vertex_layout = [
			crate::pipelines::VertexElement::new("POSITION", crate::DataTypes::Float3, 0),
			crate::pipelines::VertexElement::new("COLOR", crate::DataTypes::Float4, 0),
		];
		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(vertices.as_ptr().cast(), std::mem::size_of_val(&vertices)),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr().cast(), 3 * std::mem::size_of::<u16>()),
				&vertex_layout,
			)
		};
		let vertex = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "
						struct VertexInput {
							float3 position : POSITION;
							float4 color : COLOR0;
						};
						struct VertexOutput {
							float4 position : SV_Position;
							float4 color : COLOR0;
						};
						VertexOutput main(VertexInput input) {
							VertexOutput output;
							output.position = float4(input.position, 1.0);
							output.color = input.color;
							return output;
						}
					",
					entry_point: "main",
				},
				crate::ShaderTypes::Vertex,
				[],
			)
			.expect("Failed to compile DX12 present vertex shader.");
		let fragment = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "float4 main(float4 color : COLOR0) : SV_Target { return color; }",
					entry_point: "main",
				},
				crate::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to compile DX12 present fragment shader.");
		let shaders = [
			crate::pipelines::ShaderParameter::new(&vertex, crate::ShaderTypes::Vertex),
			crate::pipelines::ShaderParameter::new(&fragment, crate::ShaderTypes::Fragment),
		];
		let render_targets = [crate::pipelines::raster::AttachmentDescriptor::new(crate::Formats::BGRAu8)];
		let pipeline = device.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&vertex_layout,
			&shaders,
			&render_targets,
		));
		let command_buffer = device.create_command_buffer(Some("present proxy"), queue_handle);
		let synchronizer = device.create_synchronizer(None, false);
		let present_key = {
			let mut frame = device.start_frame(0, synchronizer);
			let (present_key, _) = frame.acquire_swapchain_image(swapchain);
			let mut recording = frame.create_command_buffer_recording(command_buffer);
			let attachments = [crate::AttachmentInformation::new(
				swapchain,
				crate::Layouts::RenderTarget,
				crate::ClearValue::Color(::utils::RGBA::black()),
				false,
				true,
			)];
			let render_pass =
				crate::command_buffer::CommandBufferRecording::start_render_pass(&mut recording, extent, &attachments);
			let raster = render_pass.bind_raster_pipeline(pipeline);
			raster.draw_mesh(&mesh);
			render_pass.end_render_pass();
			crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);
			present_key
		};

		device.wait_for_synchronizer(synchronizer);
		device.present_swapchain(present_key);
		let proxy = device
			.get_swapchain_image_for_sequence(swapchain, crate::Uses::RenderTarget, present_key.sequence_index)
			.0;
		let copy = device.copy_image_to_cpu_for_sequence(proxy, present_key.sequence_index);
		let pixels = device.get_image_data(copy);

		assert_eq!(device.swapchain_backbuffer_bind_count(), 1);
		assert_eq!(device.swapchain_present_transition_count(), 1);
		assert_eq!(
			&pixels[((extent.width() / 2) * 4) as usize..((extent.width() / 2) * 4 + 4) as usize],
			&[255, 0, 0, 255]
		);
		let bottom_left = (extent.width() * (extent.height() - 1) * 4) as usize;
		assert_eq!(&pixels[bottom_left..bottom_left + 4], &[0, 0, 255, 255]);
		let bottom_right = ((extent.width() * extent.height() - 1) * 4) as usize;
		assert_eq!(&pixels[bottom_right..bottom_right + 4], &[0, 255, 0, 255]);
	}

	#[test]
	fn detached_factory_raster_pipeline_preserves_hlsl_specialization_map() {
		use crate::factory::Factory as _;

		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let mut factory = device
			.create_factory()
			.expect("DX12 should expose a detached resource factory.");
		let vertex = factory
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
			.expect("Failed to create detached DX12 HLSL vertex shader.");
		let fragment = factory
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "
						#ifndef SPEC_CONSTANT_0
						#define SPEC_CONSTANT_0 1.0
						#endif
						float4 main() : SV_Target {
							return float4(SPEC_CONSTANT_0, 0.0, 0.0, 1.0);
						}
					",
					entry_point: "main",
				},
				crate::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create detached DX12 HLSL fragment shader.");
		let specialization = [crate::pipelines::SpecializationMapEntry::new(0, "f32".to_string(), 0.5f32)];
		let shaders = [
			crate::pipelines::ShaderParameter::new(&vertex, crate::ShaderTypes::Vertex),
			crate::pipelines::ShaderParameter::new(&fragment, crate::ShaderTypes::Fragment)
				.with_specialization_map(&specialization),
		];
		let render_targets = [crate::pipelines::raster::AttachmentDescriptor::new(
			crate::Formats::RGBA8UNORM,
		)];
		let detached_raster = factory.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&[],
			&shaders,
			&render_targets,
		));
		let synchronizer = device.create_synchronizer(None, false);

		let mut frame = device.start_frame(0, synchronizer);
		let pipeline = frame.intern_raster_pipeline(detached_raster);
		drop(frame);

		assert_eq!(device.graphics_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.hlsl_specialization_compile_count(), 1);
		assert_eq!(
			device.pipeline_has_native_state(pipeline),
			Some(true),
			"last graphics PSO error: {:?}",
			device.graphics_pipeline_state_last_error()
		);
	}

	#[test]
	fn mesh_raster_pipelines_attempt_native_state_stream_from_dxil() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let mesh = device
			.create_shader(
				None,
				crate::shader::Sources::DXIL(&[0, 0, 0, 0]),
				crate::ShaderTypes::Mesh,
				[],
			)
			.expect("Failed to create DX12 mesh shader metadata.");
		let fragment = device
			.create_shader(
				None,
				crate::shader::Sources::DXIL(&[0, 0, 0, 0]),
				crate::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create DX12 fragment shader metadata.");
		let shaders = [
			crate::pipelines::ShaderParameter::new(&mesh, crate::ShaderTypes::Mesh),
			crate::pipelines::ShaderParameter::new(&fragment, crate::ShaderTypes::Fragment),
		];
		let render_targets = [crate::pipelines::raster::AttachmentDescriptor::new(
			crate::Formats::RGBA8UNORM,
		)];
		let pipeline = device.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&[],
			&shaders,
			&render_targets,
		));

		assert_eq!(device.graphics_pipeline_state_create_attempt_count(), 1);
		assert_eq!(device.pipeline_has_native_state(pipeline), Some(false));
	}

	#[test]
	fn mesh_raster_pipeline_accepts_sm6_hlsl_when_dxc_is_available() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let mesh = match device.create_shader(
			None,
			crate::shader::Sources::HLSL {
				source: r#"
struct MeshVertex {
	float4 position : SV_Position;
	float4 color : COLOR0;
};

[numthreads(1, 1, 1)]
[outputtopology("triangle")]
void main(out vertices MeshVertex vertices[3], out indices uint3 triangles[1]) {
	SetMeshOutputCounts(3, 1);
	vertices[0].position = float4(0.0, 0.5, 0.0, 1.0);
	vertices[0].color = float4(1.0, 0.0, 0.0, 1.0);
	vertices[1].position = float4(0.5, -0.5, 0.0, 1.0);
	vertices[1].color = float4(0.0, 1.0, 0.0, 1.0);
	vertices[2].position = float4(-0.5, -0.5, 0.0, 1.0);
	vertices[2].color = float4(0.0, 0.0, 1.0, 1.0);
	triangles[0] = uint3(0, 1, 2);
}
"#,
				entry_point: "main",
			},
			crate::ShaderTypes::Mesh,
			[],
		) {
			Ok(shader) => shader,
			Err(()) => return,
		};
		let fragment = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: "float4 main(float4 color : COLOR0) : SV_Target { return color; }",
					entry_point: "main",
				},
				crate::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to compile DX12 fragment HLSL.");
		let shaders = [
			crate::pipelines::ShaderParameter::new(&mesh, crate::ShaderTypes::Mesh),
			crate::pipelines::ShaderParameter::new(&fragment, crate::ShaderTypes::Fragment),
		];
		let render_targets = [crate::pipelines::raster::AttachmentDescriptor::new(
			crate::Formats::RGBA8UNORM,
		)];
		let pipeline = device.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&[],
			&shaders,
			&render_targets,
		));

		assert_eq!(device.graphics_pipeline_state_create_attempt_count(), 1);
		if device.supports_native_mesh_shaders() {
			assert_eq!(
				device.pipeline_has_native_state(pipeline),
				Some(true),
				"last graphics PSO error: {:?}",
				device.graphics_pipeline_state_last_error()
			);
		} else {
			assert_eq!(device.pipeline_has_native_state(pipeline), Some(false));
		}
	}

	#[test]
	fn compute_dispatch_skips_native_encoding_without_pipeline_state() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Compute, [])
			.expect("Failed to create DX12 shader metadata.");
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			&[],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommonCommandBufferMode::bind_compute_pipeline(&mut recording, pipeline).dispatch(
			crate::DispatchExtent::new(::utils::Extent::rectangle(8, 8), ::utils::Extent::rectangle(4, 4)),
		);
		drop(recording);

		assert_eq!(device.compute_dispatch_encode_count(), 0);
	}

	#[test]
	fn indirect_dispatch_encodes_native_command() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Compute, [])
			.expect("Failed to create DX12 shader metadata.");
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[],
			&[],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));
		let indirect_buffer = device.build_buffer::<[[u32; 4]; 2]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommonCommandBufferMode::bind_compute_pipeline(&mut recording, pipeline)
			.indirect_dispatch(indirect_buffer, 1);
		drop(recording);

		assert_eq!(device.indirect_dispatch_encode_count(), 1);
		assert_eq!(device.buffer_is_in_common_state(indirect_buffer.into()), Some(false));
	}

	#[test]
	fn raster_input_and_draw_calls_encode_native_commands() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let command_buffer = device.create_command_buffer(Some("native raster commands"), queue_handle);
		let vertex_shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Vertex, [])
			.expect("Failed to create DX12 vertex shader metadata.");
		let fragment_shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Fragment, [])
			.expect("Failed to create DX12 fragment shader metadata.");
		let pipeline = device.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&[],
			&[
				crate::pipelines::ShaderParameter::new(&vertex_shader, crate::ShaderTypes::Vertex),
				crate::pipelines::ShaderParameter::new(&fragment_shader, crate::ShaderTypes::Fragment),
			],
			&[crate::pipelines::raster::AttachmentDescriptor::new(
				crate::Formats::RGBA8UNORM,
			)],
		));
		let vertex_buffer = device.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(crate::Uses::Vertex).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		let index_buffer = device.build_buffer::<[u16; 3]>(
			crate::buffer::Builder::new(crate::Uses::Index).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		let mut recording = device.create_command_buffer_recording(command_buffer);

		recording.bind_raster_pipeline(pipeline);
		recording.bind_vertex_buffers(&[crate::BufferDescriptor::new(vertex_buffer).offset(16)]);
		recording.bind_index_buffer(&crate::BufferDescriptor::new(index_buffer).index_type(crate::DataTypes::U16));
		recording.draw(3, 1, 0, 0);
		recording.draw_indexed(3, 1, 0, 0, 0);
		drop(recording);

		assert_eq!(device.vertex_buffer_bind_count(), 1);
		assert_eq!(device.index_buffer_bind_count(), 1);
		assert_eq!(device.draw_encode_count(), 1);
		assert_eq!(device.draw_indexed_encode_count(), 1);
		assert_eq!(device.primitive_topology_set_count(), 1);
		assert_eq!(device.buffer_is_in_common_state(vertex_buffer.into()), Some(false));
		assert_eq!(device.buffer_is_in_common_state(index_buffer.into()), Some(false));
	}

	#[test]
	fn draw_mesh_binds_native_mesh_buffers() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let command_buffer = device.create_command_buffer(Some("native mesh draw"), queue_handle);
		let vertex_shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Vertex, [])
			.expect("Failed to create DX12 vertex shader metadata.");
		let fragment_shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Fragment, [])
			.expect("Failed to create DX12 fragment shader metadata.");
		let pipeline = device.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&[],
			&[
				crate::pipelines::ShaderParameter::new(&vertex_shader, crate::ShaderTypes::Vertex),
				crate::pipelines::ShaderParameter::new(&fragment_shader, crate::ShaderTypes::Fragment),
			],
			&[crate::pipelines::raster::AttachmentDescriptor::new(
				crate::Formats::RGBA8UNORM,
			)],
		));
		let vertices = [0u8; 3 * 7 * std::mem::size_of::<f32>()];
		let indices = [0u16, 1, 2];
		let indices = unsafe { std::slice::from_raw_parts(indices.as_ptr().cast::<u8>(), std::mem::size_of_val(&indices)) };
		let mesh = device.add_mesh_from_vertices_and_indices(
			3,
			3,
			&vertices,
			indices,
			&[
				crate::pipelines::VertexElement::new("POSITION", crate::DataTypes::Float3, 0),
				crate::pipelines::VertexElement::new("COLOR", crate::DataTypes::Float4, 0),
			],
		);

		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording.bind_raster_pipeline(pipeline);
		recording.draw_mesh(&mesh);
		drop(recording);

		assert_eq!(device.vertex_buffer_bind_count(), 1);
		assert_eq!(device.index_buffer_bind_count(), 1);
		assert_eq!(device.draw_indexed_encode_count(), 1);
	}

	#[test]
	fn dispatch_meshes_skips_native_encoding_without_mesh_pipeline_state() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let command_buffer = device.create_command_buffer(Some("native mesh dispatch"), queue_handle);
		let mesh_shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Mesh, [])
			.expect("Failed to create DX12 mesh shader metadata.");
		let fragment_shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Fragment, [])
			.expect("Failed to create DX12 fragment shader metadata.");
		let pipeline = device.create_raster_pipeline(crate::pipelines::raster::Builder::new(
			&[],
			&[],
			&[],
			&[
				crate::pipelines::ShaderParameter::new(&mesh_shader, crate::ShaderTypes::Mesh),
				crate::pipelines::ShaderParameter::new(&fragment_shader, crate::ShaderTypes::Fragment),
			],
			&[crate::pipelines::raster::AttachmentDescriptor::new(
				crate::Formats::RGBA8UNORM,
			)],
		));

		let mut recording = device.create_command_buffer_recording(command_buffer);
		recording.bind_raster_pipeline(pipeline);
		recording.dispatch_meshes(1, 2, 3);
		drop(recording);

		assert_eq!(device.pipeline_has_native_state(pipeline), Some(false));
		assert_eq!(device.mesh_dispatch_encode_count(), 0);
	}

	#[test]
	fn render_pass_binds_native_render_targets() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::RenderTarget)
				.extent(::utils::Extent::rectangle(1, 1))
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let depth = device.build_image(
			crate::image::Builder::new(crate::Formats::Depth32, crate::Uses::DepthStencil)
				.extent(::utils::Extent::rectangle(1, 1))
				.device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let command_buffer = device.create_command_buffer(Some("native render target"), queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		let attachments = [
			crate::AttachmentInformation::new(
				image.0,
				crate::Layouts::RenderTarget,
				crate::ClearValue::Integer(9, 10, 11, 12),
				false,
				true,
			),
			crate::AttachmentInformation::new(
				depth.0,
				crate::Layouts::RenderTarget,
				crate::ClearValue::Depth(1.0),
				false,
				true,
			),
		];

		let render_pass = crate::command_buffer::CommandBufferRecording::start_render_pass(
			&mut recording,
			::utils::Extent::rectangle(1, 1),
			&attachments,
		);
		render_pass.end_render_pass();
		drop(recording);

		assert_eq!(device.render_target_bind_count(), 1);
		assert_eq!(device.render_target_clear_count(), 1);
		assert_eq!(device.render_pass_end_count(), 1);
		assert_eq!(device.depth_stencil_bind_count(), 1);
		assert_eq!(device.depth_stencil_clear_count(), 1);
		assert_eq!(device.viewport_set_count(), 1);
		assert_eq!(device.scissor_set_count(), 1);
		assert_eq!(device.upload_resource_count(), 2);
	}

	#[test]
	fn descriptor_tables_bind_native_heap_offsets() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let bindings = [
			crate::DescriptorSetBindingTemplate::uniform_buffer(0, crate::Stages::COMPUTE),
			crate::DescriptorSetBindingTemplate::storage_image(1, crate::Stages::COMPUTE),
			crate::DescriptorSetBindingTemplate::combined_image_sampler(2, crate::Stages::COMPUTE),
		];
		let template = device.create_descriptor_set_template(None, &bindings);
		let set = device.create_descriptor_set(None, &template);
		let shader = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Compute, [])
			.expect("Failed to create DX12 shader metadata.");
		let pipeline = device.create_compute_pipeline(crate::pipelines::compute::Builder::new(
			&[template],
			&[],
			crate::pipelines::ShaderParameter::new(&shader, crate::ShaderTypes::Compute),
		));

		let command_buffer = device.create_command_buffer(None, _queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommonCommandBufferMode::bind_compute_pipeline(&mut recording, pipeline)
			.bind_descriptor_sets(&[set]);
		drop(recording);

		let records = device.descriptor_table_bind_records();
		assert_eq!(records.len(), 4);
		assert_eq!(records[0].heap_slot, 0);
		assert_eq!(records[1].heap_slot, 1);
		assert_eq!(records[2].heap_slot, 2);
		assert_eq!(records[3].heap_slot, 0);
		assert!(!records[2].sampler_heap);
		assert!(records[3].sampler_heap);
	}

	#[test]
	fn storage_images_create_native_uav_descriptors() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let binding = crate::DescriptorSetBindingTemplate::storage_image(0, crate::Stages::COMPUTE);
		let template = device.create_descriptor_set_template(None, &[binding.clone()]);
		let set = device.create_descriptor_set(None, &template);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Storage)
				.extent(::utils::Extent::rectangle(1, 1)),
		);

		device.create_descriptor_binding(set, crate::BindingConstructor::image(&binding, image));

		assert_eq!(device.descriptor_write_count(), 2);
		assert_eq!(device.image_srv_descriptor_write_count(), 0);
		assert_eq!(device.image_uav_descriptor_write_count(), 2);
	}

	#[test]
	fn samplers_create_native_descriptors_from_builder_state() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let binding = crate::DescriptorSetBindingTemplate::sampler(0, crate::Stages::FRAGMENT);
		let template = device.create_descriptor_set_template(None, &[binding.clone()]);
		let set = device.create_descriptor_set(None, &template);
		let sampler = device.build_sampler(
			crate::sampler::Builder::new()
				.filtering_mode(crate::FilteringModes::Closest)
				.mip_map_mode(crate::FilteringModes::Linear)
				.reduction_mode(crate::SamplingReductionModes::Max)
				.addressing_mode(crate::SamplerAddressingModes::Mirror)
				.anisotropy(12.0)
				.min_lod(2.0)
				.max_lod(8.0),
		);

		device.create_descriptor_binding(set, crate::BindingConstructor::sampler(&binding, sampler));

		let records = device.sampler_descriptor_write_records();
		assert_eq!(records.len(), 2);
		assert_eq!(records[0].filter.0, 469);
		assert_eq!(records[0].address_mode.0, 2);
		assert_eq!(records[0].max_anisotropy, 12);
		assert_eq!(records[0].min_lod, 2.0);
		assert_eq!(records[0].max_lod, 8.0);
	}

	#[test]
	fn multiframe_resources() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		crate::graphics_hardware_interface::tests::multiframe_resources(&mut device, queue_handle);
	}

	#[test]
	fn copy_buffers_updates_shadow_storage() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
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
	fn copy_to_device_only_buffer_records_gpu_copy() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let source = device.build_buffer::<[u8; 8]>(
			crate::buffer::Builder::new(crate::Uses::TransferSource).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let destination = device.build_buffer::<[u8; 8]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);

		*device.get_mut_buffer_slice(source) = [1, 2, 3, 4, 5, 6, 7, 8];
		device.sync_buffer(source);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::copy_buffers(
			&mut recording,
			&[crate::BufferCopyDescriptor::new(source.into(), 0, destination.into(), 0, 8)],
		);
		drop(recording);

		assert_eq!(device.buffer_copy_count(), 1);
		assert_eq!(device.buffer_is_in_common_state(destination.into()), Some(true));
	}

	#[test]
	fn copy_buffer_to_image_updates_shadow_storage() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let source = device.build_buffer::<[u8; 16]>(
			crate::buffer::Builder::new(crate::Uses::TransferSource).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(2, 2))
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);

		*device.get_mut_buffer_slice(source) = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
		device.sync_buffer(source);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::copy_buffer_to_images(
			&mut recording,
			&[crate::BufferImageCopyDescriptor::new(source.into(), 0, 8, 16, image.0)],
		);
		drop(recording);

		let copy = device.copy_image_to_cpu(image);
		assert_eq!(
			device.get_image_data(copy),
			&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
		);
		assert_eq!(device.upload_resource_count(), 1);
	}

	#[test]
	fn compressed_copy_buffer_to_image_uses_bc_block_layout() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let source = device.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(crate::Uses::TransferSource).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::BC7, crate::Uses::Image | crate::Uses::TransferDestination)
				.extent(::utils::Extent::rectangle(5, 7))
				.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let mut payload = [0u8; 64];
		for (index, byte) in payload.iter_mut().enumerate() {
			*byte = index as u8;
		}

		*device.get_mut_buffer_slice(source) = payload;
		device.sync_buffer(source);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::copy_buffer_to_images(
			&mut recording,
			&[crate::BufferImageCopyDescriptor::new(source.into(), 0, 32, 64, image.0)],
		);
		drop(recording);

		let copy = device.copy_image_to_cpu(image);
		assert_eq!(device.get_image_data(copy), payload);
		assert_eq!(device.upload_resource_count(), 1);
	}

	#[test]
	fn write_image_data_records_texture_upload() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1))
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let pixel = [7, 8, 9, 10];
		let data = unsafe { std::slice::from_raw_parts(pixel.as_ptr() as *const crate::RGBAu8, 1) };

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::write_image_data(&mut recording, image.into(), data);
		drop(recording);

		let copy = device.copy_image_to_cpu(image);
		assert_eq!(device.get_image_data(copy), &pixel);
		assert_eq!(device.upload_resource_count(), 1);
	}

	#[test]
	fn clear_images_records_texture_upload() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1))
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_images(
			&mut recording,
			&[(image.into(), crate::ClearValue::Integer(1, 2, 3, 4))],
		);
		drop(recording);

		let copy = device.copy_image_to_cpu(image);
		assert_eq!(device.get_image_data(copy), &[1, 2, 3, 4]);
		assert_eq!(device.upload_resource_count(), 1);
	}

	#[test]
	fn transfer_textures_records_readback_copy() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1))
				.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		device.write_texture(image, |pixels| pixels.copy_from_slice(&[11, 12, 13, 14]));

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		let copies = crate::command_buffer::CommandBufferRecording::transfer_textures(&mut recording, &[image.into()]);
		drop(recording);

		assert_eq!(device.get_image_data(copies[0]), &[11, 12, 13, 14]);
		assert_eq!(device.readback_resource_count(), 1);
	}

	#[test]
	fn transfer_textures_resolves_submitted_readback_copy() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferSource | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1))
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);
		let synchronizer = device.create_synchronizer(None, false);
		let pixel = [21, 22, 23, 24];
		let data = unsafe { std::slice::from_raw_parts(pixel.as_ptr() as *const crate::RGBAu8, 1) };

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::write_image_data(&mut recording, image.into(), data);
		let copies = crate::command_buffer::CommandBufferRecording::transfer_textures(&mut recording, &[image.into()]);
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);

		assert_eq!(device.get_image_data(copies[0]), &pixel);
		assert_eq!(device.readback_resource_count(), 1);
		assert_eq!(device.texture_readback_resolve_count(), 1);
	}

	#[test]
	fn command_buffer_execute_signals_synchronizer() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1)),
		);
		let synchronizer = device.create_synchronizer(None, false);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_images(
			&mut recording,
			&[(image.into(), crate::ClearValue::Integer(1, 2, 3, 4))],
		);
		crate::command_buffer::CommandBufferRecording::execute(recording, synchronizer);

		device.wait_for_synchronizer(synchronizer);
		assert_eq!(device.synchronizer_value(synchronizer), Some(1));
	}

	#[test]
	fn clear_buffers_updates_shadow_storage() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
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
	fn clear_device_only_buffer_records_gpu_copy() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::clear_buffers(&mut recording, &[buffer.into()]);
		drop(recording);

		assert_eq!(device.buffer_clear_count(), 1);
		assert_eq!(device.buffer_is_in_common_state(buffer.into()), Some(true));
	}

	#[test]
	fn device_to_host_buffers_use_readback_resources() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let buffer = device.build_buffer::<[u8; 16]>(
			crate::buffer::Builder::new(crate::Uses::TransferDestination).device_accesses(crate::DeviceAccesses::DeviceToHost),
		);

		assert_eq!(
			device.buffer_resource_state(buffer.into()),
			Some((crate::DeviceAccesses::DeviceToHost, BufferHeapKind::Readback, true, true))
		);
	}

	#[test]
	fn dynamic_buffer_handles_do_not_alias_static_buffers() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let static_buffer = device.build_buffer::<[u32; 4]>(
			crate::buffer::Builder::new(crate::Uses::Uniform).device_accesses(crate::DeviceAccesses::CpuWrite),
		);
		let dynamic_buffer = device.build_dynamic_buffer::<[u32; 8]>(
			crate::buffer::Builder::new(crate::Uses::Uniform).device_accesses(crate::DeviceAccesses::DeviceToHost),
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
	fn acceleration_structures_allocate_device_resources() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let top_level = device.create_top_level_acceleration_structure(Some("top"), 3);
		let bottom_level = device.create_bottom_level_acceleration_structure(&crate::BottomLevelAccelerationStructure {
			description: crate::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count: 3,
				vertex_position_encoding: crate::Encodings::FloatingPoint,
				triangle_count: 1,
				index_format: crate::DataTypes::U32,
			},
		});

		assert_eq!(device.acceleration_structure_resource_count(), 2);
		assert!(device.native_acceleration_structure_resource_count() <= 2);
		assert_eq!(device.acceleration_structure_size(top_level), Some(512));
		assert_eq!(device.bottom_level_acceleration_structure_size(bottom_level), Some(256));
		assert_ne!(device.acceleration_structure_gpu_address(top_level), Some(0));
		assert_ne!(device.bottom_level_acceleration_structure_gpu_address(bottom_level), Some(0));
	}

	#[test]
	fn acceleration_structure_descriptors_create_native_srv() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let binding = crate::DescriptorSetBindingTemplate::acceleration_structure(0, crate::Stages::RAYGEN);
		let template = device.create_descriptor_set_template(None, &[binding.clone()]);
		let set = device.create_descriptor_set(None, &template);
		let top_level = device.create_top_level_acceleration_structure(Some("top"), 1);

		device.create_descriptor_binding(set, crate::BindingConstructor::acceleration_structure(&binding, top_level));

		assert_eq!(device.descriptor_write_count(), 2);
		assert_eq!(device.acceleration_structure_descriptor_write_count(), 2);
	}

	#[test]
	fn acceleration_structure_instances_write_dx12_layout() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let instance_buffer = device.create_acceleration_structure_instance_buffer(Some("instances"), 1);
		let bottom_level = device.create_bottom_level_acceleration_structure(&crate::BottomLevelAccelerationStructure {
			description: crate::BottomLevelAccelerationStructureDescriptions::AABB { transform_count: 1 },
		});
		let transform = [[1.0, 0.0, 0.0, 4.0], [0.0, 1.0, 0.0, 5.0], [0.0, 0.0, 1.0, 6.0]];

		device.write_instance(instance_buffer, 0, transform, 7, 0xff, 3, bottom_level);

		let bytes = device
			.buffer_bytes(
				instance_buffer,
				std::mem::size_of::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_INSTANCE_DESC>(),
			)
			.expect("Instance buffer bytes should be available.");
		let instance =
			unsafe { *(bytes.as_ptr() as *const windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_INSTANCE_DESC) };

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
	fn shader_binding_table_entries_write_placeholder_identifier() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let raygen = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::RayGen, [])
			.expect("Failed to create DX12 raygen shader metadata.");
		let miss = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Miss, [])
			.expect("Failed to create DX12 miss shader metadata.");
		let pipeline = device.create_ray_tracing_pipeline(crate::pipelines::ray_tracing::Builder::new(
			&[],
			&[],
			&[
				crate::pipelines::ShaderParameter::new(&raygen, crate::ShaderTypes::RayGen),
				crate::pipelines::ShaderParameter::new(&miss, crate::ShaderTypes::Miss),
			],
		));
		let sbt = device.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
		);

		device.write_sbt_entry(sbt.into(), 0, pipeline, raygen);
		device.write_sbt_entry(sbt.into(), 32, pipeline, miss);

		let bytes = device.buffer_bytes(sbt.into(), 64).expect("SBT bytes should be available.");
		assert_eq!(device.shader_binding_table_write_count(), 2);
		assert_eq!(&bytes[0..8], b"DX12SBT\0");
		assert_eq!(&bytes[32..40], b"DX12SBT\0");
		assert_ne!(&bytes[0..32], &bytes[32..64]);
	}

	#[test]
	fn ray_tracing_pipelines_attempt_native_state_object_from_dxil() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let raygen = device
			.create_shader(None, crate::shader::Sources::DXIL(&[0u8; 4]), crate::ShaderTypes::RayGen, [])
			.expect("Failed to create DX12 raygen shader metadata.");
		let miss = device
			.create_shader(None, crate::shader::Sources::DXIL(&[1u8; 4]), crate::ShaderTypes::Miss, [])
			.expect("Failed to create DX12 miss shader metadata.");
		let hit = device
			.create_shader(
				None,
				crate::shader::Sources::DXIL(&[2u8; 4]),
				crate::ShaderTypes::ClosestHit,
				[],
			)
			.expect("Failed to create DX12 closest-hit shader metadata.");

		let pipeline = device.create_ray_tracing_pipeline(crate::pipelines::ray_tracing::Builder::new(
			&[],
			&[],
			&[
				crate::pipelines::ShaderParameter::new(&raygen, crate::ShaderTypes::RayGen),
				crate::pipelines::ShaderParameter::new(&miss, crate::ShaderTypes::Miss),
				crate::pipelines::ShaderParameter::new(&hit, crate::ShaderTypes::ClosestHit),
			],
		));

		assert_eq!(device.ray_tracing_state_object_create_attempt_count(), 1);
		assert_eq!(device.pipeline_has_ray_tracing_state_object(pipeline), Some(false));
		assert_eq!(device.ray_tracing_shader_identifier_count(pipeline), Some(0));
	}

	#[test]
	fn ray_tracing_pipeline_accepts_sm6_hlsl_libraries_when_dxc_is_available() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let raygen = match device.create_shader(
			None,
			crate::shader::Sources::HLSL {
				source: r#"[shader("raygeneration")] void raygen() {}"#,
				entry_point: "raygen",
			},
			crate::ShaderTypes::RayGen,
			[],
		) {
			Ok(shader) => shader,
			Err(()) => return,
		};
		let miss = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: r#"
struct Payload {
	float4 color;
};

[shader("miss")]
void miss(inout Payload payload) {
	payload.color = float4(0.0, 0.0, 0.0, 1.0);
}
"#,
					entry_point: "miss",
				},
				crate::ShaderTypes::Miss,
				[],
			)
			.expect("Failed to compile DX12 miss HLSL library.");
		let hit = device
			.create_shader(
				None,
				crate::shader::Sources::HLSL {
					source: r#"
struct Payload {
	float4 color;
};

[shader("closesthit")]
void closesthit(inout Payload payload, in BuiltInTriangleIntersectionAttributes attributes) {
	payload.color = float4(attributes.barycentrics, 0.0, 1.0);
}
"#,
					entry_point: "closesthit",
				},
				crate::ShaderTypes::ClosestHit,
				[],
			)
			.expect("Failed to compile DX12 closest-hit HLSL library.");

		let pipeline = device.create_ray_tracing_pipeline(crate::pipelines::ray_tracing::Builder::new(
			&[],
			&[],
			&[
				crate::pipelines::ShaderParameter::new(&raygen, crate::ShaderTypes::RayGen),
				crate::pipelines::ShaderParameter::new(&miss, crate::ShaderTypes::Miss),
				crate::pipelines::ShaderParameter::new(&hit, crate::ShaderTypes::ClosestHit),
			],
		));

		assert_eq!(device.ray_tracing_state_object_create_attempt_count(), 1);
		if device.supports_native_ray_tracing() {
			assert_eq!(device.pipeline_has_ray_tracing_state_object(pipeline), Some(true));
			assert_eq!(device.ray_tracing_shader_identifier_count(pipeline), Some(3));
		} else {
			assert_eq!(device.pipeline_has_ray_tracing_state_object(pipeline), Some(false));
		}
	}

	#[test]
	fn trace_rays_records_shader_table_dispatch_metadata() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let command_buffer = device.create_command_buffer(Some("trace rays"), queue_handle);
		let raygen = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::RayGen, [])
			.expect("Failed to create DX12 raygen shader metadata.");
		let miss = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::Miss, [])
			.expect("Failed to create DX12 miss shader metadata.");
		let hit = device
			.create_shader(None, crate::shader::Sources::SPIRV(&[]), crate::ShaderTypes::ClosestHit, [])
			.expect("Failed to create DX12 closest-hit shader metadata.");
		let pipeline = device.create_ray_tracing_pipeline(crate::pipelines::ray_tracing::Builder::new(
			&[],
			&[],
			&[
				crate::pipelines::ShaderParameter::new(&raygen, crate::ShaderTypes::RayGen),
				crate::pipelines::ShaderParameter::new(&miss, crate::ShaderTypes::Miss),
				crate::pipelines::ShaderParameter::new(&hit, crate::ShaderTypes::ClosestHit),
			],
		));
		let raygen_sbt = device.build_buffer::<[u8; 32]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let miss_sbt = device.build_buffer::<[u8; 32]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		let hit_sbt = device.build_buffer::<[u8; 32]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		device.write_sbt_entry(raygen_sbt.into(), 0, pipeline, raygen);
		device.write_sbt_entry(miss_sbt.into(), 0, pipeline, miss);
		device.write_sbt_entry(hit_sbt.into(), 0, pipeline, hit);

		let mut recording = device.create_command_buffer_recording(command_buffer);
		let ray_tracing = crate::command_buffer::CommonCommandBufferMode::bind_ray_tracing_pipeline(&mut recording, pipeline);
		crate::command_buffer::BoundRayTracingPipelineMode::trace_rays(
			ray_tracing,
			crate::rt::BindingTables {
				raygen: crate::BufferStridedRange::new(raygen_sbt.into(), 0, 32, 32),
				miss: crate::BufferStridedRange::new(miss_sbt.into(), 0, 32, 32),
				hit: crate::BufferStridedRange::new(hit_sbt.into(), 0, 32, 32),
				callable: None,
			},
			16,
			8,
			1,
		);

		assert_eq!(device.trace_rays_record_count(), 1);
	}

	#[test]
	fn acceleration_structure_builds_record_resource_usage() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let command_buffer = device.create_command_buffer(Some("as build"), queue_handle);
		let top_level = device.create_top_level_acceleration_structure(Some("top"), 1);
		let bottom_level = device.create_bottom_level_acceleration_structure(&crate::BottomLevelAccelerationStructure {
			description: crate::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count: 3,
				vertex_position_encoding: crate::Encodings::FloatingPoint,
				triangle_count: 1,
				index_format: crate::DataTypes::U16,
			},
		});
		let instances = device.create_acceleration_structure_instance_buffer(Some("instances"), 1);
		let scratch = device.build_buffer::<[u8; 256]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let vertices = device.build_buffer::<[[f32; 3]; 3]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);
		let indices = device.build_buffer::<[u16; 3]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::DeviceOnly),
		);

		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::build_bottom_level_acceleration_structures(
			&mut recording,
			&[crate::rt::BottomLevelAccelerationStructureBuild {
				acceleration_structure: bottom_level,
				scratch_buffer: scratch.into(),
				description: crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
					vertex_buffer: crate::BufferStridedRange::new(vertices.into(), 0, std::mem::size_of::<[f32; 3]>(), 36),
					vertex_count: 3,
					vertex_position_encoding: crate::Encodings::FloatingPoint,
					index_buffer: crate::BufferStridedRange::new(indices.into(), 0, std::mem::size_of::<u16>(), 6),
					triangle_count: 1,
					index_format: crate::DataTypes::U16,
				},
			}],
		);
		crate::command_buffer::CommandBufferRecording::build_top_level_acceleration_structure(
			&mut recording,
			&crate::rt::TopLevelAccelerationStructureBuild {
				acceleration_structure: top_level,
				scratch_buffer: scratch.into(),
				description: crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance {
					instances_buffer: instances,
					instance_count: 1,
				},
			},
		);
		drop(recording);

		assert_eq!(device.bottom_level_acceleration_structure_build_record_count(), 1);
		assert_eq!(device.top_level_acceleration_structure_build_record_count(), 1);
		assert!(device.native_bottom_level_acceleration_structure_build_encode_count() <= 1);
		assert!(device.native_top_level_acceleration_structure_build_encode_count() <= 1);
		assert_eq!(device.buffer_is_in_common_state(scratch.into()), Some(false));
		assert_eq!(device.buffer_is_in_common_state(instances), Some(false));
	}

	#[test]
	fn blit_image_records_texture_copy() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		let source = device.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(::utils::Extent::rectangle(1, 1)),
		);
		let destination = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(1, 1))
			.device_accesses(crate::DeviceAccesses::DeviceToHost),
		);

		device.write_texture(source, |pixels| pixels.copy_from_slice(&[10, 20, 30, 40]));

		let command_buffer = device.create_command_buffer(None, queue_handle);
		let mut recording = device.create_command_buffer_recording(command_buffer);
		crate::command_buffer::CommandBufferRecording::blit_image(
			&mut recording,
			source.into(),
			crate::Layouts::Read,
			destination.into(),
			crate::Layouts::Transfer,
		);
		drop(recording);

		let copy = device.copy_image_to_cpu(destination);
		assert_eq!(device.get_image_data(copy), &[10, 20, 30, 40]);
		assert_eq!(device.texture_copy_count(), 1);
		assert_eq!(device.image_is_in_common_state(source), Some(true));
		assert_eq!(device.image_is_in_common_state(destination), Some(true));
	}

	#[test]
	fn image_creation_and_resize_allocate_dx12_resources() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();
		let image = device.build_image(
			crate::image::Builder::new(
				crate::Formats::RGBA8UNORM,
				crate::Uses::Image | crate::Uses::TransferDestination,
			)
			.extent(::utils::Extent::rectangle(2, 2)),
		);

		assert_eq!(
			device.image_resource_state(image),
			Some((::utils::Extent::rectangle(2, 2), true))
		);

		device.resize_image_internal(image, ::utils::Extent::rectangle(4, 4));

		assert_eq!(
			device.image_resource_state(image),
			Some((::utils::Extent::rectangle(4, 4), true))
		);
	}

	#[test]
	fn compressed_images_allocate_dx12_resources() {
		let (_instance, mut device, _queue_handle) = create_default_device_setup();

		for format in [crate::Formats::BC5, crate::Formats::BC7, crate::Formats::BC7SRGB] {
			let image = device.build_image(
				crate::image::Builder::new(format, crate::Uses::Image | crate::Uses::TransferDestination)
					.extent(::utils::Extent::rectangle(8, 8)),
			);

			assert_eq!(
				device.image_resource_state(image),
				Some((::utils::Extent::rectangle(8, 8), true))
			);
		}
	}
}
