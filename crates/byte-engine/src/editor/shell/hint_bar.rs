//! The hint bar along the window's bottom: the keys the editor offers next, each as raised keycaps beside what they do,
//! from [`Shell::hints`]. Every part of it fits what it holds, so the bar is as wide as its hints.

use super::{CORNER, MARGIN, SQUIRCLE, Shell, TEXT, TOOLBAR_MUTED, panel, raised, solid};
use crate::editor::bindings::Binding;
use crate::ui::layout::context::{Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::{Anchor, Position, Sizing, flow};

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

/// Shows [`Shell::hints`] for as long as the shell runs, declaring its hints again whenever the editor offers others.
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
				.style(panel(HEIGHT, 0.9))
		})
		.await;
	// The bindings shown, kept to tell when the editor offers others; reused, so reading them allocates nothing.
	let mut shown: Vec<&'static Binding> = Vec::new();
	let mut rows: Vec<EvaluationContext<Shell>> = Vec::new();
	loop {
		let changed;
		(shown, changed) = ctx
			.with(move |shell: &mut Shell| {
				// The bindings are statics, so a binding at another address is another binding.
				let changed = shown.len() != shell.hints.len()
					|| shown.iter().zip(&shell.hints).any(|(a, b)| !std::ptr::eq(*a, *b));
				if changed {
					shown.clear();
					shown.extend_from_slice(&shell.hints);
				}
				(shown, changed)
			})
			.await;
		if changed {
			for mut row in rows.drain(..) {
				row.remove().await;
			}
			for (index, binding) in shown.iter().enumerate() {
				rows.push(declare(&mut bar, index, binding).await);
			}
			let opacity = if shown.is_empty() { 0.0 } else { 1.0 };
			bar.update_container(|c| c.opacity(opacity)).await;
		}
		ctx.render().await;
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
					.style(raised(CAP_HEIGHT, false))
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
