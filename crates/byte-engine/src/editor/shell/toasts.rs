//! Draws the toast stack in the window's bottom-right corner: one card per toast, easing toward the place
//! [`Toasts::placement`](crate::editor::toasts::Toasts::placement) gives it.

use std::time::Instant;

use super::{CORNER, MARGIN, MUTED, SQUIRCLE, Shell, TEXT, hex, panel, row_style, solid};
use crate::editor::toasts::{HEIGHT, WIDTH};
use crate::time::MediaTime;
use crate::ui::animation::{Smoothing, smooth};
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::ui::{Anchor, Position, Transform, UiPoint, flow};
use crate::utils::r#async::select;

/// How far below its place a new card starts, so it rises in.
const ENTER: f32 = 24.0;
/// How fast a card closes on its place: half the distance in this many seconds.
const HALF_LIFE: f32 = 0.05;
/// The longest step a card's easing takes, so a stalled frame doesn't make it jump.
const MAX_STEP: f32 = 1.0 / 30.0;
/// The colors of a warning's and an error's icon: Sonner's amber and red.
const WARNING: crate::utils::RGBA = hex(0xF5A524, 1.0);
const ERROR: crate::utils::RGBA = hex(0xE5484D, 1.0);

/// Ages the toasts every frame and keeps one card per toast, newer cards over older ones.
pub(super) async fn toasts(ctx: &mut EvaluationContext<Shell>) {
	// Each card's holder, by toast, and the last depth given to it.
	let mut holders: Vec<(u64, i32, EvaluationContext<Shell>)> = Vec::new();
	// Reused every frame, so reading the stack allocates nothing.
	let mut placed: Vec<(u64, i32)> = Vec::new();
	let mut last = Instant::now();
	loop {
		let dt = step(&mut last);
		placed = ctx
			.with(move |shell: &mut Shell| {
				shell.toasts.advance(dt);
				placed.clear();
				placed.extend(
					shell
						.toasts
						.list()
						.iter()
						.filter_map(|toast| Some((toast.id, shell.toasts.placement(toast.id)?.depth))),
				);
				placed
			})
			.await;

		// A dropped toast's card goes with its holder.
		let mut index = 0;
		while index < holders.len() {
			if placed.iter().any(|(id, _)| *id == holders[index].0) {
				index += 1;
			} else {
				holders.swap_remove(index).2.remove().await;
			}
		}
		for &(id, depth) in &placed {
			match holders.iter_mut().find(|(holder, ..)| *holder == id) {
				// A toast posted again comes to the front, so its card's order among the others changes.
				Some((_, shown, holder)) if *shown != depth => {
					*shown = depth;
					holder.update_container(move |c| c.depth(depth)).await;
				}
				Some(_) => {}
				None => {
					let mut holder = ctx
						.element(("toast", id as usize))
						.container(move |c| {
							c.position(Position::anchored(Anchor::End(MARGIN), Anchor::End(MARGIN)))
								.width(WIDTH.into())
								.height(HEIGHT.into())
								.clip(false)
								.depth(depth)
						})
						.await;
					holder
						.element("card")
						.component(async move |card| card_view(card, id).await)
						.await;
					holders.push((id, depth, holder));
				}
			}
		}
		ctx.render().await;
	}
}

/// One toast's card: an icon in the level's color, the title over the detail, a count once the message repeats, and a
/// close button. It follows the pointer, so the stack spreads out, and eases toward its placement.
async fn card_view(ctx: &mut EvaluationContext<Shell>, id: u64) {
	let Some(toast) = ctx
		.with(move |shell: &mut Shell| shell.toasts.get_mut(id).map(|toast| toast.clone()))
		.await
	else {
		return;
	};
	let color = if toast.level == log::Level::Error { ERROR } else { WARNING };
	let mut card = ctx
		.element("card")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(WIDTH.into())
				.height(HEIGHT.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.hit_testable(true)
				.opacity(0.0)
				.transform(Transform::identity().translate_y(ENTER))
				.style(panel(HEIGHT, 1.0))
		})
		.await;
	card.element("icon")
		.container(|c| {
			c.absolute_position(14.0, 12.0)
				.width(18.into())
				.height(18.into())
				.corner_radius(9.0)
				.flow(flow::center)
				.style([solid(color)])
		})
		.await
		.text("!", |t| t.font_size(12.0).style([solid(hex(0x1E1F24, 1.0))]))
		.await;
	// The UI doesn't wrap text, so each line is cut at the card's edge.
	let words = WIDTH - 44.0 - 64.0;
	card.element("title")
		.container(|c| {
			c.absolute_position(44.0, 12.0)
				.width(words.into())
				.height(18.into())
				.clip(true)
				.flow(flow::centered_row)
		})
		.await
		.text(&toast.title, |t| t.font_size(13.0).style([solid(TEXT)]))
		.await;
	card.element("detail")
		.container(|c| {
			c.absolute_position(44.0, 32.0)
				.width(words.into())
				.height(16.into())
				.clip(true)
				.flow(flow::centered_row)
		})
		.await
		.text(&toast.detail, |t| t.font_size(12.0).style([solid(MUTED)]))
		.await;
	let mut count = card
		.element("count")
		.container(|c| {
			c.absolute_position(WIDTH - 62.0, 12.0)
				.width(28.into())
				.height(18.into())
				.flow(flow::center)
		})
		.await
		.text("", |t| t.font_size(11.0).style([solid(MUTED)]))
		.await;
	let mut close = card
		.element("close")
		.container(|c| {
			c.absolute_position(WIDTH - 32.0, 10.0)
				.width(22.into())
				.height(22.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	let mut cross = close.text("×", |t| t.font_size(15.0).style([solid(MUTED)])).await;

	let mut y = eased(ENTER, 0.0);
	let mut scale = eased(1.0, 1.0);
	let mut opacity = eased(0.0, 1.0);
	let mut last = Instant::now();
	let (mut hovered, mut close_hovered) = (false, false);
	let mut shown = None;
	let mut shown_count = 1;
	let mut shown_close = false;
	loop {
		select! {
			_ = close.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| shell.toasts.dismiss(id)).await;
			},
			_ = card.on(Events::PointerEntered) => hovered = true,
			_ = card.on(Events::PointerExited) => hovered = false,
			_ = close.on(Events::PointerEntered) => close_hovered = true,
			_ = close.on(Events::PointerExited) => close_hovered = false,
			_ = ctx.render() => {},
		}
		// The close button is over the card, so the pointer on it still holds the stack open.
		let over = hovered || close_hovered;
		let Some((placement, repeats)) = ctx
			.with(move |shell: &mut Shell| {
				let toast = shell.toasts.get_mut(id)?;
				toast.hovered = over;
				let repeats = toast.count;
				Some((shell.toasts.placement(id)?, repeats))
			})
			.await
		else {
			// The stack dropped the toast; the stack removes the card next.
			return;
		};

		let dt = MediaTime::from_seconds_f32(step(&mut last));
		y.set_target(placement.y);
		scale.set_target(placement.scale);
		opacity.set_target(placement.opacity);
		let look = [y.step(dt), scale.step(dt), opacity.step(dt)].map(settle);
		if shown != Some(look) {
			shown = Some(look);
			let [y, scale, opacity] = look;
			card.update_container(move |c| {
				c.transform(
					Transform::identity()
						.translate_y(y)
						.scale(scale)
						.origin(UiPoint::new(0.5, 0.0)),
				)
				.opacity(opacity)
			})
			.await;
		}
		if repeats != shown_count {
			shown_count = repeats;
			count.update_text(move |t| t.content(format_args!("{repeats}×"))).await;
		}
		if close_hovered != shown_close {
			shown_close = close_hovered;
			close
				.update_container(move |c| c.style(row_style(false, close_hovered)))
				.await;
			let tint = if close_hovered { TEXT } else { MUTED };
			cross.update_text(move |t| t.style([solid(tint)])).await;
		}
	}
}

/// An easing from `from` toward `to` at the cards' pace.
fn eased(from: f32, to: f32) -> Smoothing {
	smooth(from, to).half_life(HALF_LIFE)
}

/// Rounds an eased value to a hundredth, so a card that has all but arrived stops updating.
fn settle(value: f32) -> f32 {
	(value * 100.0).round() / 100.0
}

/// Returns the seconds since `last`, at most [`MAX_STEP`], and moves `last` to now.
fn step(last: &mut Instant) -> f32 {
	let now = Instant::now();
	let dt = now.duration_since(*last).as_secs_f32().min(MAX_STEP);
	*last = now;
	dt
}
