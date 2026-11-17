//! The preview a resource window's base shows: the stage's picture of the window's mesh or material, filling the
//! window, which a drag turns as an arcball and a scroll moves closer.
//!
//! The panel only reports the drag and the scroll, through [`Shell::turn`] and [`Shell::zoom`]; the editor turns the
//! subject, moves the camera, and hands the panel the image it draws into through [`Shell::preview`]. See
//! [`crate::editor::preview`].

use super::{Shell, open_center};
use crate::ui::flow;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::utils::r#async::select;

/// Declares the preview in `base`, filling the window.
pub(super) async fn preview(base: &mut EvaluationContext<Shell>) {
	base.element("preview").component(async move |ctx| picture(ctx).await).await;
}

/// A surface over the whole window, which shows the picture once the editor names its image, and reports every drag
/// and scroll over it to the editor. The editor draws the picture as large as the window.
async fn picture(ctx: &mut EvaluationContext<Shell>) {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	let (width, height) = (viewport.x(), viewport.y());
	let room = [width as u32, height as u32];
	let mut mat = ctx
		.element("surface")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(width.into())
				.height(height.into())
				.flow(flow::center)
				.hit_testable(true)
		})
		.await;
	let image = loop {
		if let Some(image) = ctx.with(|shell: &mut Shell| shell.preview).await {
			break image;
		}
		ctx.render().await;
	};
	mat.element("image").image(room[0], room[1], image, |i| i).await;
	// Where the press that started the drag landed, in the surface's pixels, the ball it turns, and the arcball point
	// last reported.
	let mut pressed = [0.0f32; 2];
	let mut ball = Ball::new([width * 0.5, height * 0.5], [width, height]);
	let mut reached = [0.0f32; 2];
	loop {
		select! {
			_ = mat.on(Events::Grabbed) => {
				let pointer = ctx.pointer().await.position;
				let corner = mat.geometry().await.map_or([0.0; 2], |geometry| [geometry.x(), geometry.y()]);
				pressed = [pointer.x - corner[0], pointer.y - corner[1]];
				// The ball sits on the subject, in the middle of the room the drawer leaves.
				let shown = ctx.with(|shell: &mut Shell| shell.drawer_shown).await;
				let center = open_center(width, shown);
				ball = Ball::new([center, height * 0.5], [center * 2.0, height]);
				reached = ball.point(pressed);
			},
			event = mat.on(Events::Dragged) => {
				if let Some(delta) = event.delta {
					let to = ball.point([pressed[0] + delta.x, pressed[1] + delta.y]);
					let from = std::mem::replace(&mut reached, to);
					// Drags the editor hasn't taken yet join this one, from where the first of them started.
					ctx.with(move |shell: &mut Shell| {
						shell.turn = Some((shell.turn.map_or(from, |(start, _)| start), to));
					})
					.await;
				}
			},
			event = mat.on(Events::Scrolled) => {
				// One step per scroll, as the graph canvas zooms, so a trackpad and a wheel move alike.
				if let Some(delta) = event.delta.filter(|delta| delta.y != 0.0) {
					ctx.with(move |shell: &mut Shell| shell.zoom += delta.y.signum()).await;
				}
			},
			_ = ctx.render() => {},
		}
	}
}

/// The `Ball` struct is where the arcball sits on the picture, so a drag's pixels become arcball coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ball {
	/// The ball's middle, in the picture's pixels.
	center: [f32; 2],
	/// The ball's radius, in pixels.
	radius: f32,
}

impl Ball {
	/// The ball in the middle of `room`, a `[width, height]` in pixels, centered at `center`, as wide as the room's
	/// shorter side.
	fn new(center: [f32; 2], room: [f32; 2]) -> Self {
		Self {
			center,
			radius: (room[0].min(room[1]) * 0.5).max(1.0),
		}
	}

	/// The arcball coordinates of `pixel`: +x right and +y up from the ball's middle, in radii.
	fn point(&self, pixel: [f32; 2]) -> [f32; 2] {
		[(pixel[0] - self.center[0]) / self.radius, (self.center[1] - pixel[1]) / self.radius]
	}
}

#[cfg(test)]
mod tests {
	use super::Ball;

	#[test]
	fn the_ball_fills_the_shorter_side_of_its_room() {
		let ball = Ball::new([400.0, 300.0], [800.0, 600.0]);
		assert_eq!(ball.point([400.0, 300.0]), [0.0, 0.0]);
		assert_eq!(ball.point([700.0, 0.0]), [1.0, 1.0]);
		assert_eq!(ball.point([100.0, 600.0]), [-1.0, -1.0]);
	}
}
