//! A camera-controlled canvas beside stationary labels and images.

use std::cell::Cell;

use super::*;
use crate::ui::{
	ConcreteLayer, ConcreteStyle, Container, ContainerContext, Context, Curve, CurvePath, ElementContext, Engine, Image, Size,
	Text, Transform,
};

/// Builds a graph-like workload with an explicit content-transform boundary.
pub(super) fn graph_engine(count: usize) -> Engine<Cell<(f32, f32)>> {
	let mut engine = Engine::with_context(Cell::new((0.0, 1.0)));
	engine.mount(move |ctx| {
		std::boxed::Box::pin(async move {
			let mut root = ctx
				.element("root")
				.container(Container::default().style(ConcreteStyle::new()));
			let mut viewport = root.element("viewport").container(
				Container::default()
					.width(900.into())
					.height(900.into())
					.style(ConcreteStyle::new()),
			);
			let mut content = viewport.element("content").container(
				Container::default()
					.width(900.into())
					.height(900.into())
					.clip(false)
					.style(ConcreteStyle::new()),
			);
			for index in 0..count {
				let mut node = content.element("node").container(
					Container::default()
						.absolute_position((index % 8 * 100) as i32, (index / 8 * 90) as i32)
						.width(90.into())
						.height(65.into()),
				);
				node.element("label").text(Text::new(format!("Node {index}")).font_size(12.));
				node.element("wire").curve(
					Curve::new(CurvePath::new(80.into(), 30.into()).cubic((0., 0.), (25., 30.), (50., 0.), (80., 30.)))
						.style(ConcreteLayer::default().stroke(2.))
						.hit_testable(6.),
				);
			}
			for index in 0..32 {
				let mut card = root.element("toolbar").container(
					Container::default()
						.absolute_position(1000 + (index % 4 * 180), index / 4 * 100)
						.width(170.into())
						.height(90.into()),
				);
				card.element("label").text(Text::new(format!("Stationary card {index}")));
				card.element("image").image(Image::from_rgba(2, 2, vec![255; 16]));
			}
			let mut applied = (0.0, 1.0);
			loop {
				let camera = ctx.ctx().get();
				if camera != applied {
					content.update_container(|value| {
						value.set_transform(Transform::identity().translate(camera.0, 13.25).scale(camera.1))
					});
					applied = camera;
				}
				ctx.render().await;
			}
		})
	});
	engine
}

/// Cached surfaces must produce the same submitted vertices and batches as fresh preparation.
#[test]
fn camera_geometry_matches_fresh_preparation() {
	let mut engine = graph_engine(64);
	let mut rectangles = SurfaceCache::default();
	let mut images = ImageGeometryCache::default();
	let mut curves = CurveGeometryCache::default();
	let mut text = TextSystem::new();
	let mut atlas = UiGlyphAtlas::new(64);
	let mut data = UiDrawList::default();
	let mut arena = bumpalo::Bump::new();
	for (pan, scale, extent) in [
		(0., 1., 1920),
		(12.25, 1., 1920),
		(-130., 1.5, 1920),
		(1500., 0.75, 1920),
		(0., 1., 1920),
		(-12.5, 1.25, 1440),
		(0., 1., 1920),
	] {
		arena.reset();
		engine.ctx().set((pan, scale));
		let mut snapshot = engine.evaluate(Size::new(1920, 1080), &arena);
		update_from_render(engine.render(&mut snapshot), &mut data);
		let extent = Extent::rectangle(extent, 1080);
		macro_rules! same {
			($actual:expr, $expected:expr) => {{
				let actual = $actual;
				let expected = $expected;
				assert_eq!(
					bytemuck::cast_slice::<_, u8>(&actual.vertices),
					bytemuck::cast_slice::<_, u8>(&expected.vertices)
				);
				assert_eq!(actual.indices, expected.indices);
				assert_eq!(actual.batches, expected.batches);
				assert_eq!(actual.truncated, expected.truncated);
			}};
		}
		same!(
			build_ui_geometry_cached(&data, extent, &arena, Some(&mut rectangles)),
			build_ui_geometry(&data, extent, &arena)
		);
		same!(
			build_ui_curve_geometry_cached(&data, extent, &arena, Some(&mut curves)),
			build_ui_curve_geometry(&data, extent, &arena)
		);
		same!(
			build_ui_image_geometry_cached(&data, extent, &arena, Some(&mut images)),
			build_ui_image_geometry(&data, extent, &arena)
		);
		// Preserve the atlas packing while clearing only the prepared-run cache.
		let actual = build_ui_text_geometry(&data, extent, &mut text, &mut atlas, &arena);
		atlas.clear_prepared_runs();
		let expected = build_ui_text_geometry(&data, extent, &mut text, &mut atlas, &arena);
		assert_eq!(
			bytemuck::cast_slice::<_, u8>(&actual.vertices),
			bytemuck::cast_slice::<_, u8>(&expected.vertices)
		);
		assert_eq!(actual.indices, expected.indices);
		assert_eq!(actual.batches, expected.batches);
	}
}

/// Measures camera evaluation separately from CPU geometry preparation.
#[cfg(feature = "ui-render-bench")]
#[divan::bench(args = [false, true])]
fn graph_camera_evaluate_render(bencher: divan::Bencher, zoom: bool) {
	let mut engine = graph_engine(64);
	let mut arena = bumpalo::Bump::new();
	let mut frame = 0;
	bencher.bench_local(|| {
		arena.reset();
		frame += 1;
		engine.ctx().set((
			if frame % 2 == 0 { 12.25 } else { -12.25 },
			if zoom && frame % 2 == 0 { 1.125 } else { 1.0 },
		));
		let mut snapshot = engine.evaluate(Size::new(1920, 1080), &arena);
		divan::black_box(engine.render(&mut snapshot).revision());
	});
}

/// Measures the production caches on alternating camera frames, including adoption.
#[cfg(feature = "ui-render-bench")]
#[divan::bench(args = [false, true])]
fn graph_camera_prepare(bencher: divan::Bencher, zoom: bool) {
	let mut engine = graph_engine(64);
	let mut arena = bumpalo::Bump::new();
	let frames = [(-12.25, 1.0), (12.25, if zoom { 1.125 } else { 1.0 })].map(|camera| {
		engine.ctx().set(camera);
		let mut snapshot = engine.evaluate(Size::new(1920, 1080), &arena);
		engine.render(&mut snapshot).clone()
	});
	let mut data = UiDrawList::default();
	let mut rectangles = SurfaceCache::default();
	let mut images = ImageGeometryCache::default();
	let mut curves = CurveGeometryCache::default();
	let mut text = TextSystem::new();
	let mut atlas = UiGlyphAtlas::new(UI_GLYPH_ATLAS_INITIAL_SIZE);
	let mut frame = 0;
	let extent = Extent::rectangle(1920, 1080);
	// Warm both glyph sizes and all retained buffers outside the measurement.
	for _ in 0..4 {
		update_from_render(&frames[frame % 2], &mut data);
		build_ui_text_geometry(&data, extent, &mut text, &mut atlas, &arena);
		frame += 1;
	}
	bencher.bench_local(|| {
		arena.reset();
		update_from_render(&frames[frame % 2], &mut data);
		divan::black_box(build_ui_geometry_cached(&data, extent, &arena, Some(&mut rectangles)));
		divan::black_box(build_ui_curve_geometry_cached(&data, extent, &arena, Some(&mut curves)));
		divan::black_box(build_ui_image_geometry_cached(&data, extent, &arena, Some(&mut images)));
		divan::black_box(build_ui_text_geometry(&data, extent, &mut text, &mut atlas, &arena));
		frame += 1;
	});
}
