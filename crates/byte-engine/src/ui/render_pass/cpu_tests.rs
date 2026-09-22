//! CPU preparation contracts across changing content, clipping, and painter order.

use super::*;
use crate::ui::{
	ConcreteLayer, ConcreteStyle, Container, ContainerContext, Context, Curve, CurvePath, CurveSegment, ElementContext, Engine,
	Size, Text, Transform, UiPoint, flow,
};

/// Varies content, visibility, and stroke layers between snapshots.
fn changing_render(count: usize, phase: usize) -> engine::Render {
	let mut engine = Engine::new();
	engine.mount(move |ctx| {
		std::boxed::Box::pin(async move {
			let mut root = ctx.element("root").container(Container::default());
			for index in 0..count {
				let color = RGBA::new(0.2, 0.4, 0.6, if (index + phase).is_multiple_of(3) { 0.0 } else { 1.0 });
				root.element("label").text(
					Text::new(format!("{phase}: {}", "label".repeat(index + 1)))
						.font_size(10.0 + phase as f32)
						.style(ConcreteLayer::default().color(color.into())),
				);
				root.element("curve").curve(
					Curve::new(CurvePath::new(40.into(), 30.into()).line((0.0, phase as f32), (30.0, 20.0))).style(
						ConcreteStyle::from_layers([
							ConcreteLayer::default().color(color.into()).stroke(1.0 + phase as f32),
							ConcreteLayer::default()
								.color(color.into())
								.stroke(if phase.is_multiple_of(2) { 2.0 } else { 0.0 }),
						]),
					),
				);
			}
		})
	});
	let arena = bumpalo::Bump::new();
	let mut snapshot = engine.evaluate(Size::new(800, 800), &arena);
	engine.render(&mut snapshot).clone()
}

/// Builds a frame's primitives without caches or text.
fn primitives<'a>(data: &UiDrawList, viewport: Extent, arena: &'a bumpalo::Bump) -> UiPrimitives<'a> {
	build_ui_primitives_uncached(data, viewport, arena, &mut UiMaskTable::default())
}

/// Repeated adoption must produce the same text and curve output as a fresh draw list.
#[test]
fn adoption_handles_changed_filtered_removed_and_regrown_entries() {
	let mut reused = UiDrawList::default();
	for (count, phase) in [(6, 0), (2, 1), (5, 2), (0, 3), (4, 4)] {
		let render = changing_render(count, phase);
		let mut fresh = UiDrawList::default();
		update_from_render(&render, &mut fresh);
		update_from_render(&render, &mut reused);
		assert_eq!(reused.texts, fresh.texts);
		let arena = bumpalo::Bump::new();
		let expected = primitives(&fresh, Extent::square(800), &arena);
		let actual = primitives(&reused, Extent::square(800), &arena);
		assert_eq!(actual.primitives, expected.primitives);
		assert_eq!(actual.steps, expected.steps);
	}
}

/// Draw order must remain stable when different element types share a depth and order.
#[test]
fn primitives_preserve_painter_order_for_equal_keys() {
	let rect = |depth, order, color: f32| UiDrawElement {
		depth,
		order,
		position: [0.0, 0.0],
		size: [10.0, 10.0],
		clip: None,
		clip_mask: None,
		paint: UiPaint::flat([color, 0.0, 0.0, 1.0]),
		corner_radius: 0.0,
		corner_exponent: 2.0,
		sector: None,
		layer_kind: LayerKind::Fill,
		stroke_width: 0.0,
	};
	let curve = |depth, order, color: f32| UiCurveDrawElement {
		depth,
		order,
		position: [0.0, 0.0],
		size: [10.0, 10.0],
		clip: None,
		clip_mask: None,
		paint: UiPaint::flat([color, 0.0, 0.0, 1.0]),
		stroke_width: 1.0,
		segments: vec![CurveSegment::Line {
			from: (0.0, 0.0).into(),
			to: (10.0, 10.0).into(),
		}],
	};
	// Each list arrives sorted by depth; the colors name the expected position.
	let data = UiDrawList {
		layout_size: [100.0, 100.0],
		elements: vec![rect(1, 2, 0.0), rect(1, 3, 1.0), rect(2, 4, 4.0)],
		curves: vec![curve(1, 3, 2.0), curve(2, 0, 3.0)],
		..UiDrawList::default()
	};
	let arena = bumpalo::Bump::new();
	let output = primitives(&data, Extent::square(100), &arena);
	let order: Vec<_> = output.primitives[1..].iter().map(|primitive| primitive.color[0]).collect();
	assert_eq!(order, [0.0, 1.0, 2.0, 3.0, 4.0]);
	assert_eq!(output.steps.as_slice(), [UiStep::Draw { first: 1, count: 5 }]);
}

/// Culled images must not shift the source used by a visible primitive.
#[test]
fn image_primitives_resolve_visible_sources_and_versions() {
	let images =
		[(42, 0, 0.0, 10), (7, 2, 1.0, 30), (8, 1, 1.0, 40), (7, 1, 1.0, 50)].map(|(image_id, version, opacity, value)| {
			UiImageDrawElement {
				depth: 0,
				order: 0,
				image_id,
				version,
				opacity,
				source_width: 1,
				source_height: 1,
				pixels: vec![value; 4].into(),
				position: [0.0, 0.0],
				size: [10.0, 10.0],
				clip: None,
				clip_mask: None,
			}
		});
	let mut clipped = images[1].clone();
	clipped.image_id = 99;
	clipped.clip = Some(DrawClip {
		position: [20.0, 20.0],
		size: [10.0, 10.0],
	});
	let mut data = UiDrawList {
		layout_size: [100.0, 100.0],
		images: images.into(),
		..UiDrawList::default()
	};
	data.images.insert(2, clipped);
	let arena = bumpalo::Bump::new();
	let output = primitives(&data, Extent::square(100), &arena);
	let sources: Vec<_> = output
		.images
		.iter()
		.map(|&(primitive, source)| {
			assert_eq!(output.primitives[primitive as usize].kind, UI_KIND_IMAGE);
			let image = &data.images[source as usize];
			(image.image_id, image.version, image.pixels[0])
		})
		.collect();
	assert_eq!(sources, [(7, 2, 30), (8, 1, 40), (7, 1, 50)]);
}

/// Repeated and alternating radii must match independent blur preparation at each scale.
#[test]
fn blur_kernels_match_independent_items_after_radius_and_scale_changes() {
	let data = UiDrawList {
		layout_size: [100.0, 100.0],
		blurs: [8.0, 8.0, 16.0, 8.0, 4.0, 4.0]
			.map(|radius| UiBlurDrawElement {
				depth: 0,
				order: 0,
				position: [10.0, 10.0],
				size: [20.0, 20.0],
				clip: None,
				clip_mask: None,
				color: [1.0; 4],
				corner_radius: 4.0,
				corner_exponent: 2.0,
				sector: None,
				radius,
				path: None,
			})
			.into(),
		..UiDrawList::default()
	};
	for viewport in [Extent::square(100), Extent::square(200)] {
		let arena = bumpalo::Bump::new();
		let blurs = |data: &UiDrawList| -> Vec<UiBlurDispatch> {
			primitives(data, viewport, &arena)
				.steps
				.iter()
				.filter_map(|step| match step {
					UiStep::Blur(blur) => Some(*blur),
					UiStep::Draw { .. } => None,
				})
				.collect()
		};
		let combined = blurs(&data);
		assert_eq!(combined.len(), data.blurs.len());
		for (blur, actual) in data.blurs.iter().zip(&combined) {
			let single = UiDrawList {
				layout_size: data.layout_size,
				blurs: vec![blur.clone()],
				..UiDrawList::default()
			};
			let expected = blurs(&single);
			assert_eq!(actual.full_kernel, expected[0].full_kernel);
			assert_eq!(actual.half_kernel, expected[0].half_kernel);
			assert_eq!(actual.resolution_mix, expected[0].resolution_mix);
		}
	}
}

/// A zoomed subtree must scale its wires and labels the way its rectangles are scaled.
#[test]
fn adoption_applies_inherited_scale_to_curve_points_stroke_and_font_size() {
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		std::boxed::Box::pin(async move {
			let mut canvas = ctx.element("canvas").container(
				Container::default()
					.width(100.into())
					.height(100.into())
					.flow(flow::center)
					.transform(Transform::identity().origin(UiPoint::zero()).scale(2.0)),
			);
			canvas.element("wire").curve(
				Curve::new(CurvePath::new(100.into(), 100.into()).cubic((0.0, 0.0), (5.0, 0.0), (5.0, 10.0), (10.0, 10.0)))
					.style(ConcreteLayer::default().color(RGBA::white().into()).stroke(1.5)),
			);
			canvas.element("label").text(Text::new("node").font_size(10.0));
		})
	});
	let arena = bumpalo::Bump::new();
	let mut snapshot = engine.evaluate(Size::new(400, 400), &arena);
	let render = engine.render(&mut snapshot).clone();
	let mut draw_list = UiDrawList::default();
	update_from_render(&render, &mut draw_list);

	let wire = &draw_list.curves[0];
	assert_eq!(wire.stroke_width, 3.0);
	let [
		CurveSegment::Cubic {
			from,
			control0,
			control1,
			to,
		},
	] = wire.segments.as_slice()
	else {
		panic!("expected the scaled cubic segment");
	};
	assert_eq!((from.x, from.y), (0.0, 0.0));
	assert_eq!((control0.x, control0.y), (10.0, 0.0));
	assert_eq!((control1.x, control1.y), (10.0, 20.0));
	assert_eq!((to.x, to.y), (20.0, 20.0));
	assert_eq!(draw_list.texts[0].font_size, 20.0);
}

/// Mirrors the graph demo: a wire declared empty and routed on a later frame inside a clipped, transformed canvas.
#[test]
fn a_wire_routed_after_its_first_frame_reaches_the_draw_list() {
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		std::boxed::Box::pin(async move {
			let mut root = ctx.element("root").container(Container::default().hit_testable(false));
			let mut viewport = root.element("viewport").container(
				Container::default()
					.absolute_position(100, 100)
					.width(400.into())
					.height(300.into()),
			);
			let mut content = viewport.element("content").container(
				Container::default()
					.absolute_position(0, 0)
					.width(400.into())
					.height(300.into())
					.hit_testable(false)
					.clip(false)
					.transform(Transform::identity().origin(UiPoint::zero())),
			);
			let mut wires = content.element("wires").container(
				Container::default()
					.absolute_position(0, 0)
					.width(400.into())
					.height(300.into())
					.hit_testable(false)
					.clip(false),
			);
			wires.element("wire-1").component(|ctx| {
				std::boxed::Box::pin(async move {
					let mut curve = ctx.element("curve").curve(
						Curve::new(CurvePath::new(400.into(), 300.into()))
							.style(ConcreteLayer::default().color(RGBA::white().into()).stroke(3.0))
							.hit_testable(12.0),
					);
					let mut routed = false;
					loop {
						crate::utils::r#async::select! {
							_ = curve.on(crate::ui::Events::PointerEntered) => {},
							_ = curve.on(crate::ui::Events::Actuated) => {},
							_ = ctx.render() => {},
						}
						if !routed {
							routed = true;
							curve.update_curve(|curve| {
								let path = curve.path_mut();
								path.clear();
								path.push_cubic((20.0, 20.0), (80.0, 20.0), (120.0, 200.0), (200.0, 200.0));
							});
						}
					}
				})
			});
		})
	});
	let arena = bumpalo::Bump::new();
	let mut draw_list = UiDrawList::default();
	for _ in 0..3 {
		let mut snapshot = engine.evaluate(Size::new(800, 600), &arena);
		let render = engine.render(&mut snapshot).clone();
		update_from_render(&render, &mut draw_list);
	}
	assert_eq!(draw_list.curves.len(), 1);
	assert_eq!(draw_list.curves[0].segments.len(), 1);
	let output = primitives(&draw_list, Extent::square(800), &arena);
	assert!(
		output.primitives.iter().any(|primitive| primitive.kind == UI_KIND_CURVE),
		"the routed wire produced no primitives"
	);
}

/// Surviving layers must not retain old colors, clips, or curve paths after neighboring removals.
#[test]
fn cached_surface_primitives_follow_content_and_layer_edits() {
	let mut caches = UiGeometryCaches::default();
	let mut data = UiDrawList::default();
	let mut arena = bumpalo::Bump::new();
	for (count, phase) in [(6, 0), (6, 0), (2, 1), (5, 2), (0, 3), (4, 4)] {
		arena.reset();
		update_from_render(&changing_render(count, phase), &mut data);
		let actual = build_ui_primitives(
			&data,
			Extent::square(800),
			&arena,
			Some(&mut caches),
			&mut UiMaskTable::default(),
			None,
			None,
			None,
		);
		let expected = primitives(&data, Extent::square(800), &arena);
		assert_eq!(actual.primitives, expected.primitives);
		assert_eq!(actual.steps, expected.steps);
	}
}

/// Scaling the root must change its surfaces without changing the viewport's layout units.
#[test]
fn root_transform_preserves_viewport_units() {
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		std::boxed::Box::pin(async move {
			let mut root = ctx.element("root").container(
				Container::default()
					.style(ConcreteStyle::new())
					.transform(Transform::identity().origin(UiPoint::zero()).translate(10., 20.).scale(2.)),
			);
			root.element("child").container(Container::default().size(10.into()));
		})
	});
	let arena = bumpalo::Bump::new();
	let mut snapshot = engine.evaluate(Size::new(100, 100), &arena);
	let mut data = UiDrawList::default();
	update_from_render(engine.render(&mut snapshot), &mut data);
	let output = primitives(&data, Extent::square(100), &arena);
	assert_eq!(output.primitives.len(), 2);
	assert_eq!(output.primitives[1].bounds, [10., 20., 30., 40.]);
}

/// A glass icon is three stacked paths; the glass blurs its backdrop under its outline.
/// The blur must merge before the glass fill and the highlight after both, as the tree orders them.
#[test]
fn path_blur_from_a_real_tree_merges_under_its_fill_and_before_later_siblings() {
	use crate::ui::Path;

	let frame_allocator = bumpalo::Bump::new();
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		std::boxed::Box::pin(async move {
			let mut frame = ctx.element("frame").container(
				Container::default()
					.width(40.into())
					.height(40.into())
					.clip(false)
					.flow(crate::ui::flow::center),
			);
			let square = || {
				CurvePath::new(40.into(), 40.into())
					.line((0.0, 0.0), (20.0, 0.0))
					.line((20.0, 0.0), (20.0, 20.0))
					.line((20.0, 20.0), (0.0, 20.0))
			};
			frame.element("body").path(Path::new(square()));
			frame.element("glass").path(
				Path::new(square()).style(
					ConcreteStyle::new()
						.layer(ConcreteLayer::default().backdrop_blur(8.0))
						.layer(ConcreteLayer::default()),
				),
			);
			frame.element("highlight").path(Path::new(square()));
			// A later rectangle blur lands in the blur list first, since rectangles are walked before paths.
			frame.element("pill").container(
				Container::default()
					.width(10.into())
					.height(10.into())
					.style(ConcreteLayer::default().backdrop_blur(4.0)),
			);
		})
	});
	let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
	let render = engine.render(&mut snapshot);
	let mut data = UiDrawList::default();
	update_from_render(&render, &mut data);

	assert_eq!(data.blurs.len(), 2, "the glass blur layer and the pill's blur");
	assert!(
		data.blurs[0].path.is_some(),
		"blurs are in painter order, so the glass comes first"
	);
	assert_eq!(data.paths.len(), 3, "body, glass tint, and highlight fills");
	let keys: Vec<(u32, u32)> = data.paths.iter().map(|path| (path.depth, path.order)).collect();
	assert_eq!(
		(data.blurs[0].depth, data.blurs[0].order),
		keys[1],
		"the blur shares the glass fill's key"
	);
	assert!(keys[0] < keys[1] && keys[1] < keys[2], "paths keep tree order: {keys:?}");

	let mut masks = UiMaskTable::default();
	let mut curves = UiPathCurves::new(UI_PATH_CURVE_CAPACITY, UI_PATH_BAND_CAPACITY);
	let paths = build_ui_path_geometry_damaged(&data, Extent::square(100), &mut curves, &mut masks, &frame_allocator, None);
	let output = build_ui_primitives(
		&data,
		Extent::square(100),
		&frame_allocator,
		None,
		&mut masks,
		None,
		Some(&paths),
		None,
	);
	let kinds: Vec<u32> = output.primitives[1..].iter().map(|primitive| primitive.kind).collect();
	assert_eq!(
		kinds,
		[
			UI_KIND_RECT,
			UI_KIND_PATH,
			UI_KIND_PATH_BLUR,
			UI_KIND_PATH,
			UI_KIND_PATH,
			UI_KIND_BLUR
		]
	);
}
