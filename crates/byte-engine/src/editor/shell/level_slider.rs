//! The detail level slider a mesh window shows at its top, centered over the room between its drawers: a domed knob in
//! a channel, over one stop for **Auto** and one for each of the mesh's detail levels.
//!
//! The knob follows a drag, and the mesh changes level as it passes each stop; let go, or click a stop, and it springs
//! to the nearest one, overshooting a little and settling, lifted while held. The slider only reports the level, through
//! [`Shell::detail_level`]; the editor gives it to the stage, which draws the mesh at it. See
//! [`crate::editor::preview::Stage::show_level`].

use std::time::Instant;

use super::{
	ACCENT_RGB, CORNER, MUTED, SQUIRCLE, Shell, TEXT, cap, channel, darker, hex, open_room, panel, solid, top_highlight,
	vertical,
};
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

/// The spring's damping ratio: under one, it overshoots once, by about 16 % at one half, and settles.
const DAMPING: f32 = 0.5;
/// The spring's natural frequency, in radians a second, which sets how fast it moves.
const FREQUENCY: f32 = 20.0;
/// How long the spring takes to settle within 0.2 % of its end, in seconds, after which it stands there.
const SETTLE: f32 = 0.62;

/// How far a sprung move from 0 to 1 stands `seconds` after it started: the step response of an underdamped spring,
/// which overshoots its end once and settles on it.
fn spring(seconds: f32) -> f32 {
	if seconds >= SETTLE {
		return 1.0;
	}
	let seconds = seconds.max(0.0);
	let decay = DAMPING * FREQUENCY;
	let damped = FREQUENCY * (1.0 - DAMPING * DAMPING).sqrt();
	1.0 - (-decay * seconds).exp() * ((damped * seconds).cos() + decay / damped * (damped * seconds).sin())
}

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

/// The `Motion` struct is a sprung move from one place to another, started at an instant, so a view can ask where it
/// stands each frame.
#[derive(Clone, Copy)]
struct Motion {
	from: f32,
	to: f32,
	started: Instant,
}

impl Motion {
	/// A motion that stands at `at`.
	fn at(at: f32) -> Self {
		Self { from: at, to: at, started: Instant::now() }
	}

	/// Where the motion stands now.
	fn now(&self) -> f32 {
		self.from + (self.to - self.from) * spring(self.started.elapsed().as_secs_f32())
	}

	/// A motion toward `to` from wherever this one stands now, unless it already heads there.
	fn toward(self, to: f32) -> Self {
		if self.to == to { self } else { Self { from: self.now(), to, started: Instant::now() } }
	}
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
				.style(panel(HEIGHT, 0.85))
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
				.style(channel(TRACK_HEIGHT, 0.0))
		})
		.await;
	// The accent runs from the first stop to the knob, so a pinned level reads as filled in.
	let mut fill = track
		.element("fill")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(TRACK_HEIGHT.into())
				.height(TRACK_HEIGHT.into())
				.corner_radius(TRACK_HEIGHT * 0.5)
				.style([
					solid(hex(0x000000, 0.0)),
					ConcreteLayer::default().color(vertical(
						TRACK_HEIGHT,
						hex(darker(ACCENT_RGB), 1.0),
						hex(ACCENT_RGB, 1.0),
					)),
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
		let text = if index == 0 { "Auto".to_owned() } else { (index - 1).to_string() };
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
				.text(&text, |t| t.font_size(11.0).style([solid(if index == chosen { TEXT } else { MUTED })]))
				.await,
		);
	}
	let mut knob = pill
		.element("knob")
		.container(|c| {
			c.absolute_position(stop_x(chosen) - KNOB * 0.5, center_y - KNOB * 0.5)
				.width(KNOB.into())
				.height(KNOB.into())
				.corner_radius(KNOB * 0.5)
				.style(cap(KNOB))
		})
		.await;

	// Where the knob stands, sprung toward a stop; where a drag holds it, while one does; and how far it's lifted.
	let mut motion = Motion::at(stop_x(chosen));
	let mut dragged: Option<f32> = None;
	let mut grabbed_at = 0.0f32;
	let mut lift = Motion::at(0.0);
	// What was last drawn: the slider's left edge, the knob's place and lift, and the lit stop.
	let mut drawn: Option<(f32, f32, f32, usize)> = None;
	loop {
		select! {
			_ = pill.on(Events::Grabbed) => {
				let pointer = ctx.pointer().await.position;
				let corner = pill.geometry().await.map_or(0.0, |geometry| geometry.x());
				grabbed_at = pointer.x - corner;
				let at = motion.now();
				// A press on the knob holds it where it is; a press elsewhere springs it to the nearest stop.
				if (grabbed_at - at).abs() > KNOB * 0.5 {
					motion = motion.toward(stop_x(nearest_stop(grabbed_at, count)));
				}
				lift = lift.toward(1.0);
			},
			event = pill.on(Events::Dragged) => {
				if let Some(delta) = event.delta {
					let x = (grabbed_at + delta.x).clamp(first, last);
					dragged = Some(x);
					motion = Motion::at(x);
				}
			},
			_ = pill.on(Events::DragEnded) => {
				if let Some(x) = dragged.take() {
					motion = Motion::at(x).toward(stop_x(nearest_stop(x, count)));
				}
				lift = lift.toward(0.0);
			},
			_ = ctx.render() => {},
		}
		// While dragged, the level is the stop under the knob; otherwise the one it springs to, so an overshoot past
		// half a stop never flickers to the next level.
		let at = motion.now();
		let stop = nearest_stop(dragged.unwrap_or(motion.to), count);
		let level = stop_level(stop);
		let (viewport, room) = ctx
			.with(move |shell: &mut Shell| {
				shell.detail_level = level;
				(shell.viewport, open_room(shell.viewport.x(), shell.drawer_shown, shell.drawers))
			})
			.await;
		let left = ((room[0] + room[1] - width) * 0.5).clamp(0.0, (viewport.x() - width).max(0.0));
		let lifted = lift.now();
		let now = (left, at, lifted, stop);
		if drawn == Some(now) {
			continue;
		}
		if drawn.is_none_or(|(drawn_left, ..)| drawn_left != left) {
			pill.update_container(|c| c.absolute_position(left, TOP)).await;
		}
		knob.update_container(|c| {
			c.absolute_position(at - KNOB * 0.5, center_y - KNOB * 0.5)
				.transform(Transform::identity().scale(1.0 + LIFT * lifted))
		})
		.await;
		fill.update_container(|c| c.width((at - first + TRACK_HEIGHT).into())).await;
		if drawn.is_none_or(|(.., lit)| lit != stop) {
			for (index, text) in labels.iter_mut().enumerate() {
				let color = if index == stop { TEXT } else { MUTED };
				text.update_text(|t| t.style([solid(color)])).await;
			}
		}
		// Once the springs settle, `now` stops changing, and the slider stops redrawing until something moves it.
		drawn = Some(now);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_spring_overshoots_once_and_settles_on_its_end() {
		assert_eq!(spring(0.0), 0.0);
		let samples: Vec<f32> = (0..=100).map(|step| spring(SETTLE * step as f32 / 100.0)).collect();
		let peak = samples.iter().copied().fold(0.0, f32::max);
		assert!(peak > 1.1 && peak < 1.2, "it overshoots by about 16 %: {peak}");
		assert!((spring(SETTLE - 0.001) - 1.0).abs() < 0.01, "it has settled before it snaps in place");
		assert_eq!(spring(SETTLE), 1.0);
	}

	#[test]
	fn a_place_snaps_to_the_nearest_stop_within_the_slider() {
		assert_eq!(nearest_stop(stop_x(2) + STOP_SPACING * 0.4, 4), 2);
		assert_eq!(nearest_stop(stop_x(2) + STOP_SPACING * 0.6, 4), 3);
		assert_eq!(nearest_stop(-100.0, 4), 0);
		assert_eq!(nearest_stop(1000.0, 4), 3);
	}

	#[test]
	fn the_first_stop_selects_by_distance_and_the_rest_name_levels() {
		assert_eq!(stop_level(0), None);
		assert_eq!(stop_level(1), Some(0));
		assert_eq!(stop_level(3), Some(2));
	}
}
