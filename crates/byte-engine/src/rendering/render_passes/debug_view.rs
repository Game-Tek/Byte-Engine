//! The renderer-owned pass that tiles the shown debug views over a sink's presented image.
//!
//! The renderer builds one per sink as the last node of its frame and records it only while a view is shown; see
//! [`crate::rendering::debug_views`].

use ghi::context::ContextCreate as _;

use crate::rendering::{
	Sink,
	debug_views::DEBUG_VIEW_TARGETS_BINDING,
	render_pass::{RenderPassBuilder, RenderPassReturn, simple_compute},
};

/// The `DebugViewPresentShaderData` struct mirrors `DebugViewPresentParameters` of `debug-view/present.besl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DebugViewPresentShaderData {
	/// One bit per pool slot, set while a view holds it.
	taken: u32,
	/// How many bits of `taken` are set.
	count: u32,
	/// The grid's side: the smallest whole number whose square holds `count` views.
	side: u32,
	_padding: u32,
}

/// The `DebugViewPresentPass` struct tiles one sink's shown debug views into its final output.
pub(crate) struct DebugViewPresentPass {
	pass: simple_compute::Pass,
	parameters: ghi::DynamicBufferHandle<DebugViewPresentShaderData>,
}

impl DebugViewPresentPass {
	const PIPELINE: &'static str = "byte-engine/rendering/debug-view/present.pipeline";

	/// Creates the pass that tiles `pool` into `output`, the sink's swapchain.
	pub(crate) fn new(
		render_pass_builder: &mut RenderPassBuilder<'_>,
		pool: &[ghi::BaseImageHandle],
		output: ghi::ImageOrSwapchain,
	) -> Self {
		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Debug Views", Self::PIPELINE),
		);
		let parameters = render_pass_builder.context().build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Debug View Present Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let pass = pipeline.bind(
			"Debug View Present Descriptor Set",
			&[
				simple_compute::Resource::image_array(DEBUG_VIEW_TARGETS_BINDING, pool),
				simple_compute::Resource::image("result", output),
				simple_compute::Resource::buffer("parameters", parameters),
			],
		);
		Self { pass, parameters }
	}

	/// Prepares the tiling of the slots `taken` marks, or nothing while the shader is still compiling.
	pub(crate) fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
		taken: u32,
	) -> Option<RenderPassReturn<'a>> {
		use ghi::frame::Frame as _;

		let command = self.pass.prepare(frame, sink, frame_allocator)?;
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = present_shader_data(taken);
		Some(command)
	}
}

/// Lays out the slots `taken` marks on the smallest square grid that holds them.
fn present_shader_data(taken: u32) -> DebugViewPresentShaderData {
	let count = taken.count_ones();
	DebugViewPresentShaderData {
		taken,
		count,
		side: (count as f64).sqrt().ceil() as u32,
		_padding: 0,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const PRESENT_SHADER: &str = include_str!("../../../assets/rendering/debug-view/present.besl");

	#[test]
	fn grids_hold_every_shown_view() {
		assert_eq!(present_shader_data(0).side, 0);
		assert_eq!(present_shader_data(0b1).side, 1);
		assert_eq!(present_shader_data(0b101).side, 2);
		assert_eq!(present_shader_data(0b1111).side, 2);
		assert_eq!(present_shader_data(0b11111).side, 3);
		assert_eq!(present_shader_data(0b11111).count, 5);
	}

	#[test]
	fn present_besl_lexes() {
		simple_compute::compile_test_program(PRESENT_SHADER);
	}

	/// Two shown views share a 4x4 output in a 2x2 grid: the first cell averages the first slot's block, the second
	/// cell the second slot's, and the cells past the last view are black.
	#[test]
	fn present_besl_vm_tiles_the_taken_slots_in_slot_order() {
		use besl::vm::{DescriptorBindings, ResourceSlot, Value};

		use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, compile, empty_image, rgba, run_at, texture_2d};

		let program = compile(simple_compute::compile_test_program(PRESENT_SHADER));
		let result_slot = ResourceSlot::new(0);
		let parameter_slot = ResourceSlot::new(1);
		let pool_slot = |slot: u32| ResourceSlot::new(2 + slot);
		// Slots 1 and 3 are taken; slot 1 is red and slot 3 fades from black to white along x.
		let data = present_shader_data(0b1010);
		let mut parameters = buffer(&program, parameter_slot);
		for (name, value) in [
			("taken", Value::U32(data.taken)),
			("count", Value::U32(data.count)),
			("side", Value::U32(data.side)),
		] {
			parameters.write(name, value).expect("present parameters");
		}
		let mut red = texture_2d(4, 4, &[[1.0, 0.0, 0.0, 1.0]; 16]);
		let ramp = (0..16).map(|index| [(index % 4) as f32 / 3.0; 4]).collect::<Vec<_>>();
		let mut ramp = texture_2d(4, 4, &ramp);
		let (mut unused_first, mut unused_second) = (empty_image(4, 4), empty_image(4, 4));
		let mut result = empty_image(4, 4);

		for y in 0..4 {
			for x in 0..4 {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_image(result_slot, &mut result);
				descriptors.bind_buffer(parameter_slot, &mut parameters);
				descriptors.bind_image(pool_slot(0), &mut unused_first);
				descriptors.bind_image(pool_slot(1), &mut red);
				descriptors.bind_image(pool_slot(2), &mut unused_second);
				descriptors.bind_image(pool_slot(3), &mut ramp);
				run_at(&program, &mut descriptors, [x, y]);
			}
		}

		assert_rgba_close(rgba(&result, [0, 0]), [1.0, 0.0, 0.0, 1.0], 0.0);
		assert_rgba_close(rgba(&result, [1, 1]), [1.0, 0.0, 0.0, 1.0], 0.0);
		// The second cell's first pixel averages the ramp's columns 0 and 1; its second pixel columns 2 and 3.
		assert_rgba_close(rgba(&result, [2, 0]), [1.0 / 6.0, 1.0 / 6.0, 1.0 / 6.0, 1.0], 0.001);
		assert_rgba_close(rgba(&result, [3, 1]), [5.0 / 6.0, 5.0 / 6.0, 5.0 / 6.0, 1.0], 0.001);
		assert_rgba_close(rgba(&result, [0, 2]), [0.0, 0.0, 0.0, 1.0], 0.0);
		assert_rgba_close(rgba(&result, [3, 3]), [0.0, 0.0, 0.0, 1.0], 0.0);
	}
}
