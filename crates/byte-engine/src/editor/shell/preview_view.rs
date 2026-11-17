//! The preview a resource window's main panel shows under its title bar: the stage's picture of the window's mesh or
//! material, which a drag turns as an arcball and a scroll moves closer.
//!
//! The panel only reports the drag and the scroll, through [`Shell::turn`] and [`Shell::zoom`]; the editor turns the
//! subject, moves the camera, and hands the panel the image it draws into through [`Shell::preview`]. See
//! [`crate::editor::preview`].

use super::resource_panels::TITLE_HEIGHT;
use super::{Shell, hex, solid};
use crate::ui::flow;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::utils::r#async::select;

/// Declares the preview in `frame`, the main panel, filling `room` under its title bar.
pub(super) async fn preview(frame: &mut EvaluationContext<Shell>, room: [u32; 2]) {
	frame
		.element("preview")
		.component(async move |ctx| picture(ctx, room).await)
		.await;
}

/// A dark mat under the title bar, which shows the picture once the editor names its image, and reports every drag and
/// scroll over it to the editor.
async fn picture(ctx: &mut EvaluationContext<Shell>, room: [u32; 2]) {
	let [width, height] = room.map(|side| side as f32);
	let mut mat = ctx
		.element("mat")
		.container(|c| {
			c.absolute_position(0.0, TITLE_HEIGHT)
				.width(width.into())
				.height(height.into())
				.flow(flow::center)
				.hit_testable(true)
				.style([solid(hex(0x141518, 1.0))])
		})
		.await;
	let image = loop {
		if let Some(image) = ctx.with(|shell: &mut Shell| shell.preview).await {
			break image;
		}
		ctx.render().await;
	};
	mat.element("image").image(room[0], room[1], image, |i| i).await;
	// Where the press that started the drag landed, in the mat's pixels, and the arcball point last reported.
	let mut pressed = [0.0f32; 2];
	let mut reached = [0.0f32; 2];
	loop {
		select! {
			_ = mat.on(Events::Grabbed) => {
				let pointer = ctx.pointer().await.position;
				let corner = mat.geometry().await.map_or([0.0; 2], |geometry| [geometry.x(), geometry.y()]);
				pressed = [pointer.x - corner[0], pointer.y - corner[1]];
				reached = on_ball(pressed, [width, height]);
			},
			event = mat.on(Events::Dragged) => {
				if let Some(delta) = event.delta {
					let to = on_ball([pressed[0] + delta.x, pressed[1] + delta.y], [width, height]);
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

/// The arcball coordinates of `pixel` in a picture of `size` pixels: the ball sits in the picture's middle, as wide as
/// its shorter side, with +x right and +y up, and its radius is 1.
fn on_ball(pixel: [f32; 2], [width, height]: [f32; 2]) -> [f32; 2] {
	let radius = (width.min(height) * 0.5).max(1.0);
	[(pixel[0] - width * 0.5) / radius, (height * 0.5 - pixel[1]) / radius]
}

#[cfg(test)]
mod tests {
	use super::on_ball;

	#[test]
	fn the_ball_fills_the_shorter_side_of_the_picture() {
		assert_eq!(on_ball([400.0, 300.0], [800.0, 600.0]), [0.0, 0.0]);
		assert_eq!(on_ball([700.0, 0.0], [800.0, 600.0]), [1.0, 1.0]);
		assert_eq!(on_ball([100.0, 600.0], [800.0, 600.0]), [-1.0, -1.0]);
	}
}
