//! The hint bar along the window's bottom: the keys the editor offers next, each as raised keycaps beside what they do,
//! from [`Shell::hints`].
//!
//! Containers don't size to what they hold, so each hint first lays out at a width guessed from its text, and once the
//! layout measured its text, the caps, the hints, and the bar take their measured widths.

use super::{CORNER, MARGIN, SQUIRCLE, Shell, TEXT, TOOLBAR_MUTED, panel, raised, solid};
use crate::editor::bindings::Binding;
use crate::ui::layout::context::{Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::{Anchor, Position, flow};

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
/// How wide a character is guessed to be before the layout measures it.
const GUESS: f32 = 6.5;
const KEY_SIZE: f32 = 11.0;
const LABEL_SIZE: f32 = 12.0;

/// The `Drawn` struct is one hint the bar shows: its row, the row of its keys, each key's cap and text, and what it
/// does.
struct Drawn {
	row: EvaluationContext<Shell>,
	keys: EvaluationContext<Shell>,
	caps: Vec<(EvaluationContext<Shell>, EvaluationContext<Shell>)>,
	label: EvaluationContext<Shell>,
}

/// Shows [`Shell::hints`] for as long as the shell runs, declaring its hints again whenever the editor offers others.
pub(super) async fn hint_bar(ctx: &mut EvaluationContext<Shell>) {
	let mut bar = ctx
		.element("bar")
		.container(|c| {
			c.position(Position::anchored(Anchor::Center(0.0), Anchor::End(MARGIN)))
				.width(0.into())
				.height(HEIGHT.into())
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
	let mut drawn: Vec<Drawn> = Vec::new();
	let mut measured = true;
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
			for mut hint in drawn.drain(..) {
				hint.row.remove().await;
			}
			for (index, binding) in shown.iter().enumerate() {
				drawn.push(declare(&mut bar, index, binding).await);
			}
			let width = bar_width(shown.iter().map(|binding| guessed_width(binding)));
			let opacity = if shown.is_empty() { 0.0 } else { 1.0 };
			bar.update_container(|c| c.width(width.into()).opacity(opacity)).await;
			measured = shown.is_empty();
		} else if !measured {
			measured = fit(&mut bar, &mut drawn).await;
		}
		ctx.render().await;
	}
}

/// Declares `binding` as hint `index` in `bar`, at the widths its text is guessed to take.
async fn declare(bar: &mut EvaluationContext<Shell>, index: usize, binding: &Binding) -> Drawn {
	let keys_guess = keys_width(binding.keys.iter().map(|key| guess(key.label())));
	let width = guessed_width(binding);
	let mut row = bar
		.element(("hint", index))
		.container(|c| {
			c.width(width.into())
				.height(HEIGHT.into())
				.flow(flow::centered_row_with_gap(LABEL_GAP))
		})
		.await;
	let mut keys = row
		.element("keys")
		.container(|c| {
			c.width(keys_guess.into())
				.height(CAP_HEIGHT.into())
				.flow(flow::centered_row_with_gap(CAP_GAP))
		})
		.await;
	let mut caps = Vec::with_capacity(binding.keys.len());
	for (index, key) in binding.keys.iter().map(|key| key.label()).enumerate() {
		let mut cap = keys
			.element(("key", index))
			.container(|c| {
				c.width(cap_width(guess(key)).into())
					.height(CAP_HEIGHT.into())
					.corner_radius(5.0)
					.corner_exponent(SQUIRCLE)
					.flow(flow::center)
					.style(raised(CAP_HEIGHT, false))
			})
			.await;
		let text = cap
			.element("text")
			.text(key, |t| t.font_size(KEY_SIZE).style([solid(TEXT)]))
			.await;
		caps.push((cap, text));
	}
	let label = row
		.element("label")
		.text(binding.label, |t| t.font_size(LABEL_SIZE).style([solid(TOOLBAR_MUTED)]))
		.await;
	Drawn { row, keys, caps, label }
}

/// Gives the caps, the hints, and the bar the widths the layout measured their text at, and returns whether it
/// measured every text yet.
async fn fit(bar: &mut EvaluationContext<Shell>, drawn: &mut [Drawn]) -> bool {
	// Every text is laid out in the same evaluation, so one measured means all are.
	let mut widths = 0.0;
	for (index, hint) in drawn.iter_mut().enumerate() {
		let mut keys = 0.0;
		for (count, (cap, text)) in hint.caps.iter_mut().enumerate() {
			let Some(geometry) = text.geometry().await else {
				return false;
			};
			let width = cap_width(geometry.width());
			cap.update_container(|c| c.width(width.into())).await;
			keys += width + if count > 0 { CAP_GAP } else { 0.0 };
		}
		let Some(label) = hint.label.geometry().await else {
			return false;
		};
		hint.keys.update_container(|c| c.width(keys.into())).await;
		let width = hint_width(keys, label.width());
		hint.row.update_container(|c| c.width(width.into())).await;
		widths += width + if index > 0 { HINT_GAP } else { 0.0 };
	}
	let width = widths + PADDING * 2.0;
	bar.update_container(|c| c.width(width.into())).await;
	true
}

/// The width `binding`'s hint is guessed to take before the layout measures its text.
fn guessed_width(binding: &Binding) -> f32 {
	hint_width(keys_width(binding.keys.iter().map(|key| guess(key.label()))), guess(binding.label))
}

/// The width `text` is guessed to take before the layout measures it.
fn guess(text: &str) -> f32 {
	text.chars().count() as f32 * GUESS
}

/// The width of a keycap whose text is `text` wide.
fn cap_width(text: f32) -> f32 {
	text + CAP_PADDING * 2.0
}

/// The width of a row of caps whose texts are `texts` wide.
fn keys_width(texts: impl Iterator<Item = f32>) -> f32 {
	texts
		.enumerate()
		.map(|(index, text)| cap_width(text) + if index > 0 { CAP_GAP } else { 0.0 })
		.sum()
}

/// The width of a hint whose keys are `keys` wide and whose label is `label` wide.
fn hint_width(keys: f32, label: f32) -> f32 {
	keys + LABEL_GAP + label
}

/// The bar's width around hints `widths` wide.
fn bar_width(widths: impl Iterator<Item = f32>) -> f32 {
	widths
		.enumerate()
		.map(|(index, width)| width + if index > 0 { HINT_GAP } else { 0.0 })
		.sum::<f32>()
		+ PADDING * 2.0
}
