//! Damage regions, damage-filtered geometry, and the compute shaders that clear and composite the layer.

use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, Value};
use utils::Extent;

use super::*;
use crate::rendering::{
	render_pass::simple_compute,
	shader_vm_test::{assert_rgba_close, compile as compile_shader_vm, empty_image, rgba, run_at, texture_2d},
};
use crate::ui::{Size, flow::Location3, layout::Geometry, style::LayerKind};

const UI_COMPOSITE_BESL: &str = include_str!("../../../assets/rendering/ui/composite.besl");

fn region(x: u32, y: u32, width: u32, height: u32) -> UiPixelRegion {
	UiPixelRegion {
		origin: [x, y],
		extent: Extent::rectangle(width, height),
	}
}

fn element(order: u32, position: [f32; 2], size: [f32; 2]) -> UiDrawElement {
	UiDrawElement {
		depth: 0,
		order,
		position,
		size,
		clip: None,
		clip_mask: None,
		color: [1.0, 1.0, 1.0, 1.0],
		corner_radius: 0.0,
		corner_exponent: 2.0,
		layer_kind: LayerKind::Fill,
		stroke_width: 0.0,
	}
}

#[test]
fn layout_damage_becomes_padded_pixel_regions() {
	let viewport = Extent::rectangle(200, 100);
	let mut out = Vec::new();
	// Layout is half the pixel size on each axis.
	pixel_damage(
		&[Geometry::new(Location3::new(10.0, 10.0, 0), Size::new(5.0, 5.0))],
		[100.0, 50.0],
		viewport,
		&mut out,
	);
	let margin = UI_DAMAGE_MARGIN_PIXELS as u32;
	assert_eq!(out, vec![region(20 - margin, 20 - margin, 10 + margin * 2, 10 + margin * 2)]);

	out.clear();
	pixel_damage(
		&[Geometry::new(Location3::new(-10.0, 0.0, 0), Size::new(200.0, 1.0))],
		[100.0, 50.0],
		viewport,
		&mut out,
	);
	assert_eq!(out[0].origin, [0, 0]);
	assert_eq!(out[0].end(), [200, 2 + margin]);
}

#[test]
fn overlapping_damage_merges_and_large_damage_becomes_full() {
	let viewport = Extent::rectangle(100, 100);
	let mut damage = vec![
		region(0, 0, 10, 10),
		region(5, 5, 10, 10),
		region(50, 50, 5, 5),
		region(0, 0, 0, 3),
	];
	merge_damage(&mut damage, viewport);
	assert_eq!(damage, vec![region(0, 0, 15, 15), region(50, 50, 5, 5)]);

	let mut damage: Vec<_> = (0..MAX_UI_DAMAGE_REGIONS as u32 + 1)
		.map(|index| region(index * 10, 0, 5, 5))
		.collect();
	merge_damage(&mut damage, viewport);
	assert_eq!(damage, vec![region(0, 0, MAX_UI_DAMAGE_REGIONS as u32 * 10 + 5, 5)]);

	let mut damage = vec![region(0, 0, 80, 80)];
	merge_damage(&mut damage, viewport);
	assert_eq!(damage, vec![UiPixelRegion::full(viewport)]);
}

#[test]
fn blur_footprints_cover_the_kernel_reads_every_frame() {
	let viewport = Extent::rectangle(400, 400);
	let draw_list = UiDrawList {
		layout_size: [400.0, 400.0],
		blurs: vec![UiBlurDrawElement {
			depth: 0,
			order: 1,
			position: [100.0, 100.0],
			size: [50.0, 50.0],
			clip: None,
			clip_mask: None,
			color: [1.0; 4],
			corner_radius: 0.0,
			corner_exponent: 2.0,
			radius: 8.0,
		}],
		..UiDrawList::default()
	};
	let mut out = Vec::new();
	blur_footprints(&draw_list, viewport, &mut out);
	let margin = UI_BLUR_FOOTPRINT_MARGIN;
	assert_eq!(
		out,
		vec![region(100 - margin, 100 - margin, 50 + margin * 2, 50 + margin * 2)]
	);
	assert!(margin >= UI_BLUR_GAUSSIAN_SUPPORT * UI_BLUR_HALF_DOWNSCALE);
}

#[test]
fn only_elements_touching_damage_get_geometry() {
	let frame_allocator = bumpalo::Bump::new();
	let draw_list = UiDrawList {
		layout_size: [100.0, 100.0],
		elements: vec![element(1, [0.0, 0.0], [10.0, 10.0]), element(2, [50.0, 50.0], [10.0, 10.0])],
		..UiDrawList::default()
	};
	let viewport = Extent::square(100);
	let all = build_ui_geometry_damaged(&draw_list, viewport, &frame_allocator, None, None);
	assert_eq!(all.indices.len(), 2 * UI_INDICES_PER_ELEMENT);

	let damage = [region(55, 55, 2, 2)];
	let partial = build_ui_geometry_damaged(&draw_list, viewport, &frame_allocator, None, Some(&damage));
	assert_eq!(partial.indices.len(), UI_INDICES_PER_ELEMENT);
	assert_eq!(partial.batches[0].order, 2);

	// The margin pulls in a neighbor that only touches the region by anti-aliasing distance.
	let damage = [region(12, 12, 2, 2)];
	let neighbor = build_ui_geometry_damaged(&draw_list, viewport, &frame_allocator, None, Some(&damage));
	assert_eq!(neighbor.batches[0].order, 1);

	let nothing = build_ui_geometry_damaged(&draw_list, viewport, &frame_allocator, None, Some(&[]));
	assert!(nothing.batches.is_empty());
	assert!(damage_intersects(None, [1000.0, 1000.0, 1001.0, 1001.0]));
}

fn region_push_constant(executable: &ExecutableProgram, region: UiPixelRegion) -> Buffer {
	let mut push_constant = Buffer::new(
		executable
			.push_constant_layout()
			.expect("Missing region push constants. The most likely cause is a changed production shader interface.")
			.clone(),
	);
	push_constant
		.write("origin", Value::Vec2U(region.origin))
		.expect("Failed to write the region origin.");
	push_constant
		.write("extent", Value::Vec2U(region.push_extent()))
		.expect("Failed to write the region extent.");
	push_constant
}

#[test]
fn composite_besl_vm_places_the_premultiplied_layer_over_the_scene_inside_the_region() {
	let program = compile_shader_vm(simple_compute::compile_test_program(UI_COMPOSITE_BESL));
	let mut scene = texture_2d(3, 1, &[[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 1.0]]);
	// Premultiplied half-covered green, fully transparent, then a pixel outside the region.
	let mut layer = texture_2d(3, 1, &[[0.0, 0.5, 0.0, 0.5], [0.0, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.5]]);
	let mut result = texture_2d(3, 1, &[[9.0; 4], [9.0; 4], [9.0; 4]]);
	let mut push_constant = region_push_constant(&program, region(0, 0, 2, 1));
	// Dispatch rounding produces excess threads; the extent guard must make them no-ops.
	for x in 0..3 {
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(besl::vm::ResourceSlot::new(0), &mut scene);
		descriptors.bind_image(besl::vm::ResourceSlot::new(1), &mut layer);
		descriptors.bind_image(besl::vm::ResourceSlot::new(2), &mut result);
		descriptors.bind_push_constant(&mut push_constant);
		run_at(&program, &mut descriptors, [x, 0]);
	}
	assert_rgba_close(rgba(&result, [0, 0]), [0.5, 0.5, 0.0, 1.0], 0.0001);
	assert_rgba_close(rgba(&result, [1, 0]), [0.0, 0.0, 1.0, 1.0], 0.0001);
	assert_rgba_close(rgba(&result, [2, 0]), [9.0; 4], 0.0);
}

#[test]
fn clear_quad_covers_the_viewport_with_transparent_black() {
	let quad = clear_quad();
	assert_eq!(
		quad.iter().map(|vertex| vertex.position).collect::<Vec<_>>(),
		vec![[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]]
	);
	for vertex in quad {
		assert_eq!(vertex.color, [0.0; 4]);
		// No rounded corner and no feather mask, so the rectangle shader's coverage is one everywhere.
		assert_eq!(vertex.corner_radius, 0.0);
		assert_eq!(vertex.clip_mask_size, [0.0, 0.0]);
	}
}
