//! The hint bar along the window's bottom: the keys that would finish what the user left partway, each as raised
//! keycaps beside what they do. Every part of it fits what it holds, so the bar is as wide as its hints.

use std::time::Duration;

use super::{CORNER, MARGIN, SQUIRCLE, Shell, TEXT, TOOLBAR_MUTED, panel, raised, solid};
use crate::editor::bindings::{self, Binding, Held, Situation};
use crate::ui::layout::context::{Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::{Anchor, Position, Sizing, flow};
use crate::utils::r#async::select;

/// How long an action left partway, a gizmo held still or a modifier held without a second key, waits before the bar
/// offers what finishes it.
const HESITATION: Duration = Duration::from_secs(1);
/// The bar's height.
const HEIGHT: f32 = 32.0;
/// The room between the bar's edge and its first and last hint.
const PADDING: f32 = 12.0;
/// The room between two hints.
const HINT_GAP: f32 = 16.0;
/// A keycap's height, and the room on each side of its text.
const CAP_HEIGHT: f32 = 20.0;
const CAP_PADDING: f32 = 6.0;
/// The room between two keycaps of one hint, and between the last and what the hint does.
const CAP_GAP: f32 = 3.0;
const LABEL_GAP: f32 = 6.0;
const KEY_SIZE: f32 = 11.0;
const LABEL_SIZE: f32 = 12.0;

/// Offers what would finish the action the user left partway, for as long as the shell runs.
///
/// The bar stays empty until the user stops partway for [`HESITATION`]: a gizmo drag whose pointer stood still, or
/// modifiers held without the key that finishes a shortcut. Any change of the held keys or the dragged pointer hides
/// it and starts the wait again, and a shortcut run with the modifiers still down ends it. It follows the gesture the
/// editor publishes in [`Shell::held`], [`Shell::dragged`], and [`Shell::finished`], so it runs only when those
/// change or the wait ends.
pub(super) async fn hint_bar(ctx: &mut EvaluationContext<Shell>) {
	let mut bar = ctx
		.element("bar")
		.container(|c| {
			c.position(Position::anchored(Anchor::Center(0.0), Anchor::End(MARGIN)))
				.width(Sizing::fit())
				.height(HEIGHT.into())
				.padding(PADDING, 0.0)
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.opacity(0.0)
				.flow(flow::centered_row_with_gap(HINT_GAP))
				// Blurred, as the toolbar is, but denser, so its small text reads over a bright sky.
				.style(panel(0.9))
		})
		.await;
	let mut gesture = ctx.proactive(|shell: &Shell| (shell.held, shell.dragged, shell.finished));
	// The offered bindings, and the rows showing them; both reused, so offering again allocates nothing.
	let mut hints: Vec<&'static Binding> = Vec::new();
	let mut rows: Vec<EvaluationContext<Shell>> = Vec::new();
	let mut current = *gesture.next().await;
	loop {
		let (held, dragged, finished) = current;
		let partway = dragged.is_some() || (held.modifiers() != Held::default() && !finished);
		if partway {
			// The gesture changed before the user stopped: the wait starts again from the new gesture.
			select! {
				next = gesture.next() => {
					current = *next;
					continue;
				},
				_ = ctx.wait(HESITATION) => {},
			}
			ctx.with(|shell: &mut Shell| {
				let situation = Situation {
					layers: &shell.layers,
					scene: true,
					dragging: dragged.is_some(),
					tool: shell.tool,
				};
				bindings::offered(&situation, held, &mut hints);
			})
			.await;
			for (index, binding) in hints.iter().enumerate() {
				rows.push(declare(&mut bar, index, binding).await);
			}
			if !hints.is_empty() {
				bar.update_container(|c| c.opacity(1.0)).await;
			}
		}
		current = *gesture.next().await;
		if !rows.is_empty() {
			for mut row in rows.drain(..) {
				row.remove().await;
			}
			bar.update_container(|c| c.opacity(0.0)).await;
		}
	}
}

/// Declares `binding` as hint `index` in `bar`: its keycaps, then what it does. Returns the hint's row.
async fn declare(bar: &mut EvaluationContext<Shell>, index: usize, binding: &Binding) -> EvaluationContext<Shell> {
	let mut row = bar
		.element(("hint", index))
		.container(|c| c.size(Sizing::fit()).flow(flow::centered_row_with_gap(LABEL_GAP)))
		.await;
	let mut keys = row
		.element("keys")
		.container(|c| c.size(Sizing::fit()).flow(flow::centered_row_with_gap(CAP_GAP)))
		.await;
	for (index, key) in binding.keys.iter().map(|key| key.label()).enumerate() {
		keys.element(("key", index))
			.container(|c| {
				c.width(Sizing::fit())
					.height(CAP_HEIGHT.into())
					.padding(CAP_PADDING, 0.0)
					.corner_radius(5.0)
					.corner_exponent(SQUIRCLE)
					.flow(flow::centered_row)
					.style(raised(false))
			})
			.await
			.element("text")
			.text(key, |t| t.font_size(KEY_SIZE).style([solid(TEXT)]))
			.await;
	}
	row.element("label")
		.text(binding.label, |t| t.font_size(LABEL_SIZE).style([solid(TOOLBAR_MUTED)]))
		.await;
	row
}

#[cfg(test)]
mod tests {
	use std::time::Instant;

	use super::*;
	use crate::editor::bindings::Key;
	use crate::ui::{Engine, Size};

	fn frame(engine: &mut Engine<Shell>) {
		engine.evaluate(Size::new(800, 600), &bumpalo::Bump::new());
	}

	#[test]
	fn the_bar_waits_on_one_timer_while_the_user_is_partway_and_on_nothing_otherwise() {
		let mut engine = Engine::with_context(Shell::new(Size::new(800, 600)));
		engine.mount(async |ctx| hint_bar(ctx).await);
		frame(&mut engine);
		assert_eq!(engine.next_tick(), None, "an idle bar asked for a tick");

		engine.ctx_mut().held = Held::of(&[Key::Cmd]);
		frame(&mut engine);
		let deadline = engine.next_tick().expect("a held modifier started no hesitation wait");
		assert!(
			deadline > Instant::now() + HESITATION / 2,
			"the bar asked for frames instead of waiting for the hesitation"
		);

		engine.ctx_mut().held = Held::default();
		frame(&mut engine);
		assert_eq!(engine.next_tick(), None, "letting go of the modifier left the wait running");
	}
}
