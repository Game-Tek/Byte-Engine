#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot};

	use crate::rendering::{
		render_pass::simple_compute,
		shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d},
	};

	const BILATERAL_X_SHADER: &str = include_str!("../../../assets/rendering/bilateral-blur/x.besl");
	const BILATERAL_Y_SHADER: &str = include_str!("../../../assets/rendering/bilateral-blur/y.besl");

	/// Executes one canonical bilateral shader against deterministic texture fixtures.
	fn run_bilateral_vm(
		shader: &str,
		extent: [u32; 2],
		depth_texels: &[[f32; 4]],
		source_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(shader));
		let mut depth = texture_2d(extent[0], extent[1], depth_texels);
		let mut source = texture_2d(extent[0], extent[1], source_texels);
		let mut result = empty_image(extent[0], extent[1]);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut depth);
		descriptors.bind_texture(ResourceSlot::new(1), &mut source);
		descriptors.bind_image(ResourceSlot::new(2), &mut result);
		run_at(&program, &mut descriptors, coordinate);
		drop(descriptors);
		rgba(&result, coordinate)
	}

	/// Verifies that both canonical blur axes preserve a constant source.
	#[test]
	fn bilateral_blur_besl_vm_preserves_constant_input_in_both_axes() {
		for shader in [BILATERAL_X_SHADER, BILATERAL_Y_SHADER] {
			let output = run_bilateral_vm(shader, [1, 1], &[[0.5, 0.0, 0.0, 1.0]], &[[0.375, 0.9, 0.1, 0.2]], [0, 0]);
			assert_rgba_close(output, [0.375, 0.375, 0.375, 1.0], 1e-5);
		}
	}

	/// Verifies that depth rejection blocks horizontal edge bleed while the vertical kernel preserves its column.
	#[test]
	fn bilateral_blur_besl_vm_respects_depth_edges_and_direction() {
		let depth = [
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
		];
		let source = [
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
		];

		let horizontal = run_bilateral_vm(BILATERAL_X_SHADER, [3, 3], &depth, &source, [1, 1]);
		let vertical = run_bilateral_vm(BILATERAL_Y_SHADER, [3, 3], &depth, &source, [1, 1]);

		assert_rgba_close(horizontal, [0.0, 0.0, 0.0, 1.0], 1e-6);
		assert_rgba_close(vertical, [0.2, 0.2, 0.2, 1.0], 1e-5);
	}
}
