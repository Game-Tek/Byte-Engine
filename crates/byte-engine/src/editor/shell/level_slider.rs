//! The detail level slider a mesh window shows at its top, centered over the room between its drawers: a domed knob in
//! a channel, over one stop for **Auto** and one for each of the mesh's detail levels.
//!
//! The knob follows a drag, and the mesh changes level as it passes each stop; let go, or click a stop, and it glides
//! to the nearest one on the editor's curve, lifted while held. The slider only reports the level, through
//! [`Shell::detail_level`]; the editor gives it to the stage, which draws the mesh at it. See
//! [`crate::editor::preview::Stage::show_level`].

use super::{
	ACCENT_RGB, CORNER, MUTED, SQUIRCLE, Shell, TEXT, cap, channel, darker, hex, open_room, panel, solid, top_highlight,
	vertical,
};
use crate::editor::drawer::arriving;
use crate::ui::Animation;
use crate::ui::flow;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::ui::{ConcreteLayer, Transform};
use crate::utils::r#async::select;

/// The distance between two stops, which is also the width of each stop's label.
const STOP_SPACING: f32 = 46.0;
/// The room left of the first stop's label and right of the last's.
const SIDE: f32 = 10.0;
/// The slider's height, and its gap from the window's top.
const HEIGHT: f32 = 50.0;
const TOP: f32 = 12.0;
/// The channel's height and its top, inside the slider.
const TRACK_HEIGHT: f32 = 6.0;
const TRACK_TOP: f32 = 15.0;
/// The knob's size, and how much larger it shows while held.
const KNOB: f32 = 18.0;
const LIFT: f32 = 0.18;
/// The top of the stops' labels, inside the slider, and their height.
const LABEL_TOP: f32 = 29.0;
const LABEL_HEIGHT: f32 = 14.0;
/// The size of the dot that marks each stop in the channel.
const TICK: f32 = 4.0;

/// How long the knob takes to glide to a stop, and to lift or set down, in seconds.
const GLIDE: f32 = 0.2;

/// Where stop `index` stands from the slider's left edge.
fn stop_x(index: usize) -> f32 {
	SIDE + STOP_SPACING * (index as f32 + 0.5)
}

/// The stop nearest `x`, from the slider's left edge, among `count`.
fn nearest_stop(x: f32, count: usize) -> usize {
	(((x - SIDE) / STOP_SPACING - 0.5).round().max(0.0) as usize).min(count.saturating_sub(1))
}

/// The detail level stop `index` draws: none, by distance, for the first, **Auto**, then each level from 0.
fn stop_level(index: usize) -> Option<u32> {
	index.checked_sub(1).map(|level| level as u32)
}

/// Shows the slider, over **Auto** and `levels` detail levels, until the view that declared it removes it.
pub(super) async fn level_slider(ctx: &mut EvaluationContext<Shell>, levels: usize) {
	let count = levels + 1;
	let width = SIDE * 2.0 + STOP_SPACING * count as f32;
	// A level kept from a mesh with more levels than this one shows as its coarsest.
	let chosen = ctx
		.with(|shell: &mut Shell| shell.detail_level.map_or(0, |level| level as usize + 1))
		.await
		.min(count - 1);
	let mut pill = ctx
		.element("pill")
		.container(|c| {
			c.absolute_position(0.0, TOP)
				.width(width.into())
				.height(HEIGHT.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				// The slider takes presses, so a drag on it never turns the mesh behind it.
				.hit_testable(true)
				.style(panel(0.85))
		})
		.await;
	let (first, last) = (stop_x(0), stop_x(count - 1));
	let mut track = pill
		.element("track")
		.container(|c| {
			c.absolute_position(first - TRACK_HEIGHT * 0.5, TRACK_TOP)
				.width((last - first + TRACK_HEIGHT).into())
				.height(TRACK_HEIGHT.into())
				.corner_radius(TRACK_HEIGHT * 0.5)
				.style(channel(0.0))
		})
		.await;
	// The accent runs from the first stop to the knob, so a pinned level reads as filled in.
	let fill = track
		.element("fill")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(TRACK_HEIGHT.into())
				.height(TRACK_HEIGHT.into())
				.corner_radius(TRACK_HEIGHT * 0.5)
				.style([
					solid(hex(0x000000, 0.0)),
					ConcreteLayer::default().color(vertical(hex(darker(ACCENT_RGB), 1.0), hex(ACCENT_RGB, 1.0))),
					top_highlight(0.12),
				])
		})
		.await;
	let center_y = TRACK_TOP + TRACK_HEIGHT * 0.5;
	let mut labels = Vec::with_capacity(count);
	for index in 0..count {
		pill.element(("tick", index))
			.container(|c| {
				c.absolute_position(stop_x(index) - TICK * 0.5, center_y - TICK * 0.5)
					.width(TICK.into())
					.height(TICK.into())
					.corner_radius(TICK * 0.5)
					.style([solid(hex(0xFFFFFF, 0.22))])
			})
			.await;
		let text = if index == 0 {
			"Auto".to_owned()
		} else {
			(index - 1).to_string()
		};
		let mut label = pill
			.element(("label", index))
			.container(|c| {
				c.absolute_position(stop_x(index) - STOP_SPACING * 0.5, LABEL_TOP)
					.width(STOP_SPACING.into())
					.height(LABEL_HEIGHT.into())
					.flow(flow::center)
			})
			.await;
		labels.push(
			label
				.element("text")
				.text(&text, |t| {
					t.font_size(11.0).style([solid(if index == chosen { TEXT } else { MUTED })])
				})
				.await,
		);
	}
	let knob = pill
		.element("knob")
		.container(|c| {
			c.absolute_position(stop_x(chosen) - KNOB * 0.5, center_y - KNOB * 0.5)
				.width(KNOB.into())
				.height(KNOB.into())
				.corner_radius(KNOB * 0.5)
				.style(cap())
		})
		.await;

	// Where the knob stands, gliding toward a stop, which places the knob and fills the track up to it; and how far
	// the knob is lifted. The engine moves both while they glide.
	let (knob_id, fill_id) = (knob.id(), fill.id());
	let motion = ctx
		.animation(Animation::new(first, arriving(GLIDE)), move |frame, v| {
			frame.update_container(knob_id, |c| {
				c.absolute_position(v.current - KNOB * 0.5, center_y - KNOB * 0.5)
			});
			frame.update_container(fill_id, |c| c.width((v.current - first + TRACK_HEIGHT).into()));
		})
		.await;
	// It starts at the first stop and snaps to the chosen one, so the first frame also sizes the fill.
	motion.snap(stop_x(chosen)).await;
	let lift = ctx
		.animation(Animation::new(0.0, arriving(GLIDE)), move |frame, v| {
			frame.update_container(knob_id, |c| c.transform(Transform::identity().scale(1.0 + LIFT * v.current)))
		})
		.await;
	// Where a drag holds the knob, while one does.
	let mut dragged: Option<f32> = None;
	let mut grabbed_at = 0.0f32;
	// What was last drawn: the slider's left edge and the lit stop.
	let mut drawn_left = None;
	let mut lit = chosen;
	// The slider stands centered in the room the drawers leave, which follows the window and the drawers.
	let mut room = ctx.proactive(|shell: &Shell| (shell.viewport, shell.drawer_shown, shell.drawers));
	loop {
		// While dragged, the level is the stop under the knob; otherwise the one it glides to, so the level changes once,
		// when the knob is let go, rather than at each stop it passes on the way.
		let target = motion.target().await;
		let stop = nearest_stop(dragged.unwrap_or(target), count);
		let level = stop_level(stop);
		let (viewport, room_now) = ctx
			.with(move |shell: &mut Shell| {
				shell.detail_level = level;
				(
					shell.viewport,
					open_room(shell.viewport.x(), shell.drawer_shown, shell.drawers),
				)
			})
			.await;
		let left = ((room_now[0] + room_now[1] - width) * 0.5).clamp(0.0, (viewport.x() - width).max(0.0));
		if drawn_left != Some(left) {
			drawn_left = Some(left);
			pill.update_container(|c| c.absolute_position(left, TOP)).await;
		}
		if lit != stop {
			lit = stop;
			for (index, text) in labels.iter_mut().enumerate() {
				let color = if index == stop { TEXT } else { MUTED };
				text.update_text(|t| t.style([solid(color)])).await;
			}
		}
		select! {
			_ = room.next() => {},
			_ = pill.on(Events::Grabbed) => {
				let pointer = ctx.pointer().await.position;
				let corner = pill.geometry().await.map(|geometry| geometry.logical).map_or(0.0, |geometry| geometry.x());
				grabbed_at = pointer.x - corner;
				let at = motion.current().await;
				// A press on the knob holds it where it is; a press elsewhere glides it to the nearest stop.
				if (grabbed_at - at).abs() > KNOB * 0.5 {
					motion.set_target(stop_x(nearest_stop(grabbed_at, count))).await;
				}
				lift.set_target(1.0).await;
			},
			event = pill.on(Events::Dragged) => {
				if let Some(delta) = event.delta {
					let x = (grabbed_at + delta.x).clamp(first, last);
					dragged = Some(x);
					motion.snap(x).await;
				}
			},
			_ = pill.on(Events::DragEnded) => {
				if let Some(x) = dragged.take() {
					motion.snap(x).await;
					motion.set_target(stop_x(nearest_stop(x, count))).await;
				}
				lift.set_target(0.0).await;
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_place_snaps_to_the_nearest_stop_within_the_slider() {
		assert_eq!(nearest_stop(stop_x(2) + STOP_SPACING * 0.4, 4), 2);
		assert_eq!(nearest_stop(stop_x(2) + STOP_SPACING * 0.6, 4), 3);
		assert_eq!(nearest_stop(-100.0, 4), 0);
		assert_eq!(nearest_stop(1000.0, 4), 3);
	}
}
