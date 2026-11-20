//! The gym's panel: a large panel over the scene that shows the gym the editor flies through, under a header that
//! names the gym and holds the Place button.
//!
//! The panel only shows; the editor flies the gym's camera, picks where a click lands, and hands the panel the image
//! the gym is drawn into through [`Shell::gym_image`]. The Place button stays off until something is picked, then turns
//! green and asks the editor to carry the pick into the scene through [`Shell::place_gym`], as Enter does. Its place
//! comes from [`gym::panel`] and [`gym::picture`], which the editor maps clicks with too. See [`crate::editor::gym`].

use super::{ACCENT, CORNER, MUTED, SQUIRCLE, Shell, TEXT, hairline, hex, panel, solid, top_highlight, vertical, well};
use crate::editor::gym;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::ui::{ConcreteLayer, flow};
use crate::utils::r#async::select;

/// The Place button's width, enough for a picked asset's name.
const PLACE_WIDTH: f32 = 200.0;
/// The text on Byte Engine's green, dark so it reads over the bright fill.
const ON_ACCENT: u32 = 0x0B2413;
/// Byte Engine's green in shade, where the Place button's fill ends at its bottom edge.
const ACCENT_SHADE: u32 = 0x2FA650;
/// The shade the fill ends at under the pointer, closer to the green, so the button brightens without passing it.
const ACCENT_SHADE_HOVERED: u32 = 0x39BF5C;

/// Shows the gym's panel while a gym is the top layer, declaring it again when the window resizes.
pub(super) async fn gym_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		// Waiting on the layer reads it only when the shell changed, so a closed view asks for no frames.
		let mut open = ctx.proactive(|shell: &Shell| shell.layers.gym().is_some());
		while !*open.next().await {}
		let mut frame = gym_panel(ctx).await;
		frame.remove().await;
	}
}

/// Declares the panel, its header, and its picture, and keeps the Place button and the picture current until the gym
/// closes or the window resizes, then returns the panel for removal.
async fn gym_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let size = ctx.with(|shell: &mut Shell| shell.viewport).await;
	let viewport = [size.x(), size.y()];
	let [x, y, width, height] = gym::panel(viewport);
	let [.., picture_width, picture_height] = gym::picture(viewport);
	let file = ctx
		.with(|shell: &mut Shell| shell.layers.gym().map(|gym| gym.file().to_owned()))
		.await
		.unwrap_or_default();
	let mut frame = ctx
		.element("frame")
		.container(|c| {
			c.absolute_position(x, y)
				.width(width.into())
				.height(height.into())
				.padding(gym::INSET, gym::INSET)
				.flow(flow::column_with_gap(gym::INSET as u32))
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(true)
				.style(panel(height, 1.0))
		})
		.await;
	let inner = width - 2.0 * gym::INSET;
	let mut header = frame
		.element("header")
		.container(|c| c.width(inner.into()).height(gym::CAPTION.into()).flow(flow::centered_row))
		.await;
	header
		.element("caption")
		.container(|c| {
			c.width((inner - PLACE_WIDTH).into())
				.height(gym::CAPTION.into())
				.padding(4.0, 0.0)
				.flow(flow::centered_row)
		})
		.await
		.element("text")
		.text(&file, |t| t.font_size(13.0).style([solid(TEXT)]))
		.await;
	let mut place = header
		.element("place")
		.container(|c| {
			c.width(PLACE_WIDTH.into())
				.height(gym::CAPTION.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(place_style(false, false))
		})
		.await;
	let mut label = place
		.element("label")
		.text("Select an object", |t| t.font_size(13.0).style([solid(MUTED)]))
		.await;
	let mut picture = frame
		.element("picture")
		.container(|c| {
			c.width(picture_width.into())
				.height(picture_height.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.clip(true)
		})
		.await;
	let mut hovered = false;
	let mut shown_button = (None, false);
	let mut shown_image = None;
	let mut state = ctx.proactive(|shell: &Shell| {
		let gym = shell.layers.gym()?;
		let picked = gym.picked.as_ref().map(|pick| pick.name.clone());
		Some((picked, shell.gym_image, shell.viewport))
	});
	loop {
		select! {
			_ = place.on(Events::Actuated) => {
				// An off button takes the click, so it doesn't pick behind it, but places nothing.
				if shown_button.0.is_some() {
					ctx.with(|shell: &mut Shell| shell.place_gym = true).await;
				}
			},
			_ = place.on(Events::PointerEntered) => hovered = true,
			_ = place.on(Events::PointerExited) => hovered = false,
			_ = state.next() => {},
		}
		// Another branch can win before the first read resolves; the next pass shows the state then.
		let Some(latest) = state.latest().cloned() else {
			continue;
		};
		let Some((picked, image, now)) = latest else {
			break;
		};
		if now != size {
			break;
		}
		if (picked.clone(), hovered) != shown_button {
			let on = picked.is_some();
			place.update_container(|c| c.style(place_style(on, hovered))).await;
			let (text, color) = match &picked {
				Some(name) => (format!("Place {name}"), hex(ON_ACCENT, 1.0)),
				None => ("Select an object".to_owned(), MUTED),
			};
			label.update_text(|t| t.content(&text).style([solid(color)])).await;
			shown_button = (picked, hovered);
		}
		if let Some(image) = image
			&& shown_image != Some(image)
		{
			// Declaring the image again under its key replaces the picture when the renderer names a new one.
			picture
				.element("image")
				.image(picture_width as u32, picture_height as u32, image, |i| i)
				.await;
			shown_image = Some(image);
		}
	}
	frame
}

/// The Place button: a sunken well while there's nothing to place, and once there is, a key in Byte Engine's green at
/// its top, shading down to its bottom, and shading less under the pointer.
fn place_style(on: bool, hovered: bool) -> [ConcreteLayer; 3] {
	if !on {
		return well();
	}
	let bottom = hex(if hovered { ACCENT_SHADE_HOVERED } else { ACCENT_SHADE }, 1.0);
	[
		ConcreteLayer::default().color(vertical(gym::CAPTION, ACCENT, bottom)),
		// No highlight: nothing on the button is lighter than the green.
		top_highlight(0.0),
		hairline(0.6),
	]
}
