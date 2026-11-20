//! Draws the toast stack in the window's bottom-right corner: one card per toast, easing toward the place
//! [`Toasts::placement`](crate::editor::toasts::Toasts::placement) gives it.

use super::{CORNER, MARGIN, MUTED, SQUIRCLE, Shell, TEXT, hex, panel, row_style, solid};
use crate::editor::toasts::{HEIGHT, WIDTH};
use crate::ui::animation::{Animation, Smoothing};
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::ui::{Anchor, Position, Transform, UiPoint, flow};
use crate::utils::r#async::select;

/// How far below its place a new card starts, so it rises in.
const ENTER: f32 = 24.0;
/// How fast a card closes on its place: half the distance in this many seconds.
const HALF_LIFE: f32 = 0.05;
/// The colors of a note's, a warning's, and an error's icon: Sonner's blue, amber, and red.
const INFO: crate::utils::RGBA = hex(0x3E9FF5, 1.0);
const WARNING: crate::utils::RGBA = hex(0xF5A524, 1.0);
const ERROR: crate::utils::RGBA = hex(0xE5484D, 1.0);

/// Ages the toasts every frame while any shows and keeps one card per toast, newer cards over older ones.
pub(super) async fn toasts(ctx: &mut EvaluationContext<Shell>) {
	// Each card's holder, by toast, and the last depth given to it.
	let mut holders: Vec<(u64, i32, EvaluationContext<Shell>)> = Vec::new();
	// Reused every frame, so reading the stack allocates nothing.
	let mut placed: Vec<(u64, i32)> = Vec::new();
	// The toasts age by each frame's time, from the engine's clock.
	let mut dt = 0.0;
	// Nothing ages while no toast shows, so the stack sleeps until one is posted.
	let mut empty = ctx.proactive(|shell: &Shell| shell.toasts.list().is_empty());
	loop {
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
		if placed.is_empty() {
			while *empty.next().await {}
			dt = 0.0;
		} else {
			dt = ctx.render().await.as_seconds_f32();
		}
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
	let (color, mark) = match toast.level {
		log::Level::Error => (ERROR, "!"),
		log::Level::Warn => (WARNING, "!"),
		_ => (INFO, "i"),
	};
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
				.style(panel(1.0))
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
		.text(mark, |t| t.font_size(12.0).style([solid(hex(0x1E1F24, 1.0))]))
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

	// The card's offset, scale, and opacity, chasing its placement, which moves as the stack spreads and toasts leave.
	// The engine moves the card while it chases.
	let card_id = card.id();
	let look = ctx
		.animation(
			Animation::new([ENTER, 1.0, 0.0], Smoothing::default().half_life(HALF_LIFE)),
			move |frame, v| {
				let [y, scale, opacity] = v.current;
				frame.update_container(card_id, |c| {
					c.transform(
						Transform::identity()
							.translate_y(y)
							.scale(scale)
							.origin(UiPoint::new(0.5, 0.0)),
					)
					.opacity(opacity)
				});
			},
		)
		.await;
	let (mut hovered, mut close_hovered) = (false, false);
	let mut shown_count = 1;
	let mut shown_close = false;
	// The toast's placement, which the stack changes as the toasts age, and how many times it was posted.
	let mut state = ctx.proactive(move |shell: &Shell| {
		let count = shell.toasts.list().iter().find(|toast| toast.id == id)?.count;
		Some((shell.toasts.placement(id)?, count))
	});
	state.next().await;
	// Whether the stack was last told the pointer is over the card.
	let mut told_over = false;
	loop {
		select! {
			_ = close.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| shell.toasts.dismiss(id)).await;
			},
			_ = card.on(Events::PointerEntered) => hovered = true,
			_ = card.on(Events::PointerExited) => hovered = false,
			_ = close.on(Events::PointerEntered) => close_hovered = true,
			_ = close.on(Events::PointerExited) => close_hovered = false,
			_ = state.next() => {},
		}
		// The close button is over the card, so the pointer on it still holds the stack open.
		let over = hovered || close_hovered;
		if over != told_over {
			told_over = over;
			ctx.with(move |shell: &mut Shell| {
				if let Some(toast) = shell.toasts.get_mut(id) {
					toast.hovered = over;
				}
			})
			.await;
		}
		let Some(&Some((placement, repeats))) = state.latest() else {
			// The stack dropped the toast; the stack removes the card next.
			return;
		};
		look.set_target([placement.y, placement.scale, placement.opacity]);
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
