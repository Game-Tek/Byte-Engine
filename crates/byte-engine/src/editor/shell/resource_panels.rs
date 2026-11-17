//! The two layers every preview window lays out: the base, which fills the whole window with what the window shows,
//! and the drawer, which runs the window's whole height against its right edge, as the scene drawer does against the
//! main window's left, and holds the resource's name and settings.
//!
//! The drawer slides away while what the base shows needs the window's whole width, and comes back once it fits the
//! room the drawer leaves; [`fits`] decides, [`Shell::drawer_open`] carries the decision, and [`Drawer::follow`] moves
//! the drawer by it. What the base shows stays centered in the open room, [`open_center`].

use std::time::Instant;

use super::{CORNER, DRAWER_TUCK, INSET, MUTED, PROPERTIES_WIDTH, SQUIRCLE, Shell, TEXT, hex, panel, sidebar_height, solid};
use crate::editor::drawer;
use crate::ui::flow;
use crate::ui::layout::context::{Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::{Anchor, Position, Transform};

/// How far right the drawer moves to hide, its shadow included.
const DRAWER_HIDDEN: f32 = PROPERTIES_WIDTH + 32.0;
/// The height of the drawer's title row.
const TITLE_HEIGHT: f32 = 22.0;

/// Whether content `width` pixels wide fits beside the drawer in a window `window` pixels wide, so the drawer can show.
pub(crate) fn fits(width: f32, window: f32) -> bool {
	width <= window - PROPERTIES_WIDTH
}

/// The middle of the room a drawer showing `shown`, from 0 hidden to 1 in place, leaves in a window `window` pixels
/// wide, from the window's left edge, where what the base shows is centered.
pub(crate) fn open_center(window: f32, shown: f32) -> f32 {
	(window - shown.clamp(0.0, 1.0) * PROPERTIES_WIDTH) * 0.5
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

/// The `Drawer` struct is a preview window's right-hand panel, declared over the base, with the motion that moves it in
/// and out, so a view can keep it following [`Shell::drawer_open`] while it shows.
pub(super) struct Drawer {
	frame: EvaluationContext<Shell>,
	/// The column the view fills in, under the title.
	pub(super) content: EvaluationContext<Shell>,
	/// The column's width.
	pub(super) width: f32,
	/// How much showed when the drawer started its current move, which way it moves, and when it started.
	from: f32,
	open: bool,
	moved: Instant,
	settled: bool,
}

impl Drawer {
	/// Declares the drawer titled `name`, with `detail` beside it in muted text, where the last drawer of the window
	/// left off, so one declared again as the window resizes doesn't move.
	pub(super) async fn declare(ctx: &mut EvaluationContext<Shell>, name: &str, detail: &str) -> Self {
		let (viewport, shown, open) = ctx
			.with(|shell: &mut Shell| (shell.viewport, shell.drawer_shown, shell.drawer_open))
			.await;
		let width = PROPERTIES_WIDTH - INSET * 2.0;
		let height = sidebar_height(viewport);
		// The panel runs past the window's right edge, so only the corners facing the open room show rounded.
		let mut frame = ctx
			.element("drawer")
			.container(|c| {
				c.position(Position::anchored(Anchor::End(-DRAWER_TUCK), Anchor::Start(0.0)))
					.width((PROPERTIES_WIDTH + DRAWER_TUCK).into())
					.height(height.into())
					.transform(drawer_transform(shown))
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
				c.absolute_position(INSET, INSET)
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
			content,
			width,
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
		self.frame.update_container(|c| c.transform(drawer_transform(shown))).await;
		ctx.with(move |shell: &mut Shell| shell.drawer_shown = shown).await;
	}

	/// Keeps the drawer following [`Shell::drawer_open`] while `showing` says the window shows what it was declared for
	/// and the window keeps its size, then returns, so the view declares its layers again or for what shows next.
	pub(super) async fn hold(&mut self, ctx: &mut EvaluationContext<Shell>, showing: impl Fn(&Shell) -> bool) {
		let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
		loop {
			ctx.render().await;
			self.follow(ctx).await;
			if !ctx.with(|shell: &mut Shell| showing(shell) && shell.viewport == viewport).await {
				return;
			}
		}
	}

	pub(super) async fn remove(mut self) {
		self.frame.remove().await;
	}
}

/// The transform that shows `shown` of the drawer, from 0, hidden past the window's right edge, to 1, in place.
fn drawer_transform(shown: f32) -> Transform {
	Transform::identity().translate_x((1.0 - shown) * DRAWER_HIDDEN)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn content_fits_while_it_leaves_the_drawer_room() {
		assert!(fits(800.0 - PROPERTIES_WIDTH, 800.0));
		assert!(!fits(801.0 - PROPERTIES_WIDTH, 800.0));
	}

	#[test]
	fn the_open_room_is_centered_left_of_the_drawer_and_widens_as_it_leaves() {
		assert_eq!(open_center(1000.0, 1.0), (1000.0 - PROPERTIES_WIDTH) * 0.5);
		assert_eq!(open_center(1000.0, 0.0), 500.0);
		// The drawer overshoots a little as it springs in; the room doesn't.
		assert_eq!(open_center(1000.0, 1.03), open_center(1000.0, 1.0));
	}
}
