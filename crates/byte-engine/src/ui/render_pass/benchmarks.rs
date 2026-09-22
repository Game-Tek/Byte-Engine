//! CPU renderer stages, without a window, device, command recording, or GPU submission.
//!
//! Run `cargo bench -p byte-engine --bench ui_render --features ui-render-bench`.
//! Fixtures use real UI snapshots; layout and font loading happen outside measurement.
//! Warm geometry resets the same frame arena each iteration. Buffer-copy measurements
//! use CPU destinations and exclude GHI mapping, synchronization, and driver costs.

use divan::{Bencher, black_box};

use super::*;
use crate::ui::{
	ConcreteLayer, ConcreteStyle, Container, Context, Curve, CurvePath, ElementContext, Engine, Image, Size, Text,
};

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn viewport() -> Extent {
	Extent::rectangle(1920, 1080)
}

#[derive(Clone, Copy)]
enum Scene {
	Rectangles,
	Text,
	Curves,
	Images,
	Blur,
	Mixed,
}

/// Builds a visible grid and retains its render independently of the layout engine.
fn scene(count: usize, kind: Scene, alternate: bool) -> engine::Render {
	let mut engine = Engine::new();
	engine.mount(move |ctx| {
		std::boxed::Box::pin(async move {
			let mut root = ctx
				.element("root")
				.container(Container::default().style(ConcreteStyle::new()));
			for index in 0..count {
				let fill = ConcreteLayer::default().color(RGBA::new(0.2, 0.4, 0.7, if alternate { 0.7 } else { 0.9 }).into());
				let mut card = root.element("card").container(
					Container::default()
						.absolute_position((index % 40 * 48) as u32, (index / 40 * 40) as u32)
						.width(46.into())
						.height(38.into())
						.depth(crate::ui::Depth::Relative((index % 4) as i32 + 1))
						.corner_radius(4.0)
						.style(if matches!(kind, Scene::Blur | Scene::Mixed) {
							ConcreteStyle::from_layers([fill.clone().backdrop_blur(8.0), fill.clone().stroke(1.0)])
						} else {
							fill.clone().into()
						}),
				);
				if matches!(kind, Scene::Text | Scene::Mixed) {
					card.element("label")
						.text(Text::new(format!("{} {index:04}", if alternate { "B" } else { "A" })).font_size(10.0));
				}
				if matches!(kind, Scene::Curves | Scene::Mixed) {
					card.element("curve").curve(
						Curve::new(CurvePath::new(40.into(), 30.into()).cubic(
							(1.0, 25.0),
							(8.0, 0.0),
							(32.0, 30.0),
							(39.0, 5.0),
						))
						.style(fill.stroke(2.0)),
					);
				}
				if matches!(kind, Scene::Images) {
					card.element("image")
						.image(Image::from_rgba(4, 4, vec![255; 64]).size(24.into()));
				}
			}
		})
	});
	let allocator = bumpalo::Bump::new();
	let mut snapshot = engine.evaluate(Size::new(1920, 1080), &allocator);
	engine.render(&mut snapshot).clone()
}

/// Converts a real render once so geometry benchmarks exclude data adoption.
fn draw_list(count: usize, kind: Scene) -> UiDrawList {
	let mut data = UiDrawList::default();
	update_from_render(&scene(count, kind, false), &mut data);
	data
}

/// Reports live output and retained arena bytes outside timing when diagnostics are requested.
fn report_primitives(name: &str, count: usize, arena: &bumpalo::Bump, primitives: usize, draws: usize) {
	if std::env::var_os("UI_RENDER_BENCH_STATS").is_some() {
		eprintln!(
			"{name}/{count}: primitive_bytes={} draws={draws} arena_bytes={}",
			primitives * std::mem::size_of::<UiPrimitive>(),
			arena.allocated_bytes()
		);
	}
}

/// Builds a frame's primitives without caches or text. The first record is the clear quad.
fn primitives<'a>(data: &UiDrawList, masks: &mut UiMaskTable, arena: &'a bumpalo::Bump) -> UiPrimitives<'a> {
	masks.clear();
	build_ui_primitives_uncached(data, viewport(), arena, masks)
}

/// Alternates prepared render snapshots so every adoption represents changed content.
fn transfer(bencher: Bencher, count: usize, kind: Scene) {
	let renders = [scene(count, kind, false), scene(count, kind, true)];
	let mut data = UiDrawList::default();
	for render in &renders {
		update_from_render(render, &mut data);
	}
	let mut index = 0;
	bencher.bench_local(|| {
		index ^= 1;
		update_from_render(black_box(&renders[index]), &mut data);
		black_box(&data);
	});
}

#[divan::bench(args = [100, 1000])]
fn transfer_text(bencher: Bencher, count: usize) {
	transfer(bencher, count, Scene::Text);
}

#[divan::bench(args = [100, 1000])]
fn transfer_curves(bencher: Bencher, count: usize) {
	transfer(bencher, count, Scene::Curves);
}

#[divan::bench(args = [100, 1000])]
fn transfer_images(bencher: Bencher, count: usize) {
	transfer(bencher, count, Scene::Images);
}

#[divan::bench(args = [100, 1000])]
/// Measures ownership of a snapshot independently of draw-list adoption.
fn clone_render(bencher: Bencher, count: usize) {
	let render = scene(count, Scene::Mixed, false);
	bencher.bench_local(|| drop(black_box(render.clone())));
}

#[divan::bench(args = [100, 1000])]
/// Builds rectangle primitives in a reused frame arena.
fn rectangles(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Rectangles);
	let mut arena = bumpalo::Bump::new();
	let mut masks = UiMaskTable::default();
	assert_eq!(primitives(&data, &mut masks, &arena).primitives.len(), count + 1);
	bencher.bench_local(|| {
		arena.reset();
		black_box(primitives(black_box(&data), &mut masks, &arena));
	});
}

#[divan::bench(args = [100, 1000])]
/// Picks piece counts for cubic curves and writes their records in a reused arena.
fn curves(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Curves);
	let mut arena = bumpalo::Bump::new();
	let mut masks = UiMaskTable::default();
	// Bump reset retains its largest chunk; warm through growth before steady measurements.
	for _ in 0..3 {
		arena.reset();
		black_box(primitives(&data, &mut masks, &arena));
	}
	arena.reset();
	let output = primitives(&data, &mut masks, &arena);
	assert!(!output.truncated && output.primitives.len() > count * 2);
	report_primitives("curves", count, &arena, output.primitives.len(), output.steps.len());
	drop(output);
	bencher.bench_local(|| {
		arena.reset();
		black_box(primitives(black_box(&data), &mut masks, &arena));
	});
}

#[divan::bench(args = [100, 1000])]
/// Builds image primitives without creating textures or descriptors.
fn images(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Images);
	let mut arena = bumpalo::Bump::new();
	let mut masks = UiMaskTable::default();
	assert_eq!(primitives(&data, &mut masks, &arena).images.len(), count);
	bencher.bench_local(|| {
		arena.reset();
		black_box(primitives(black_box(&data), &mut masks, &arena));
	});
}

#[divan::bench(args = [100, 1000])]
/// Builds blur primitives, dispatch regions, and Gaussian kernels, with the stroke each blurred card also has.
fn blur_regions_and_kernels(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Blur);
	let mut arena = bumpalo::Bump::new();
	let mut masks = UiMaskTable::default();
	let blurs = |output: &UiPrimitives| output.steps.iter().filter(|step| matches!(step, UiStep::Blur(_))).count();
	assert_eq!(blurs(&primitives(&data, &mut masks, &arena)), count);
	bencher.bench_local(|| {
		arena.reset();
		black_box(primitives(black_box(&data), &mut masks, &arena));
	});
}

#[divan::bench(args = [100, 1000])]
/// Builds text geometry with glyph bitmaps and atlas residency already cached.
fn text_warm(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Text);
	let mut arena = bumpalo::Bump::new();
	let mut system = TextSystem::new();
	let mut atlas = UiGlyphAtlas::new(UI_GLYPH_ATLAS_INITIAL_SIZE);
	let mut masks = UiMaskTable::default();
	for _ in 0..3 {
		arena.reset();
		black_box(build_ui_text_geometry(
			&data,
			viewport(),
			&mut system,
			&mut atlas,
			&mut masks,
			&arena,
		));
	}
	arena.reset();
	let geometry = build_ui_text_geometry(&data, viewport(), &mut system, &mut atlas, &mut masks, &arena);
	assert!(!geometry.truncated && geometry.dropped_glyphs == 0 && geometry.primitives.len() >= count);
	report_primitives("text", count, &arena, geometry.primitives.len(), 0);
	drop(geometry);
	bencher.bench_local(|| {
		arena.reset();
		black_box(build_ui_text_geometry(
			black_box(&data),
			viewport(),
			&mut system,
			&mut atlas,
			&mut masks,
			&arena,
		));
	});
}

#[divan::bench(args = [64, 512])]
/// Packs cached glyph bitmaps into a fresh atlas, including any growth and repacking.
fn atlas_growth(bencher: Bencher, initial_size: u32) {
	let mut system = TextSystem::new();
	let keys: Vec<_> = [8.0, 10.0, 12.0, 16.0, 20.0, 24.0, 32.0, 48.0]
		.into_iter()
		.flat_map(|size| ('A'..='Z').map(move |character| crate::ui::font::GlyphKey::new(character, size)))
		.collect();
	for &key in &keys {
		assert!(system.glyph_by_key(key).is_some());
	}
	bencher
		.with_inputs(|| UiGlyphAtlas::new(initial_size))
		.bench_local_refs(|atlas| {
			for &key in black_box(&keys) {
				black_box(atlas.ensure(key, &mut system));
			}
			assert_eq!(atlas.len(), keys.len());
		});
}

#[divan::bench(args = [512, 2048])]
/// Measures the CPU copy portion of a full dirty-atlas upload.
fn atlas_cpu_copy(bencher: Bencher, size: u32) {
	let atlas = UiGlyphAtlas::new(size);
	let mut destination = vec![0; atlas.pixels().len()];
	bencher.bench_local(|| {
		destination.copy_from_slice(black_box(atlas.pixels()));
		black_box(&destination);
	});
}

#[divan::bench(args = [100, 1000])]
/// Includes glyph rasterization and arena growth, with font loading outside timing.
fn text_cold_glyphs(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Text);
	bencher
		.with_inputs(|| {
			let mut system = TextSystem::new();
			assert!(system.has_font());
			(system, UiGlyphAtlas::new(UI_GLYPH_ATLAS_INITIAL_SIZE), bumpalo::Bump::new())
		})
		.bench_local_refs(|(system, atlas, arena)| {
			black_box(build_ui_text_geometry(
				black_box(&data),
				viewport(),
				system,
				atlas,
				&mut UiMaskTable::default(),
				arena,
			));
		});
}

#[divan::bench(args = [100, 1000])]
/// Builds Slug text geometry with every glyph's curves and bands already packed.
fn slug_text_warm(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Text);
	let mut arena = bumpalo::Bump::new();
	let mut system = TextSystem::new();
	let mut glyphs = UiGlyphCurves::new(UI_GLYPH_CURVE_CAPACITY, UI_GLYPH_BAND_CAPACITY);
	let mut masks = UiMaskTable::default();
	let geometry = build_ui_slug_geometry(&data, viewport(), &mut system, &mut glyphs, &mut masks, &arena);
	assert!(!geometry.truncated && geometry.dropped_glyphs == 0 && geometry.primitives.len() >= count);
	report_primitives("slug text", count, &arena, geometry.primitives.len(), 0);
	drop(geometry);
	bencher.bench_local(|| {
		arena.reset();
		black_box(build_ui_slug_geometry(
			black_box(&data),
			viewport(),
			&mut system,
			&mut glyphs,
			&mut masks,
			&arena,
		));
	});
}

#[divan::bench(args = [100, 1000])]
/// Includes reading outlines and packing curves and bands, with font loading outside timing.
fn slug_text_cold_glyphs(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Text);
	bencher
		.with_inputs(|| {
			let mut system = TextSystem::new();
			assert!(system.has_font());
			(
				system,
				UiGlyphCurves::new(UI_GLYPH_CURVE_CAPACITY, UI_GLYPH_BAND_CAPACITY),
				bumpalo::Bump::new(),
			)
		})
		.bench_local_refs(|(system, glyphs, arena)| {
			black_box(build_ui_slug_geometry(
				black_box(&data),
				viewport(),
				system,
				glyphs,
				&mut UiMaskTable::default(),
				arena,
			));
		});
}

#[divan::bench(args = [100, 1000])]
/// Merges rectangles, blurs, curves, and glyphs into one painter-ordered primitive stream.
fn merge_mixed_frame(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Mixed);
	let mut arena = bumpalo::Bump::new();
	let mut masks = UiMaskTable::default();
	let mut system = TextSystem::new();
	let mut glyphs = UiGlyphCurves::new(UI_GLYPH_CURVE_CAPACITY, UI_GLYPH_BAND_CAPACITY);
	let text_arena = bumpalo::Bump::new();
	let text = build_ui_slug_geometry(&data, viewport(), &mut system, &mut glyphs, &mut masks, &text_arena);
	let output = build_ui_primitives(&data, viewport(), &arena, None, &mut masks, Some(&text), None, None);
	report_primitives("mixed", count, &arena, output.primitives.len(), output.steps.len());
	drop(output);
	bencher.bench_local(|| {
		arena.reset();
		black_box(build_ui_primitives(
			black_box(&data),
			viewport(),
			&arena,
			None,
			&mut masks,
			Some(&text),
			None,
			None,
		));
	});
}

#[divan::bench(args = [100, 1000])]
/// Copies generated primitives into a CPU buffer sized before measurement.
fn cpu_buffer_copy(bencher: Bencher, count: usize) {
	let data = draw_list(count, Scene::Mixed);
	let arena = bumpalo::Bump::new();
	let mut masks = UiMaskTable::default();
	let mut system = TextSystem::new();
	let mut atlas = UiGlyphAtlas::new(UI_GLYPH_ATLAS_INITIAL_SIZE);
	let text = build_ui_text_geometry(&data, viewport(), &mut system, &mut atlas, &mut masks, &arena);
	let output = build_ui_primitives(&data, viewport(), &arena, None, &mut masks, Some(&text), None, None);
	let source: &[u8] = bytemuck::cast_slice(&output.primitives);
	let mut destination = vec![0; source.len()];
	bencher.bench_local(|| {
		destination.copy_from_slice(black_box(source));
		black_box(&destination);
	});
}

#[divan::bench]
/// Measures only the prepared-frame cache-key comparison, excluding pipeline checks.
fn unchanged_revision(bencher: Bencher) {
	let render = scene(100, Scene::Mixed, false);
	let frame = UiPreparedFrame {
		revision: Some(render.revision()),
		extent: viewport(),
		glyph_generation: 0,
		damage: vec![UiPixelRegion::full(viewport())],
		steps: Vec::new(),
	};
	bencher.bench_local(|| {
		black_box(frame.matches(
			black_box(Some(render.revision())),
			black_box(viewport()),
			black_box(0),
			black_box(&frame.damage),
		))
	});
}

/// The `CpuFrame` struct retains the same CPU caches between changed-frame rebuilds.
struct CpuFrame {
	caches: UiGeometryCaches,
	masks: UiMaskTable,
	data: UiDrawList,
	system: TextSystem,
	atlas: UiGlyphAtlas,
	arena: bumpalo::Bump,
	staging: Vec<u8>,
}

impl CpuFrame {
	/// Exercises the real builders together, then copies their output into CPU staging storage.
	fn rebuild(&mut self, render: &engine::Render) {
		self.arena.reset();
		self.staging.clear();
		self.masks.clear();
		update_from_render(render, &mut self.data);
		let text = build_ui_text_geometry_damaged(
			&self.data,
			viewport(),
			&mut self.system,
			&mut self.atlas,
			&mut self.masks,
			&self.arena,
			None,
		);
		let output = build_ui_primitives(
			&self.data,
			viewport(),
			&self.arena,
			Some(&mut self.caches),
			&mut self.masks,
			Some(&text),
			None,
			None,
		);
		assert!(!output.truncated && output.dropped_glyphs == 0);
		self.staging.extend_from_slice(bytemuck::cast_slice(&output.primitives));
		self.staging.extend_from_slice(bytemuck::cast_slice(self.masks.entries()));
		black_box((&self.staging, &output.steps));
	}
}

#[divan::bench(args = [100, 1000])]
/// Combines adoption, warm primitive building, and CPU staging copies for changed frames.
fn changed_mixed_frame(bencher: Bencher, count: usize) {
	let renders = [scene(count, Scene::Mixed, false), scene(count, Scene::Mixed, true)];
	let mut frame = CpuFrame {
		caches: UiGeometryCaches::default(),
		masks: UiMaskTable::default(),
		data: UiDrawList::default(),
		system: TextSystem::new(),
		atlas: UiGlyphAtlas::new(UI_GLYPH_ATLAS_INITIAL_SIZE),
		arena: bumpalo::Bump::new(),
		staging: Vec::new(),
	};
	for render in &renders {
		frame.rebuild(render);
	}
	let mut index = 0;
	bencher.bench_local(|| {
		index ^= 1;
		frame.rebuild(black_box(&renders[index]));
	});
}
