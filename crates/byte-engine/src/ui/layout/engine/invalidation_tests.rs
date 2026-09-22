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
				actual.clip_mask,
				actual.opacity,
				actual.corner_radius,
				actual.corner_exponent
			),
			(
				expected.id,
				expected.position,
				expected.size,
				expected.clip,
				expected.clip_mask,
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
				actual.clip_mask,
				actual.opacity,
				actual.font_size,
				&actual.content
			),
			(
				expected.id,
				expected.position,
				expected.size,
				expected.clip,
				expected.clip_mask,
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
				actual.clip_mask,
				actual.opacity,
				&actual.segments
			),
			(
				expected.id,
				expected.position,
				expected.size,
				expected.clip,
				expected.clip_mask,
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

/// Builds adjacent text, an editable field, and a hit target to expose stale flow sizes.
fn text_scene(stage: usize) -> Engine<std::cell::Cell<usize>> {
	let mut engine = Engine::with_context(std::cell::Cell::new(stage));
	engine.mount(|ctx| {
		Box::pin(async move {
			let mut root = ctx
				.element("root")
				.container(Container::default().flow(flow::row).clip(false));
			let mut label = root.element("label").text(Text::new("ab"));
			let mut field = root.element("field").text_field(TextField::new("ab"));
			root.element("target").container(Container::default().size(20.into()));
			let mut applied = 0;
			loop {
				let target = ctx.ctx().get();
				for stage in applied + 1..=target {
					match stage {
						1 => {
							// Reversing two advances preserves width while content and appearance change.
							label.update_text(|value| {
								value.set_content("ba");
								value.set_style(ConcreteLayer::default().color(RGBA::new(0.2, 0.4, 0.6, 1.0).into()));
							});
							field.update_text_field(|value| {
								value.set_content("ba");
								value.set_opacity(0.5);
							});
						}
						2 => {
							// Multiple edits before evaluation must compare the final content's size.
							label.update_text(|value| value.set_content("temporary wider content"));
							label.update_text(|value| value.set_content("ab"));
						}
						3 => {
							label.update_text(|value| value.set_content("wider label"));
							field.update_text_field(|value| value.set_content("wider field"));
						}
						4 => {
							label.update_text(|value| value.settings.font_size = 24.0);
							field.update_text_field(|value| value.settings.font_size = 24.0);
						}
						5 => {
							label.update_text(|value| value.set_content("two\nlines"));
							field.update_text_field(|value| value.set_content(""));
						}
						6 => {
							label.update_text(|value| value.set_transform(Transform::identity().translate_x(10.0)));
							field.update_text_field(|value| value.set_transform(Transform::identity().scale(0.5)));
						}
						7 => {
							label.update_text(|value| value.set_content("lines\ntwo"));
							field.update_text_field(|value| value.set_content("ba"));
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

#[test]
fn text_edits_match_fresh_layout_rendering_and_hit_bounds() {
	let mut engine = text_scene(0);
	let mut allocator = bumpalo::Bump::new();
	let mut original = None;
	for stage in 0..=7 {
		allocator.reset();
		engine.ctx().set(stage);
		let mut fresh = text_scene(stage);
		let size = Size::new(if stage == 7 { 500 } else { 400 }, 200);
		let mut actual = engine.evaluate(size, &allocator);
		let mut expected = fresh.evaluate(size, &allocator);
		let actual_render = engine.render(&mut actual);
		let expected_render = fresh.render(&mut expected);
		assert_same_render(actual_render, expected_render);
		let (original, original_expected) = original.get_or_insert_with(|| (actual_render.clone(), expected_render.clone()));
		assert_same_render(original, original_expected);
		let mut actual_hits = crate::ui::intersection::HitTest::default();
		let mut expected_hits = crate::ui::intersection::HitTest::default();
		actual.retain_hit_test(&mut actual_hits);
		expected.retain_hit_test(&mut expected_hits);
		for id in expected.elements.iter().map(|element| element.id) {
			assert_eq!(actual_hits.bounds(id), expected_hits.bounds(id), "stage {stage}");
		}
	}
}

#[test]
fn text_edits_before_scope_removal_do_not_affect_replacement_content() {
	let mut engine = Engine::new();
	engine.mount(|ctx| {
		Box::pin(async move {
			let mut root = ctx.element("root").container(Container::default());
			for content in ["ab", "ba"] {
				root.element("temporary")
					.mount(move |ctx| {
						Box::pin(async move {
							let mut label = ctx.element("label").text(Text::new(content));
							ctx.render().await;
							label.update_text(|value| value.set_content("removed before layout"));
						})
					})
					.await;
			}
			let mut field = root.element("field").text_field(TextField::new("kept"));
			ctx.render().await;
			field.update_text_field(|value| value.set_content("much wider replacement"));
		})
	});
	let mut allocator = bumpalo::Bump::new();
	for content in ["ab", "ba", "kept", "much wider replacement"] {
		allocator.reset();
		let mut snapshot = engine.evaluate(Size::new(400, 200), &allocator);
		let render = engine.render(&mut snapshot);
		assert_eq!(render.texts().count(), 1);
		let text = render.texts().next().unwrap();
		assert_eq!(text.content, content);
		assert_eq!(text.size, TextSystem::new().measure(content, 16.0));
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
			let mut label = root.element("label").text(Text::new("ab"));
			ctx.render().await;
			OFFSET.with(|offset| offset.set(40.0));
			child.update_container(|value| value.set_opacity(0.5));
			ctx.render().await;
			OFFSET.with(|offset| offset.set(60.0));
			label.update_text(|value| value.set_content("ba"));
		})
	});
	let allocator = bumpalo::Bump::new();
	let mut first = engine.evaluate(Size::new(100, 100), &allocator);
	assert_eq!(engine.render(&mut first).elements().nth(1).unwrap().position.x(), 0.0);
	let mut second = engine.evaluate(Size::new(100, 100), &allocator);
	assert_eq!(engine.render(&mut second).elements().nth(1).unwrap().position.x(), 40.0);
	let mut third = engine.evaluate(Size::new(100, 100), &allocator);
	let render = engine.render(&mut third);
	assert_eq!(render.elements().nth(1).unwrap().position.x(), 60.0);
	assert_eq!(render.texts().next().unwrap().content, "ba");
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

/// Camera edits preserve flow results, nested pivots, clipping, and older snapshots.
#[test]
fn visual_subtrees_reuse_flow_placement_and_match_fresh_scenes() {
	thread_local! { static CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
	fn counted(input: crate::ui::FlowInput) -> crate::ui::FlowOutput {
		CALLS.with(|calls| calls.set(calls.get() + 1));
		flow::row(input)
	}
	fn scene(camera: Transform, nested: Transform) -> Engine<std::cell::Cell<(Transform, Transform)>> {
		let mut engine = Engine::with_context(std::cell::Cell::new((camera, nested)));
		engine.mount(|ctx| {
			Box::pin(async move {
				let (camera, nested) = ctx.ctx().get();
				let mut applied = (camera, nested);
				let mut root = ctx
					.element("root")
					.container(Container::default().flow(counted as fn(_) -> _));
				let mut canvas = root
					.element("canvas")
					.container(Container::default().size(100.into()).clip(false).transform(camera));
				let mut child = canvas
					.element("child")
					.container(Container::default().size(30.into()).flow(flow::column).transform(nested));
				child.element("label").text(Text::new("Local label"));
				child.element("wire").curve(
					Curve::new(CurvePath::new(20.into(), 20.into()).cubic((0., 0.), (5., 20.), (15., 0.), (20., 20.)))
						.hit_testable(6.0),
				);
				root.element("toolbar").container(Container::default().size(40.into()));
				loop {
					let (camera, nested) = ctx.ctx().get();
					if camera != applied.0 {
						canvas.update_container(|value| value.set_transform(camera));
					}
					if nested != applied.1 {
						child.update_container(|value| value.set_transform(nested));
					}
					applied = (camera, nested);
					ctx.render().await;
				}
			})
		});
		engine
	}
	let arena = bumpalo::Bump::new();
	let identity = Transform::identity();
	let mut retained = scene(identity, identity);
	let original = retained.evaluate(Size::new(300, 200), &arena);
	let original_elements = original.elements.to_vec();
	for (camera, nested) in [
		(identity.translate(12.25, 6.5), identity.scale(0.75)),
		(
			identity.translate(-70., 18.).scale(1.5),
			identity.translate(9., 3.).scale(0.5),
		),
		(identity.translate(500., 0.), identity),
		(identity, identity),
	] {
		retained.ctx().set((camera, nested));
		let calls = CALLS.with(std::cell::Cell::get);
		let mut actual = retained.evaluate(Size::new(300, 200), &arena);
		assert_eq!(
			CALLS.with(std::cell::Cell::get),
			calls,
			"A transform edit called the layout flow."
		);
		let mut fresh = scene(camera, nested);
		let mut expected = fresh.evaluate(Size::new(300, 200), &arena);
		assert_eq!(actual.elements, expected.elements);
		assert_same_render(retained.render(&mut actual), fresh.render(&mut expected));
		for y in (0..200).step_by(5) {
			for x in (0..300).step_by(5) {
				assert_eq!(
					actual.click(UiPoint::new(x as f32 / 150.0 - 1.0, 1.0 - y as f32 / 100.0)),
					expected.click(UiPoint::new(x as f32 / 150.0 - 1.0, 1.0 - y as f32 / 100.0))
				);
			}
		}
		assert_eq!(*original.elements, original_elements);
	}
}

/// Editing a retained path must move its hit surface even when its declared size stays fixed.
#[test]
fn edited_curve_paths_refresh_hits_and_preserve_older_snapshots() {
	let id = Rc::new(std::cell::Cell::new(None));
	let output = Rc::clone(&id);
	let mut engine = Engine::new();
	engine.mount(move |ctx| {
		Box::pin(async move {
			let mut root = ctx.element("root").container(Container::default());
			let mut wire = root
				.element("wire")
				.curve(Curve::new(CurvePath::new(100.into(), 100.into()).line((0., 0.), (40., 0.))).hit_testable(6.));
			output.set(Some(wire.id()));
			ctx.render().await;
			wire.update_curve(|curve| {
				let path = curve.path_mut();
				path.clear();
				path.push_line((0., 20.), (40., 20.));
			});
		})
	});
	let arena = bumpalo::Bump::new();
	let window = |x: f32, y: f32| UiPoint::new(x / 50.0 - 1.0, 1.0 - y / 50.0);
	let mut first = engine.evaluate(Size::new(100, 100), &arena);
	assert_eq!(first.click(window(20., 1.)), id.get());
	let mut second = engine.evaluate(Size::new(100, 100), &arena);
	assert_ne!(second.click(window(20., 1.)), id.get());
	assert_eq!(second.click(window(20., 21.)), id.get());
	assert_eq!(first.click(window(20., 1.)), id.get());
}

/// A transform edit refreshes clip and mask inheritance inside the moved subtree only,
/// so nested clips, rounded masks, absolute-depth layers, and untouched siblings must
/// all match a scene mounted with the transform already applied.
#[test]
fn transform_edits_refresh_appearance_inside_the_moved_subtree_only() {
	fn scene(outer: Transform, inner: Transform) -> Engine<std::cell::Cell<(Transform, Transform)>> {
		let mut engine = Engine::with_context(std::cell::Cell::new((outer, inner)));
		engine.mount(|ctx| {
			Box::pin(async move {
				let (outer, inner) = ctx.ctx().get();
				let mut applied = (outer, inner);
				let mut root = ctx
					.element("root")
					.container(Container::default().flow(flow::row).clip(true).corner_radius(6.0));
				let mut moved = root.element("moved").container(
					Container::default()
						.size(120.into())
						.flow(flow::column)
						.clip(true)
						.corner_radius(10.0)
						.transform(outer),
				);
				let mut nested = moved.element("nested").container(
					Container::default()
						.size(50.into())
						.clip(true)
						.corner_radius(4.0)
						.opacity(0.5)
						.transform(inner),
				);
				nested.element("leaf").container(Container::default().size(80.into()));
				nested.element("label").text(Text::new("Inside"));
				moved
					.element("overlay")
					.container(Container::default().size(30.into()).depth(Depth::Absolute(3)));
				let mut sibling = root
					.element("sibling")
					.container(Container::default().size(60.into()).clip(true).corner_radius(8.0));
				sibling.element("sibling_leaf").container(Container::default().size(90.into()));
				loop {
					let (outer, inner) = ctx.ctx().get();
					if outer != applied.0 {
						moved.update_container(|value| value.set_transform(outer));
					}
					if inner != applied.1 {
						nested.update_container(|value| value.set_transform(inner));
					}
					applied = (outer, inner);
					ctx.render().await;
				}
			})
		});
		engine
	}
	let arena = bumpalo::Bump::new();
	let identity = Transform::identity();
	let mut retained = scene(identity, identity);
	let _ = retained.evaluate(Size::new(300, 200), &arena);
	for (outer, inner) in [
		(identity.translate(15.5, 7.25), identity),
		(identity.translate(15.5, 7.25), identity.scale(1.6)),
		(identity.rotate(0.3).scale(0.8), identity.scale(1.6)),
		(identity.translate(-40., 20.), identity.translate(12., -8.).scale(2.0)),
		// The nested subtree leaves its parent's clip entirely, then returns.
		(identity.translate(-40., 20.), identity.translate(400., 0.)),
		(identity.translate(-40., 20.), identity.translate(12., -8.)),
		(identity, identity),
	] {
		retained.ctx().set((outer, inner));
		let mut actual = retained.evaluate(Size::new(300, 200), &arena);
		let mut fresh = scene(outer, inner);
		let mut expected = fresh.evaluate(Size::new(300, 200), &arena);
		assert_eq!(actual.elements, expected.elements);
		assert_same_render(retained.render(&mut actual), fresh.render(&mut expected));
		for y in (0..200).step_by(4) {
			for x in (0..300).step_by(4) {
				let point = UiPoint::new(x as f32 / 150.0 - 1.0, 1.0 - y as f32 / 100.0);
				assert_eq!(actual.click(point), expected.click(point));
			}
		}
	}
}
