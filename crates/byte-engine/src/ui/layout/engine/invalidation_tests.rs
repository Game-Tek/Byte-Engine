//! Observable comparisons between retained updates and fresh scene construction.

use super::*;
use crate::ui::{
	ContainerContext, Position, Sizing, flow,
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
		assert_eq!(actual.style.layers().len(), expected.style.layers().len());
		for (actual, expected) in actual.style.layers().iter().zip(expected.style.layers()) {
			let (Color::Value(actual_color), Color::Value(expected_color)) = (actual.fill(), expected.fill()) else {
				panic!("Expected solid fixture colors")
			};
			assert_eq!(actual_color, expected_color);
			assert_eq!(actual.feather(), expected.feather());
		}
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
		for x in [0.0, 10.0, 30.0, 50.0, 90.0] {
			for y in [0.0, 20.0, 80.0] {
				assert_eq!(
					actual.hit(UiPoint::new(x, y), None),
					expected.hit(UiPoint::new(x, y), None),
					"stage {stage}"
				);
			}
		}
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
	assert_eq!(clicked.hit(UiPoint::new(45.0, 5.0), None), None);
	let next = engine.evaluate(Size::new(100, 100), &allocator);
	assert_eq!(next.hit(UiPoint::new(45.0, 5.0), None), Some(child_id));
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
