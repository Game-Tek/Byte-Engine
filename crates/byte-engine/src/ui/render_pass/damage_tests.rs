//! Damage regions, damage-filtered primitives, and the compute shader that composites the layer.

use besl::vm::{Buffer, DescriptorBindings, Value};
use utils::Extent;

use super::*;
use crate::rendering::{
	render_pass::simple_compute,
	shader_vm_test::{assert_rgba_close, compile as compile_shader_vm, rgba, run_at, slot, texture_2d},
};
use crate::ui::components::image::ImageSource;
use crate::ui::{Size, flow::Location3, layout::Geometry, style::LayerKind};

const UI_COMPOSITE_BESL: &str = include_str!("../../../assets/rendering/ui/composite.besl");
const UI_LAYER_COMPOSITE_BESL: &str = include_str!("../../../assets/rendering/ui/layer-composite.besl");

fn region(x: u32, y: u32, width: u32, height: u32) -> UiPixelRegion {
	UiPixelRegion {
		origin: [x, y],
		extent: Extent::rectangle(width, height),
	}
}

fn element(order: u32, position: [f32; 2], size: [f32; 2]) -> UiDrawElement {
	UiDrawElement {
		place: UiDrawPlace {
			order,
			position,
			size,
			..Default::default()
		},
		paint: UiPaint::flat([1.0; 4]),
		corner_radii: [0.0; 4],
		corner_exponent: 2.0,
		sector: None,
		fade: EdgeFeather::none(),
		layer_kind: LayerKind::Fill,
		stroke_widths: [0.0; 4],
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

/// Builds a backdrop blur of `radius` over a `size`-pixel quad at `position`.
fn blur_element(position: [f32; 2], size: [f32; 2], radius: f32) -> UiBlurDrawElement {
	UiBlurDrawElement {
		place: UiDrawPlace {
			order: 1,
			position,
			size,
			..Default::default()
		},
		color: [1.0; 4],
		corner_radii: [0.0; 4],
		corner_exponent: 2.0,
		sector: None,
		fade: EdgeFeather::none(),
		radius,
		path: None,
		shadow: None,
	}
}

/// Builds a 400-pixel draw list holding `blurs` at one layout pixel per screen pixel.
fn blur_list(blurs: Vec<UiBlurDrawElement>) -> UiDrawList {
	UiDrawList {
		layout_size: [400.0, 400.0],
		blurs,
		..UiDrawList::default()
	}
}

#[test]
fn blur_footprints_over_a_scene_cover_the_kernel_reads_every_frame() {
	let viewport = Extent::rectangle(400, 400);
	let draw_list = blur_list(vec![blur_element([100.0, 100.0], [50.0, 50.0], 8.0)]);
	let mut out = Vec::new();
	blur_footprints(&draw_list, viewport, true, &mut out);
	let sigma = blur_sigma(8.0);
	let margin = blur_read_margin(sigma);
	assert_eq!(
		out,
		vec![region(100 - margin, 100 - margin, 50 + margin * 2, 50 + margin * 2)]
	);
	// Every pixel the blur's first stages read lies inside the damage, whichever paths the radius uses.
	let bounds = [100.0, 100.0, 150.0, 150.0];
	let full_support = UiBlurKernel::gaussian(sigma).support;
	let half_support = UiBlurKernel::gaussian(blur_half_sigma(sigma)).support;
	let reads = blur_backdrop_reads(
		(blur_full_dispatch_regions(bounds, viewport, full_support), full_support),
		blur_half_dispatch_regions(bounds, viewport, half_support),
		blur_resolution_mix(sigma),
		viewport,
	);
	assert!(out[0].contains(reads));
}

#[test]
fn damage_a_blur_reads_redoes_it_and_every_blur_its_footprint_reaches() {
	let viewport = Extent::rectangle(400, 400);
	let first = blur_element([100.0, 100.0], [50.0, 50.0], 8.0);
	// Close enough that the two footprints overlap, though the quads do not.
	let second = blur_element([180.0, 100.0], [50.0, 50.0], 8.0);
	let untouched = blur_element([100.0, 320.0], [40.0, 40.0], 8.0);
	let draw_list = blur_list(vec![second, untouched, first]);
	let margin = blur_read_margin(blur_sigma(8.0));
	// One pixel inside the first blur's read margin, left of its quad.
	let touch = region(100 - margin, 120, 1, 1);
	let mut damage = vec![touch];
	blur_footprints(&draw_list, viewport, false, &mut damage);
	assert_eq!(
		damage,
		vec![
			touch,
			region(100 - margin, 100 - margin, 50 + margin * 2, 50 + margin * 2),
			region(180 - margin, 100 - margin, 50 + margin * 2, 50 + margin * 2),
		]
	);

	// Without a scene, blurs stay clean while nothing they read changes.
	let far = region(300, 300, 20, 20);
	let mut damage = vec![far];
	blur_footprints(&draw_list, viewport, false, &mut damage);
	assert_eq!(damage, vec![far]);
	damage.clear();
	blur_footprints(&draw_list, viewport, false, &mut damage);
	assert!(damage.is_empty());
}

#[test]
fn only_elements_touching_damage_get_primitives() {
	let frame_allocator = bumpalo::Bump::new();
	let draw_list = UiDrawList {
		layout_size: [100.0, 100.0],
		elements: vec![element(1, [0.0, 0.0], [10.0, 10.0]), element(2, [50.0, 50.0], [10.0, 10.0])],
		..UiDrawList::default()
	};
	let viewport = Extent::square(100);
	// Returns the left edges of the rectangles drawn, after the clear quad.
	let drawn = |damage: Option<&[UiPixelRegion]>| -> Vec<f32> {
		let output = build_ui_primitives(
			&draw_list,
			viewport,
			&frame_allocator,
			Vec::new(),
			None,
			&mut UiMaskTable::default(),
			None,
			None,
			damage,
		);
		output.primitives[1..].iter().map(|primitive| primitive.bounds[0]).collect()
	};
	assert_eq!(drawn(None), [0.0, 50.0]);
	assert_eq!(drawn(Some(&[region(55, 55, 2, 2)])), [50.0]);
	// The margin pulls in a neighbor that only touches the region by anti-aliasing distance.
	assert_eq!(drawn(Some(&[region(12, 12, 2, 2)])), [0.0]);
	assert!(drawn(Some(&[])).is_empty());
	assert!(damage_intersects(None, [1000.0, 1000.0, 1001.0, 1001.0]));
}

/// Runs a composite shader at `threads` invocations along one row, over `region`, with `images` bound by name. A
/// `target` sets where the result image starts on screen.
fn run_composite(
	source: &str,
	images: &mut [(&str, &mut besl::vm::Texture)],
	region: UiPixelRegion,
	target: Option<[u32; 2]>,
	threads: u32,
) {
	let builtins = besl::Builtins::new();
	let program = compile_shader_vm(&simple_compute::compile_test_program(&builtins, source));
	let mut push_constant = Buffer::new(program.push_constant_layout().expect("region push constants").clone());
	push_constant.write("origin", Value::Vec2U(region.origin)).unwrap();
	push_constant.write("extent", Value::Vec2U(region.push_extent())).unwrap();
	if let Some(target) = target {
		push_constant.write("target", Value::Vec2U(target)).unwrap();
	}
	// Dispatch rounding produces excess threads; the extent guard must make them no-ops.
	for x in 0..threads {
		let mut descriptors = DescriptorBindings::new();
		for (name, image) in images.iter_mut() {
			descriptors.bind_image(slot(&program, name), image);
		}
		descriptors.bind_push_constant(&mut push_constant);
		run_at(&program, &mut descriptors, [x, 0]);
	}
}

#[test]
fn composite_besl_vm_places_the_premultiplied_layer_inside_the_region() {
	// Premultiplied half-covered green, fully transparent, then a pixel outside the region.
	let layer = || texture_2d(3, 1, &[[0.0, 0.5, 0.0, 0.5], [0.0, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.5]]);
	let unwritten = || texture_2d(3, 1, &[[9.0; 4], [9.0; 4], [9.0; 4]]);

	// Over the scene.
	let mut scene = texture_2d(3, 1, &[[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 1.0]]);
	let (mut over, mut result) = (layer(), unwritten());
	let mut images = [("scene", &mut scene), ("layer", &mut over), ("result", &mut result)];
	run_composite(UI_COMPOSITE_BESL, &mut images, region(0, 0, 2, 1), None, 3);
	assert_rgba_close(rgba(&result, [0, 0]), [0.5, 0.5, 0.0, 1.0], 0.0001);
	assert_rgba_close(rgba(&result, [1, 0]), [0.0, 0.0, 1.0, 1.0], 0.0001);
	assert_rgba_close(rgba(&result, [2, 0]), [9.0; 4], 0.0);

	// Over black.
	let (mut over, mut result) = (layer(), unwritten());
	let mut images = [("layer", &mut over), ("result", &mut result)];
	run_composite(UI_LAYER_COMPOSITE_BESL, &mut images, region(0, 0, 2, 1), None, 3);
	assert_rgba_close(rgba(&result, [0, 0]), [0.0, 0.5, 0.0, 1.0], 0.0001);
	assert_rgba_close(rgba(&result, [1, 0]), [0.0, 0.0, 0.0, 1.0], 0.0001);
	assert_rgba_close(rgba(&result, [2, 0]), [9.0; 4], 0.0);
}

#[test]
fn composite_besl_vm_writes_a_blur_backdrop_where_its_window_starts() {
	let mut scene = texture_2d(4, 1, &[[1.0, 0.0, 0.0, 1.0]; 4]);
	let mut layer = texture_2d(
		4,
		1,
		&[[0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.5], [0.0; 4]],
	);
	// The backdrop image holds the screen from pixel 2 on.
	let mut result = texture_2d(2, 1, &[[9.0; 4], [9.0; 4]]);
	let mut images = [("scene", &mut scene), ("layer", &mut layer), ("result", &mut result)];
	run_composite(UI_COMPOSITE_BESL, &mut images, region(2, 0, 2, 1), Some([2, 0]), 2);
	assert_rgba_close(rgba(&result, [0, 0]), [0.5, 0.5, 0.0, 1.0], 0.0001);
	assert_rgba_close(rgba(&result, [1, 0]), [1.0, 0.0, 0.0, 1.0], 0.0001);
}

/// Creates an opaque GPU image handle, for draw lists that only name an image.
#[allow(
	unsafe_code,
	clippy::undocumented_unsafe_blocks,
	reason = "UI tests manufacture an opaque GHI handle without exposing a production constructor."
)]
fn gpu_image(value: u64) -> ghi::ImageHandle {
	assert_ne!(value, 0);
	// SAFETY: `ImageHandle` is a transparent wrapper over the nonzero opaque `u64` handle representation GHI uses.
	unsafe { std::mem::transmute(value) }
}

fn image_element(order: u32, position: [f32; 2], size: [f32; 2], source: ImageSource) -> UiImageDrawElement {
	UiImageDrawElement {
		place: UiDrawPlace {
			order,
			position,
			size,
			..Default::default()
		},
		image_id: u64::from(order),
		version: 0,
		source_width: 1,
		source_height: 1,
		source,
		opacity: 1.0,
	}
}

#[test]
fn gpu_images_are_redrawn_every_frame_and_bitmaps_only_when_they_change() {
	let viewport = Extent::rectangle(400, 400);
	let draw_list = UiDrawList {
		layout_size: [400.0, 400.0],
		images: vec![
			image_element(1, [100.0, 100.0], [50.0, 40.0], ImageSource::Gpu(gpu_image(7))),
			image_element(2, [200.0, 200.0], [50.0, 50.0], ImageSource::Pixels(vec![255; 4].into())),
		],
		..UiDrawList::default()
	};
	let mut out = Vec::new();
	gpu_image_footprints(&draw_list, viewport, &mut out);
	let margin = UI_DAMAGE_MARGIN_PIXELS as u32;
	assert_eq!(
		out,
		vec![region(100 - margin, 100 - margin, 50 + margin * 2, 40 + margin * 2)]
	);
}

#[test]
fn blur_images_grow_in_steps_with_the_viewports_parity_and_never_shrink() {
	let viewport = Extent::rectangle(1920, 1081);
	let first = blur_image_extent(Extent::square(0), [70, 30], viewport);
	assert_eq!(first, Extent::rectangle(128, 65));
	// A smaller blur keeps the images; a larger one grows them by whole steps.
	assert_eq!(blur_image_extent(first, [10, 10], viewport), first);
	assert_eq!(blur_image_extent(first, [130, 64], viewport), Extent::rectangle(192, 129));
	// They never pass the viewport.
	assert_eq!(blur_image_extent(first, [4000, 4000], viewport), viewport);
}

#[test]
fn a_blur_window_starts_even_and_ends_on_the_far_edge_it_reaches() {
	let viewport = Extent::rectangle(400, 301);
	let inside = region(101, 50, 60, 40);
	let image = blur_image_extent(Extent::square(0), [60, 40], viewport);
	assert_eq!(blur_window(inside, viewport, image), [100, 50]);

	let corner = region(331, 250, 69, 51);
	let image = blur_image_extent(Extent::square(0), [69, 51], viewport);
	let window = blur_window(corner, viewport, image);
	assert_eq!([window[0] + image.width(), window[1] + image.height()], [400, 301]);
	assert!(window.iter().all(|origin| origin % 2 == 0));
}
