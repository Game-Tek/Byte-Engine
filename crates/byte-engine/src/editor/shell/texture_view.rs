//! The texture preview: while a texture is open from the explorer, a panel over the game view that shows it, scaled
//! down to fit, with its name and its size in pixels.
//!
//! The panel is declared each time a texture opens and removed when it closes, by its ×, by Escape, or by another
//! resource opening in its place.

use super::{
	CORNER, MARGIN, MUTED, PROPERTIES_WIDTH, SIDEBAR_WIDTH, SQUIRCLE, Shell, TEXT, TOOLBAR_HEIGHT, hex, panel, row_style,
	solid,
};
use crate::editor::layers::Layer;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::ui::flow;
use crate::utils::r#async::select;

/// The height of the panel's title bar, over the picture.
const TITLE_HEIGHT: f32 = 36.0;
/// The gap between the panel's edge and the picture.
const INSET: f32 = 10.0;

/// Shows the open texture's panel while a texture is open, declaring it again for each texture that opens.
pub(super) async fn texture_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		while ctx.with(|shell: &mut Shell| shell.layers.texture().is_none()).await {
			ctx.render().await;
		}
		let mut frame = texture_panel(ctx).await;
		frame.remove().await;
	}
}

/// The size, in pixels, a `width` by `height` picture shows at to fit in `room`: its own size, or smaller, keeping its
/// aspect.
fn fitted(width: u32, height: u32, room: [f32; 2]) -> [f32; 2] {
	let (width, height) = (width.max(1) as f32, height.max(1) as f32);
	let scale = (room[0] / width).min(room[1] / height).min(1.0);
	[width * scale, height * scale]
}

/// Declares the open texture's panel, centered over the game view, and keeps it until the texture closes or another
/// opens in its place, then returns it for removal.
async fn texture_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	// The room between the scene list, the toolbar, and the properties panel, as the graph canvas takes.
	let left = MARGIN * 2.0 + SIDEBAR_WIDTH;
	let top = MARGIN * 2.0 + TOOLBAR_HEIGHT;
	let room = [
		(viewport.x() - left - MARGIN * 2.0 - PROPERTIES_WIDTH).max(200.0),
		(viewport.y() - top - MARGIN).max(TITLE_HEIGHT + 100.0),
	];

	// The picture's pixels are copied into the UI's storage once, as the panel is declared.
	let (id, name, size, picture) = ctx
		.with(|shell: &mut Shell| {
			let texture = shell
				.layers
				.texture()
				.expect("The texture panel shows only while a texture is open.");
			let preview = &texture.preview;
			let name = texture.id.rsplit('/').next().unwrap_or(&texture.id).to_owned();
			let size = format!("{} × {}", preview.source[0], preview.source[1]);
			(texture.id.clone(), name, size, (preview.width, preview.height, preview.pixels.clone()))
		})
		.await;
	let shown = fitted(picture.0, picture.1, [room[0] - INSET * 2.0, room[1] - TITLE_HEIGHT - INSET]);
	// The panel hugs the picture, but stays wide enough for its title.
	let width = (shown[0] + INSET * 2.0).max(240.0);
	let height = shown[1] + TITLE_HEIGHT + INSET;

	let mut frame = ctx
		.element("frame")
		.container(|c| {
			c.absolute_position(left + (room[0] - width) * 0.5, top + (room[1] - height) * 0.5)
				.width(width.into())
				.height(height.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
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
	bar.element("name").text(&name, |t| t.font_size(13.0).style([solid(TEXT)])).await;
	bar.element("size").text(&size, |t| t.font_size(11.0).style([solid(MUTED)])).await;
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
	// A dark mat under the picture, so its transparent parts read as such.
	let mut mat = frame
		.element("mat")
		.container(|c| {
			c.absolute_position((width - shown[0]) * 0.5, TITLE_HEIGHT)
				.width(shown[0].into())
				.height(shown[1].into())
				.style([solid(hex(0x141518, 1.0))])
		})
		.await;
	let (picture_width, picture_height, pixels) = picture;
	mat.element("picture")
		.image(picture_width, picture_height, pixels, |i| {
			i.width(shown[0].into()).height(shown[1].into())
		})
		.await;

	let mut close_hovered = false;
	loop {
		select! {
			_ = close.on(Events::Actuated) => {
				ctx.with(|shell: &mut Shell| shell.layers.close(|layer| matches!(layer, Layer::Texture(_)))).await;
			},
			_ = close.on(Events::PointerEntered) => close_hovered = true,
			_ = close.on(Events::PointerExited) => close_hovered = false,
			_ = ctx.render() => {},
		}
		close.update_container(|c| c.style(row_style(false, close_hovered))).await;
		let open = ctx
			.with(|shell: &mut Shell| shell.layers.texture().is_some_and(|texture| texture.id == id))
			.await;
		if !open {
			break;
		}
	}
	frame
}

#[cfg(test)]
mod tests {
	use super::fitted;

	#[test]
	fn a_picture_shows_at_its_own_size_when_it_fits_and_scales_down_keeping_its_aspect_otherwise() {
		assert_eq!(fitted(100, 50, [400.0, 400.0]), [100.0, 50.0]);
		assert_eq!(fitted(800, 400, [400.0, 400.0]), [400.0, 200.0]);
		assert_eq!(fitted(400, 800, [400.0, 400.0]), [200.0, 400.0]);
	}
}
