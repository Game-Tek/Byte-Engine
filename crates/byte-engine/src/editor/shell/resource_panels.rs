//! The two panels every resource window lays out: the main panel, which fills the window left of the docked one under
//! a title bar, and the docked panel, which runs the window's whole height against its right edge, as the scene
//! drawer does against the main window's left.

use super::{
	CORNER, DRAWER_TUCK, INSET, MUTED, PROPERTIES_WIDTH, SQUIRCLE, Shell, TEXT, panel, row_style, sidebar_height, solid,
};
use crate::ui::flow;
use crate::ui::layout::context::{Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::{Anchor, Position};

/// The height of the main panel's title bar.
pub(super) const TITLE_HEIGHT: f32 = 36.0;

/// Declares the main panel, titled `name` with `detail` beside it in muted text, and returns it with its close button
/// and its size. What it shows goes under the title bar, from `TITLE_HEIGHT` down.
pub(super) async fn main_panel(
	ctx: &mut EvaluationContext<Shell>,
	name: &str,
	detail: &str,
) -> (EvaluationContext<Shell>, EvaluationContext<Shell>, [f32; 2]) {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	let width = (viewport.x() - PROPERTIES_WIDTH).max(200.0);
	let height = viewport.y().max(TITLE_HEIGHT + 100.0);
	let mut frame = ctx
		.element("frame")
		.container(|c| {
			// The panel runs under the docked panel's rounded corners, so no window shows through them.
			c.absolute_position(0.0, 0.0)
				.width((width + CORNER).into())
				.height(height.into())
				.clip(false)
				.hit_testable(true)
				.style(panel(height, 1.0))
		})
		.await;
	let mut bar = frame
		.element("title")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(width.into())
				.height(TITLE_HEIGHT.into())
				.flow(flow::centered_row_with_gap(8))
		})
		.await;
	bar.element("lead").container(|c| c.width(6.into()).height(1.into())).await;
	bar.element("name").text(name, |t| t.font_size(13.0).style([solid(TEXT)])).await;
	bar.element("detail").text(detail, |t| t.font_size(11.0).style([solid(MUTED)])).await;
	let mut close = frame
		.element("close")
		.container(|c| {
			c.absolute_position(width - 32.0, (TITLE_HEIGHT - 24.0) * 0.5)
				.width(24.into())
				.height(24.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	close
		.element("cross")
		.text("×", |t| t.font_size(16.0).style([solid(MUTED)]))
		.await;
	(frame, close, [width, height])
}

/// Declares the docked panel and returns it with the column its content goes in, and that column's width.
pub(super) async fn docked_panel(ctx: &mut EvaluationContext<Shell>) -> (EvaluationContext<Shell>, EvaluationContext<Shell>, f32) {
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	let height = sidebar_height(ctx.with(|shell: &mut Shell| shell.viewport).await);
	// The panel runs past the window's right edge, so only the corners facing the main panel show rounded.
	let mut frame = ctx
		.element("properties")
		.container(|c| {
			c.position(Position::anchored(Anchor::End(-DRAWER_TUCK), Anchor::Start(0.0)))
				.width((PROPERTIES_WIDTH + DRAWER_TUCK).into())
				.height(height.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.hit_testable(true)
				.style(panel(height, 1.0))
		})
		.await;
	let content = frame
		.element("content")
		.container(|c| {
			c.absolute_position(INSET, INSET)
				.width(width.into())
				.height((height - INSET * 2.0).into())
				.flow(flow::column_with_gap(6))
		})
		.await;
	(frame, content, width)
}
