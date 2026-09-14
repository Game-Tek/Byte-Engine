//! Observable comparisons between retained updates and fresh scene construction.

use super::*;
use crate::ui::{
	ContainerContext, Curve, CurvePath, Position, Sizing, flow,
	style::{ConcreteLayer, ConcreteStyle, Layer},
};

/// Builds a scene by applying the requested changes before its next frame.
fn scene(stage: usize) -> Engine<std::cell::Cell<usize>> {
	let mut engine = Engine::with_context(std::cell::Cell::new(stage));
	engine.mount(|ctx| {
		Box::pin(async move {
			let mut root = ctx
				.element("root")
				.container(Container::default().flow(flow::row).clip(false));
			let mut panel = root
				.element("panel")
				.container(Container::default().width(40.into()).height(40.into()).flow(flow::column));
			let mut label = panel.element("label").text(Text::new("Small"));
			let mut sibling = root
				.element("sibling")
				.container(Container::default().width(20.into()).height(20.into()));
			let mut applied = 0;
			loop {
				let target = ctx.ctx().get();
				for stage in applied + 1..=target {
					match stage {
						1 => {
							panel.update_container(|value| {
								value.set_style(ConcreteLayer::default().color(RGBA::new(1.0, 0.0, 0.0, 1.0).into()))
							});
						}
						2 => {
							panel.update_container(|value| value.set_opacity(0.5));
						}
						3 => {
							label.update_text(|value| *value = Text::new("A much wider label").font_size(24.0));
						}
						4 => {
							panel.update_container(|value| value.width = Sizing::pixels(80));
						}
						5 => {
							panel.update_container(|value| {
								value.set_transform(Transform::identity().translate(8.0, 6.0).scale(0.75))
							});
						}
						6 => {
							panel.update_container(|value| value.set_clip(false));
						}
						7 => {
							panel.update_container(|value| {
								value.set_clip(true);
								value.corner_radius = 8.0;
								value.set_style(
									ConcreteStyle::new()
										.layer(ConcreteLayer::default())
										.layer(ConcreteLayer::default().feather(EdgeFeather::all(5.0))),
								);
							});
						}
						8 => {
							panel.update_container(|value| value.depth = Depth::Absolute(2));
						}
						9 => {
							assert!(label.reparent(sibling.id()));
						}
						10 => {
							sibling.update_container(|value| {
								value.width = Sizing::Relative(1, 2);
								value.flow =
									utils::InlineCopyFn::<fn(crate::ui::flow::FlowInput) -> crate::ui::flow::FlowOutput>::new(
										flow::column,
									);
							});
						}
						11 => {
							label.update_text(|value| value.set_opacity(0.25));
						}
						_ => unreachable!(),
					}
				}
				applied = target;
				ctx.render().await;
			}
		})
	});
	engine
}

/// Compares all layer properties consumed by the renderer.
fn assert_same_style(actual: &ConcreteStyle, expected: &ConcreteStyle) {
	assert_eq!(actual.layers().len(), expected.layers().len());
	for (actual, expected) in actual.layers().iter().zip(expected.layers()) {
		match (actual.fill(), expected.fill()) {
			(Color::Value(actual), Color::Value(expected)) => assert_eq!(actual, expected),
			(Color::Sample(actual), Color::Sample(expected)) => assert_eq!(actual, expected),
			_ => panic!("Render layer colors differ"),
		}
		assert_eq!(actual.kind(), expected.kind());
		assert_eq!(actual.feather(), expected.feather());
		assert_eq!(actual.backdrop_blur_radius(), expected.backdrop_blur_radius());
	}
}

/// Compares the geometry, inherited appearance, and content consumed by the renderer.
fn assert_same_render(actual: &Render, expected: &Render) {
	assert_eq!(actual.size(), expected.size());
	assert_eq!(actual.elements().count(), expected.elements().count());
	for (actual, expected) in actual.elements().zip(expected.elements()) {
		assert_eq!(
			(
				actual.id,
				actual.position,
				actual.size,
				actual.clip,
				actual.feather_mask,
				actual.opacity,
				actual.corner_radius,
				actual.corner_exponent
			),
			(
				expected.id,
				expected.position,
				expected.size,
				expected.clip,
				expected.feather_mask,
				expected.opacity,
				expected.corner_radius,
				expected.corner_exponent
			),
		);
		assert_eq!(actual.backdrop_blur_radius, expected.backdrop_blur_radius);
		assert_same_style(&actual.style, &expected.style);
	}
	assert_eq!(actual.texts().count(), expected.texts().count());
	for (actual, expected) in actual.texts().zip(expected.texts()) {
		assert_eq!(
			(
				actual.id,
				actual.position,
				actual.size,
				actual.clip,
				actual.feather_mask,
				actual.opacity,
				actual.font_size,
				&actual.content
			),
			(
				expected.id,
				expected.position,
				expected.size,
				expected.clip,
				expected.feather_mask,
				expected.opacity,
				expected.font_size,
				&expected.content
			),
		);
		assert_eq!(actual.color, expected.color);
	}
	assert_eq!(actual.curves().count(), expected.curves().count());
	for (actual, expected) in actual.curves().zip(expected.curves()) {
		assert_eq!(
			(
				actual.id,
				actual.position,
				actual.size,
				actual.clip,
				actual.feather_mask,
				actual.opacity,
				&actual.segments
			),
			(
				expected.id,
				expected.position,
				expected.size,
				expected.clip,
				expected.feather_mask,
				expected.opacity,
				&expected.segments
			),
		);
		assert_same_style(&actual.style, &expected.style);
	}
}

/// Replaces a scope with different text, layers, paths, depths, and element counts.
fn changing_content_scene(stage: usize) -> Engine<std::cell::Cell<usize>> {
	let mut engine = Engine::with_context(std::cell::Cell::new(stage));
	engine.mount(|ctx| {
		Box::pin(async move {
			let mut root = ctx.element("root").container(Container::default().clip(false));
			loop {
				root.element("screen")
					.mount(|ctx| {
						Box::pin(async move {
							let stage = ctx.ctx().get();
							let mut root = ctx
								.element("root")
								.container(Container::default().clip(false).flow(flow::row));
							for index in 0..[3, 3, 1, 0, 4, 2][stage] {
								let style = ConcreteStyle::from_layers((0..[1, 3, 0, 1, 2, 1][stage]).map(|layer| {
									ConcreteLayer::default()
										.color(RGBA::new(stage as f32 / 6.0, layer as f32 / 3.0, 0.5, 1.0).into())
										.stroke(layer as f32 + 1.0)
										.feather(EdgeFeather::all(stage as f32))
								}));
								let mut panel = root.element(["a", "b", "c", "d"][index]).container(
									Container::default()
										.size((30 + stage as u32).into())
										.clip(false)
										.flow(flow::column)
										.depth(Depth::Absolute((index as i32 + stage as i32) % 3))
										.corner_radius(stage as f32)
										.opacity(1.0 - stage as f32 / 10.0)
										.style(style.clone()),
								);
								panel.element("text").text(
									Text::new(
										["Long initial content", "X", "Longer replacement text", "", "Grow again", "Y"][stage],
									)
									.font_size(10.0 + stage as f32)
									.style(style.clone()),
								);
								let mut path = CurvePath::new(20.into(), 20.into());
								for segment in 0..[3, 1, 4, 0, 2, 1][stage] {
									path = path.line((0.0, segment as f32), (10.0 + stage as f32, 10.0));
								}
								panel.element("curve").curve(Curve::new(path).style(style));
							}
							while ctx.ctx().get() == stage {
								ctx.render().await;
							}
						})
					})
					.await;
			}
		})
	});
	engine
}

#[test]
fn rebuilt_render_matches_fresh_content_and_preserves_older_clones() {
	let mut engine = changing_content_scene(0);
	let mut allocator = bumpalo::Bump::new();
	let mut original = None;
	for stage in 0..6 {
		allocator.reset();
		engine.ctx().set(stage);
		let mut fresh = changing_content_scene(stage);
		let mut actual = engine.evaluate(Size::new(200, 200), &allocator);
		let mut expected = fresh.evaluate(Size::new(200, 200), &allocator);
		let actual = engine.render(&mut actual);
		let expected = fresh.render(&mut expected);
		assert_same_render(actual, expected);
		let (original, expected_original) = original.get_or_insert_with(|| (actual.clone(), expected.clone()));
		assert_same_render(original, expected_original);
	}
}

#[test]
fn retained_changes_match_fresh_layout_appearance_and_hits() {
	let mut engine = scene(0);
	let mut allocator = bumpalo::Bump::new();
	for stage in 0..=11 {
		allocator.reset();
		engine.ctx().set(stage);
		let mut fresh = scene(stage);
		let size = if stage == 10 {
			Size::new(160, 120)
		} else {
			Size::new(100, 100)
		};
		let mut actual = engine.evaluate(size, &allocator);
		let mut expected = fresh.evaluate(size, &allocator);
		assert_same_render(engine.render(&mut actual), fresh.render(&mut expected));
		let mut actual_hits = crate::ui::intersection::HitTest::default();
		let mut expected_hits = crate::ui::intersection::HitTest::default();
		actual.retain_hit_test(&mut actual_hits);
		expected.retain_hit_test(&mut expected_hits);
		for x in [0.0, 10.0, 30.0, 50.0, 90.0] {
			for y in [0.0, 20.0, 80.0] {
				let point = UiPoint::new(x * 2.0 / size.x() - 1.0, 1.0 - y * 2.0 / size.y());
				assert_eq!(actual_hits.query(point), expected_hits.query(point), "stage {stage}");
			}
		}
	}
}

#[test]
fn flow_replacements_and_gap_changes_update_layout_after_paint_changes() {
	thread_local! { static OFFSET: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) }; }
	fn stateful(input: crate::ui::FlowInput) -> crate::ui::FlowOutput {
		crate::ui::FlowOutput::new(crate::ui::Offset::new(OFFSET.with(std::cell::Cell::get), 0.0), input.cursor())
	}
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		Box::pin(async move {
			let mut root = ctx
				.element("root")
				.container(Container::default().flow(flow::row_with_gap(3)));
			let mut first = root
				.element("a")
				.container(Container::default().width(20.into()).height(10.into()));
			root.element("b")
				.container(Container::default().width(20.into()).height(10.into()));
			ctx.render().await;
			root.update_container(|value| value.set_opacity(0.5));
			ctx.render().await;
			root.update_container(|value| {
				value.flow =
					utils::InlineCopyFn::<fn(crate::ui::FlowInput) -> crate::ui::FlowOutput>::new(flow::row_with_gap(7))
			});
			ctx.render().await;
			root.update_container(|value| value.set_opacity(0.25));
			ctx.render().await;
			root.update_container(|value| {
				value.flow =
					utils::InlineCopyFn::<fn(crate::ui::FlowInput) -> crate::ui::FlowOutput>::new(flow::column_with_gap(4))
			});
			ctx.render().await;
			root.update_container(|value| {
				value.flow = utils::InlineCopyFn::<fn(crate::ui::FlowInput) -> crate::ui::FlowOutput>::new(flow::center)
			});
			ctx.render().await;
			// Replacing a built-in through the public field must enable the custom-flow fallback.
			root.update_container(|value| {
				value.flow =
					utils::InlineCopyFn::<fn(crate::ui::FlowInput) -> crate::ui::FlowOutput>::new(stateful as fn(_) -> _)
			});
			ctx.render().await;
			OFFSET.with(|offset| offset.set(13.0));
			first.update_container(|value| value.set_opacity(0.5));
		})
	});
	let mut allocator = bumpalo::Bump::new();
	for (x, y) in [
		(23.0, 0.0),
		(23.0, 0.0),
		(27.0, 0.0),
		(27.0, 0.0),
		(0.0, 14.0),
		(40.0, 45.0),
		(0.0, 0.0),
		(13.0, 0.0),
	] {
		allocator.reset();
		let mut snapshot = engine.evaluate(Size::new(100, 100), &allocator);
		let position = engine.render(&mut snapshot).elements().last().unwrap().position;
		assert_eq!((position.x(), position.y()), (x, y));
	}
}

#[test]
fn custom_flow_observes_captured_state_after_a_paint_update() {
	thread_local! { static OFFSET: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) }; }
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		Box::pin(async move {
			let mut root = ctx
				.element("root")
				.container(Container::default().flow(|input: crate::ui::flow::FlowInput| {
					crate::ui::flow::FlowOutput::new(
						crate::ui::flow::Offset::new(OFFSET.with(std::cell::Cell::get), 0.0),
						input.cursor(),
					)
				}));
			let mut child = root.element("child").container(Container::default().size(20.into()));
			ctx.render().await;
			OFFSET.with(|offset| offset.set(40.0));
			child.update_container(|value| value.set_opacity(0.5));
		})
	});
	let allocator = bumpalo::Bump::new();
	let mut first = engine.evaluate(Size::new(100, 100), &allocator);
	assert_eq!(engine.render(&mut first).elements().nth(1).unwrap().position.x(), 0.0);
	let mut second = engine.evaluate(Size::new(100, 100), &allocator);
	assert_eq!(engine.render(&mut second).elements().nth(1).unwrap().position.x(), 40.0);
}

#[test]
fn snapshots_keep_distinct_geometry_when_custom_flow_changes_across_resizes() {
	thread_local! { static OFFSET: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) }; }
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		Box::pin(async move {
			ctx.element("root")
				.container(Container::default().flow(|input: crate::ui::flow::FlowInput| {
					crate::ui::flow::FlowOutput::new(
						crate::ui::flow::Offset::new(OFFSET.with(std::cell::Cell::get), 0.0),
						input.cursor(),
					)
				}))
				.element("child")
				.container(Container::default().size(20.into()));
		})
	});
	let allocator = bumpalo::Bump::new();
	let mut first = engine.evaluate(Size::new(100, 100), &allocator);
	OFFSET.with(|offset| offset.set(40.0));
	engine.evaluate(Size::new(200, 100), &allocator);
	let mut latest = engine.evaluate(Size::new(100, 100), &allocator);
	assert_eq!(engine.render(&mut latest).elements().nth(1).unwrap().position.x(), 40.0);
	assert_eq!(engine.render(&mut first).elements().nth(1).unwrap().position.x(), 0.0);
}

#[test]
fn input_updates_appearance_before_the_next_hit_geometry() {
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		Box::pin(async move {
			let mut root = ctx.element("root").container(Container::default().size(30.into()));
			root.element("child")
				.container(Container::default().size(20.into()).position(Position::absolute(40, 0)));
			root.on(Events::Actuated).await;
			root.update_container(|value| {
				value.set_opacity(0.25);
				value.set_clip(false);
			});
		})
	});
	let allocator = bumpalo::Bump::new();
	let mut initial = engine.evaluate(Size::new(100, 100), &allocator);
	assert_eq!(engine.render(&mut initial).elements().count(), 1);
	engine.set_cursor_position(UiPoint::new(-0.8, 0.8));
	engine.update_click_state(true);
	let mut clicked = engine.evaluate(Size::new(100, 100), &allocator);
	let child = engine.render(&mut clicked).elements().nth(1).unwrap();
	assert_eq!(child.opacity, 0.25);
	assert_eq!(child.clip, None);
	let child_id = Id::new(child.id).unwrap();
	let mut hits = crate::ui::intersection::HitTest::default();
	clicked.retain_hit_test(&mut hits);
	let point = UiPoint::new(-0.1, 0.9);
	assert_eq!(hits.query(point), None);
	let next = engine.evaluate(Size::new(100, 100), &allocator);
	next.retain_hit_test(&mut hits);
	assert_eq!(hits.query(point), Some(child_id));
}

#[test]
fn remounting_the_same_id_restores_its_context_geometry() {
	let observed = Rc::new(RefCell::new(Vec::new()));
	let output = Rc::clone(&observed);
	let mut engine = Engine::new();
	engine.mount(move |ctx| {
		Box::pin(async move {
			let mut root = ctx.element("root").container(Container::default());
			for _ in 0..2 {
				let output = Rc::clone(&output);
				root.element("scope")
					.mount(move |ctx| {
						Box::pin(async move {
							let child = ctx.element("child").container(Container::default().size(20.into()));
							ctx.render().await;
							output.borrow_mut().push((child.id(), child.geometry()));
						})
					})
					.await;
			}
		})
	});
	let allocator = bumpalo::Bump::new();
	for _ in 0..3 {
		engine.evaluate(Size::new(100, 100), &allocator);
	}
	let observed = observed.borrow();
	assert_eq!(observed.len(), 2);
	assert_eq!(observed[0], observed[1]);
	assert_eq!(observed[1].1.unwrap().size, Size::new(20, 20));
}
