//! The layers every preview window lays out: the base, which fills the whole window with what the window shows, and
//! its drawers, which run the window's whole height against its edges, as the scene drawer does against the main
//! window's left. The right drawer holds the resource's name and settings; a window that lists more, such as a mesh's
//! primitives, adds a left one.
//!
//! The drawers slide away together while what the base shows needs the window's whole width, and come back once it
//! fits the room between them; [`fits`] decides, [`Shell::drawer_open`] carries the decision, and [`hold`] moves the
//! drawers by it. What the base shows stays centered in the open room, [`open_room`].

use std::time::Instant;

use super::{
	CORNER, DRAWER_TUCK, INSET, MUTED, PROPERTIES_WIDTH, SIDEBAR_WIDTH, SQUIRCLE, Shell, TEXT, hex, panel, sidebar_height,
	solid,
};
use crate::editor::drawer;
use crate::ui::flow;
use crate::ui::layout::context::{Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::{Anchor, Position, Transform};

/// How much further than its width a drawer moves to hide, so its shadow hides too.
const SHADOW_ROOM: f32 = 32.0;
/// The height of the drawer's title row.
const TITLE_HEIGHT: f32 = 22.0;

/// The `Side` enum is the window edge a drawer runs against, which indexes [`Shell::drawers`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
	Left = 0,
	Right = 1,
}

impl Side {
	/// How wide the side's drawer is: the left as wide as the scene drawer, the right as the properties panel.
	const fn width(self) -> f32 {
		match self {
			Self::Left => SIDEBAR_WIDTH,
			Self::Right => PROPERTIES_WIDTH,
		}
	}
}

/// Whether content `width` pixels wide fits between `drawers`, the widths of a window's left and right drawers, 0 for
/// one it doesn't have, in a window `window` pixels wide, so the drawers can show.
pub(crate) fn fits(width: f32, window: f32, drawers: [f32; 2]) -> bool {
	width <= window - drawers[0] - drawers[1]
}

/// The room `drawers` showing `shown`, from 0 hidden to 1 in place, leave in a window `window` pixels wide, as its
/// left and right edges from the window's left edge. What the base shows is centered in it.
pub(crate) fn open_room(window: f32, shown: f32, drawers: [f32; 2]) -> [f32; 2] {
	// A drawer overshoots a little as it springs in; the room doesn't.
	let shown = shown.clamp(0.0, 1.0);
	[drawers[0] * shown, window - drawers[1] * shown]
}

/// Declares the base: a dark mat under the whole window, so transparent parts of what it shows read as such, and around
/// it. It takes presses, so a drag or a scroll over it reaches the view that declared it.
pub(super) async fn base(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	ctx.element("base")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(viewport.x().into())
				.height(viewport.y().into())
				.clip(true)
				.hit_testable(true)
				.style([solid(hex(0x141518, 1.0))])
		})
		.await
}

/// The `Drawer` struct is a preview window's side panel, declared over the base, with the motion that moves it in and
/// out, so a view can keep it following [`Shell::drawer_open`] while it shows, with [`hold`].
pub(super) struct Drawer {
	frame: EvaluationContext<Shell>,
	side: Side,
	/// The column the view fills in, under the title.
	pub(super) content: EvaluationContext<Shell>,
	/// The column's width.
	pub(super) width: f32,
	/// The column's height under the title, for a view that fits what it lists to it.
	pub(super) room: f32,
	/// How much showed when the drawer started its current move, which way it moves, and when it started.
	from: f32,
	open: bool,
	moved: Instant,
	settled: bool,
}

impl Drawer {
	/// Declares the drawer against the window's `side`, titled `name`, with `detail` beside it in muted text, where the
	/// last drawers of the window left off, so one declared again as the window resizes doesn't move. It records its
	/// width in [`Shell::drawers`], which the room between the drawers is measured by.
	pub(super) async fn declare(ctx: &mut EvaluationContext<Shell>, side: Side, name: &str, detail: &str) -> Self {
		let (viewport, shown, open) = ctx
			.with(move |shell: &mut Shell| {
				shell.drawers[side as usize] = side.width();
				(shell.viewport, shell.drawer_shown, shell.drawer_open)
			})
			.await;
		let width = side.width() - INSET * 2.0;
		let height = sidebar_height(viewport);
		// The panel runs past the window's edge, so only the corners facing the open room show rounded, and its content
		// starts past the part tucked behind a left edge.
		let (anchor, lead) = match side {
			Side::Left => (Anchor::Start(-DRAWER_TUCK), DRAWER_TUCK),
			Side::Right => (Anchor::End(-DRAWER_TUCK), 0.0),
		};
		let mut frame = ctx
			.element(("drawer", side as usize))
			.container(|c| {
				c.position(Position::anchored(anchor, Anchor::Start(0.0)))
					.width((side.width() + DRAWER_TUCK).into())
					.height(height.into())
					.transform(drawer_transform(side, shown))
					.corner_radius(CORNER)
					.corner_exponent(SQUIRCLE)
					.clip(false)
					// The panel takes presses, so a click on it never reaches the base behind it.
					.hit_testable(true)
					.style(panel(height, 1.0))
			})
			.await;
		let mut content = frame
			.element("content")
			.container(|c| {
				c.absolute_position(lead + INSET, INSET)
					.width(width.into())
					.height((height - INSET * 2.0).into())
					.flow(flow::column_with_gap(6))
			})
			.await;
		let mut title = content
			.element("title")
			.container(|c| c.width(width.into()).height(TITLE_HEIGHT.into()).flow(flow::centered_row_with_gap(8)))
			.await;
		title.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
		title.element("name")
			.text(name, |t| t.font_size(13.0).style([solid(TEXT)]))
			.await;
		title.element("detail")
			.text(detail, |t| t.font_size(11.0).style([solid(MUTED)]))
			.await;
		Self {
			frame,
			side,
			content,
			width,
			// The column's gap separates the title from what's under it.
			room: height - INSET * 2.0 - TITLE_HEIGHT - 6.0,
			from: shown,
			open,
			moved: Instant::now(),
			settled: shown == if open { 1.0 } else { 0.0 },
		}
	}

	/// Moves the drawer toward where [`Shell::drawer_open`] wants it, from wherever it is, and reports how much of it
	/// shows in [`Shell::drawer_shown`], which centers what the base shows. Call it every frame the drawer shows.
	pub(super) async fn follow(&mut self, ctx: &mut EvaluationContext<Shell>) {
		let (open, shown) = ctx.with(|shell: &mut Shell| (shell.drawer_open, shell.drawer_shown)).await;
		if open != self.open {
			self.from = shown;
			self.open = open;
			self.moved = Instant::now();
			self.settled = false;
		}
		if self.settled {
			return;
		}
		let seconds = self.moved.elapsed().as_secs_f32();
		let shown = drawer::toward(self.from, open, seconds);
		self.settled = drawer::settled(open, seconds);
		let side = self.side;
		self.frame.update_container(|c| c.transform(drawer_transform(side, shown))).await;
		ctx.with(move |shell: &mut Shell| shell.drawer_shown = shown).await;
	}

	pub(super) async fn remove(mut self) {
		self.frame.remove().await;
	}
}

/// Keeps `drawers` following [`Shell::drawer_open`] while `showing` says the window shows what they were declared for
/// and the window keeps its size, then returns, so the view declares its layers again or for what shows next.
pub(super) async fn hold(ctx: &mut EvaluationContext<Shell>, drawers: &mut [Drawer], showing: impl Fn(&Shell) -> bool) {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	loop {
		ctx.render().await;
		for drawer in drawers.iter_mut() {
			drawer.follow(ctx).await;
		}
		if !ctx.with(|shell: &mut Shell| showing(shell) && shell.viewport == viewport).await {
			return;
		}
	}
}

/// The transform that shows `shown` of the drawer on `side`, from 0, hidden past the window's edge, to 1, in place.
fn drawer_transform(side: Side, shown: f32) -> Transform {
	let hidden = (1.0 - shown) * (side.width() + SHADOW_ROOM);
	Transform::identity().translate_x(match side {
		Side::Left => -hidden,
		Side::Right => hidden,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn content_fits_while_it_leaves_the_drawers_room() {
		assert!(fits(500.0, 800.0, [0.0, 300.0]));
		assert!(!fits(501.0, 800.0, [0.0, 300.0]));
		assert!(fits(240.0, 800.0, [260.0, 300.0]));
		assert!(!fits(241.0, 800.0, [260.0, 300.0]));
	}

	#[test]
	fn the_open_room_lies_between_the_drawers_and_widens_as_they_leave() {
		assert_eq!(open_room(1000.0, 1.0, [0.0, 300.0]), [0.0, 700.0]);
		assert_eq!(open_room(1000.0, 1.0, [260.0, 300.0]), [260.0, 700.0]);
		assert_eq!(open_room(1000.0, 0.5, [260.0, 300.0]), [130.0, 850.0]);
		assert_eq!(open_room(1000.0, 0.0, [260.0, 300.0]), [0.0, 1000.0]);
		// The drawers overshoot a little as they spring in; the room doesn't.
		assert_eq!(open_room(1000.0, 1.03, [260.0, 300.0]), open_room(1000.0, 1.0, [260.0, 300.0]));
	}
}
