//! The texture preview: while a texture is open from the explorer, a panel over the game view that shows it, scaled
//! down to fit, with its name and its size in pixels, and a properties panel on the right with its image settings.
//!
//! The properties panel shows what the texture bakes with: its semantic and color space, which its file name sets, and
//! the settings it takes, as switches and a typed coverage cutoff. A change goes to the editor, which writes it to the
//! texture's sidecar. Both panels are declared each time a texture opens and removed when it closes, by its ×, by
//! Escape, or by another resource opening in its place.

use resource_management::asset::{ImageSetting, ResolvedImageSettings};
use resource_management::processors::image::{Semantic, gamma_from_semantic};
use resource_management::types::Gamma;

use super::{
	ACCENT, CORNER, DRAWER_TUCK, FIELD_HEIGHT, INSET, MUTED, PROPERTIES_WIDTH, SQUIRCLE, Shell, TEXT,
	block, hairline, hex, panel, read_typed, row_style, setting_style, sidebar_height, solid, top_highlight, typing_style,
	well,
};
use crate::editor::layers::Layer;
use crate::ui::{Anchor, Position};
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::ui::flow;
use crate::utils::r#async::select;

/// The height of the panel's title bar, over the picture.
const TITLE_HEIGHT: f32 = 36.0;
/// The width of a property's control at the end of its row: a switch, a field, or a value.
const CONTROL_WIDTH: f32 = 110.0;
/// A switch's width and height.
const SWITCH: [f32; 2] = [30.0, 16.0];
/// The height of the line under the properties that says where they're saved.
const FOOTER_HEIGHT: f32 = 16.0;

/// Shows the open texture's panels while a texture is open, declaring them again for each texture that opens.
pub(super) async fn texture_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		while ctx.with(|shell: &mut Shell| shell.layers.texture().is_none()).await {
			ctx.render().await;
		}
		let mut properties = properties_panel(ctx).await;
		let mut frame = texture_panel(ctx).await;
		frame.remove().await;
		properties.remove().await;
	}
}

/// The size, in pixels, a `width` by `height` picture shows at to fill `room` as far as its aspect lets it: as large as
/// fits, larger or smaller than its own size.
fn fitted(width: u32, height: u32, room: [f32; 2]) -> [f32; 2] {
	let (width, height) = (width.max(1) as f32, height.max(1) as f32);
	let scale = (room[0] / width).min(room[1] / height);
	[width * scale, height * scale]
}

/// Declares the open texture's panel, which fills the window left of the properties panel, and keeps it until the
/// texture closes or the window resizes, then returns it for removal.
async fn texture_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	let width = (viewport.x() - PROPERTIES_WIDTH).max(200.0);
	let height = viewport.y().max(TITLE_HEIGHT + 100.0);

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
	// The mat fills the panel under its title, and the picture fills the mat as far as its aspect lets it.
	let mat_size = [width, height - TITLE_HEIGHT];
	let shown = fitted(picture.0, picture.1, mat_size);

	let mut frame = ctx
		.element("frame")
		.container(|c| {
			// The panel runs under the properties panel's rounded corners, so no window shows through them.
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
	// A dark mat under the picture, so its transparent parts read as such, and around it, where its aspect leaves room.
	let mut mat = frame
		.element("mat")
		.container(|c| {
			c.absolute_position(0.0, TITLE_HEIGHT)
				.width(mat_size[0].into())
				.height(mat_size[1].into())
				.flow(flow::center)
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
		// The panel is declared again when the window resizes, to center the picture in the room it leaves.
		let open = ctx
			.with(|shell: &mut Shell| {
				shell.layers.texture().is_some_and(|texture| texture.id == id) && shell.viewport == viewport
			})
			.await;
		if !open {
			break;
		}
	}
	frame
}

/// The `Property` enum is one row of the texture's properties panel: what its semantic sets, or a setting it takes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Property {
	Semantic,
	ColorSpace,
	Mips,
	Compression,
	Coverage,
	CpuReadable,
}

impl Property {
	/// Every row, in the panel's order: what the file name sets, then the settings.
	const ALL: [Self; 6] = [
		Self::Semantic,
		Self::ColorSpace,
		Self::Mips,
		Self::Compression,
		Self::Coverage,
		Self::CpuReadable,
	];

	fn label(self) -> &'static str {
		match self {
			Self::Semantic => "Semantic",
			Self::ColorSpace => "Color space",
			Self::Mips => "Mips",
			Self::Compression => "Compression",
			Self::Coverage => "Coverage",
			Self::CpuReadable => "CPU readable",
		}
	}

	/// The switch's state, for a setting that's on or off, or `None` for any other row.
	fn switched(self, settings: &ResolvedImageSettings) -> Option<bool> {
		match self {
			Self::Mips => Some(settings.settings.mips),
			Self::Compression => Some(settings.settings.compression),
			Self::CpuReadable => Some(settings.settings.cpu_readable),
			Self::Semantic | Self::ColorSpace | Self::Coverage => None,
		}
	}

	/// The change a click on the row's switch makes: the setting turned to the other state.
	fn toggled(self, on: bool) -> Option<ImageSetting> {
		match self {
			Self::Mips => Some(ImageSetting::Mips(!on)),
			Self::Compression => Some(ImageSetting::Compression(!on)),
			Self::CpuReadable => Some(ImageSetting::CpuReadable(!on)),
			Self::Semantic | Self::ColorSpace | Self::Coverage => None,
		}
	}

	/// What a row without a switch shows.
	fn value(self, settings: &ResolvedImageSettings) -> String {
		match self {
			Self::Semantic => semantic_name(settings.semantic).to_owned(),
			Self::ColorSpace => match gamma_from_semantic(settings.semantic) {
				Gamma::SRGB => "sRGB".to_owned(),
				Gamma::Linear => "Linear".to_owned(),
			},
			Self::Coverage => settings.settings.coverage.map_or_else(|| "off".to_owned(), |cutoff| format!("{cutoff:.2}")),
			Self::Mips | Self::Compression | Self::CpuReadable => String::new(),
		}
	}
}

/// A semantic's name, as the image settings reference names it.
fn semantic_name(semantic: Semantic) -> &'static str {
	match semantic {
		Semantic::Albedo => "albedo",
		Semantic::Normal => "normal",
		Semantic::Metallic => "metallic",
		Semantic::Roughness => "roughness",
		Semantic::Gloss => "gloss",
		Semantic::Specular => "specular",
		Semantic::Translucency => "translucency",
		Semantic::Sheen => "sheen",
		Semantic::MetallicRoughness => "metallic-roughness",
		Semantic::Emissive => "emissive",
		Semantic::Height => "height",
		Semantic::Opacity => "opacity",
		Semantic::Displacement => "displacement",
		Semantic::AO => "ambient occlusion",
		Semantic::Cavity => "cavity",
		Semantic::Other => "other",
	}
}

/// Reads a typed coverage: `off` or `false` for none, or a cutoff from 0 to 1; `None` for anything else.
fn parse_coverage(typed: &str) -> Option<ImageSetting> {
	let typed = typed.trim();
	if typed.eq_ignore_ascii_case("off") || typed.eq_ignore_ascii_case("false") {
		return Some(ImageSetting::Coverage(None));
	}
	let cutoff: f64 = typed.parse().ok()?;
	(0.0..=1.0).contains(&cutoff).then_some(ImageSetting::Coverage(Some(cutoff)))
}

/// The open texture's settings, if they could be read.
async fn settings(ctx: &mut EvaluationContext<Shell>) -> Option<ResolvedImageSettings> {
	ctx.with(|shell: &mut Shell| shell.layers.texture().and_then(|texture| texture.settings))
		.await
}

/// Declares the properties panel of the open texture, docked against the window's right edge for its whole height, as
/// the scene drawer is against the left, and returns it for removal once the texture closes or the window resizes.
async fn properties_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	let height = sidebar_height(ctx.with(|shell: &mut Shell| shell.viewport).await);
	// The panel runs past the window's right edge, so only the corners facing the picture show rounded.
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
	let mut content = frame
		.element("content")
		.container(|c| {
			c.absolute_position(INSET, INSET)
				.width(width.into())
				.height((height - INSET * 2.0).into())
				.flow(crate::ui::flow::column_with_gap(6))
		})
		.await;
	let (mut rows, _) = block(&mut content, "texture", "Texture", Property::ALL.len()).await;
	for property in Property::ALL {
		rows.element(property.label())
			.component(async move |ctx| property_row(ctx, property, width).await)
			.await;
	}
	content
		.element("footer")
		.component(async move |ctx| footer(ctx, width).await)
		.await;
	frame
}

/// One property's row: its label, and its value, its switch, or, for the coverage, a field that takes a typed cutoff.
async fn property_row(ctx: &mut EvaluationContext<Shell>, property: Property, width: f32) {
	let mut line = ctx
		.element("line")
		.container(|c| c.width(width.into()).height(FIELD_HEIGHT.into()).flow(flow::centered_row))
		.await;
	let mut label = line
		.element("label")
		.container(|c| c.width((width - CONTROL_WIDTH).into()).height(FIELD_HEIGHT.into()).flow(flow::centered_row))
		.await;
	label.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
	label
		.element("text")
		.text(property.label(), |t| t.font_size(12.0).style([solid(MUTED)]))
		.await;
	let mut control = line
		.element("control")
		.container(|c| c.width(CONTROL_WIDTH.into()).height(FIELD_HEIGHT.into()))
		.await;
	let switched = settings(ctx).await.and_then(|settings| property.switched(&settings));
	if switched.is_some() {
		switch(ctx, &mut control, property).await;
	} else if property == Property::Coverage {
		coverage_field(ctx, &mut control).await;
	} else {
		value_text(ctx, &mut control, property).await;
	}
}

/// A switch at the end of the row, lit in the accent while the setting is on; a click turns the setting over.
async fn switch(ctx: &mut EvaluationContext<Shell>, control: &mut EvaluationContext<Shell>, property: Property) {
	let knob = SWITCH[1] - 4.0;
	let mut track = control
		.element("switch")
		.container(|c| {
			c.absolute_position(CONTROL_WIDTH - SWITCH[0], (FIELD_HEIGHT - SWITCH[1]) * 0.5)
				.width(SWITCH[0].into())
				.height(SWITCH[1].into())
				.corner_radius(SWITCH[1] * 0.5)
				.hit_testable(true)
				.style(well())
		})
		.await;
	let mut handle = track
		.element("knob")
		.container(|c| {
			c.absolute_position(2.0, 2.0)
				.width(knob.into())
				.height(knob.into())
				.corner_radius(knob * 0.5)
				.style([solid(TEXT), top_highlight(0.3)])
		})
		.await;
	let mut shown = None;
	loop {
		select! {
			_ = track.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| {
					let on = shell.layers.texture().and_then(|texture| texture.settings).and_then(|settings| property.switched(&settings));
					shell.texture_edit = on.and_then(|on| property.toggled(on));
				})
				.await;
			},
			_ = ctx.render() => {},
		}
		let on = settings(ctx).await.and_then(|settings| property.switched(&settings));
		if shown != Some(on) {
			shown = Some(on);
			let on = on.unwrap_or(false);
			let x = if on { SWITCH[0] - knob - 2.0 } else { 2.0 };
			track
				.update_container(|c| c.style(if on { [solid(ACCENT), top_highlight(0.15), hairline(0.5)] } else { well() }))
				.await;
			handle.update_container(|c| c.absolute_position(x, 2.0)).await;
		}
	}
}

/// The coverage's field: a well that shows the cutoff, or `off`, and takes a typed one when clicked.
async fn coverage_field(ctx: &mut EvaluationContext<Shell>, control: &mut EvaluationContext<Shell>) {
	let mut field = control
		.element("well")
		.container(|c| {
			c.width(CONTROL_WIDTH.into())
				.height(FIELD_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.clip(true)
				.flow(flow::centered_row_with_gap(6))
				.hit_testable(true)
				.style(setting_style(true, false))
		})
		.await;
	field.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
	let mut value = field
		.element("value")
		.text("", |t| t.font_size(12.0).style([solid(TEXT)]))
		.await;
	let mut shown = None;
	loop {
		select! {
			_ = field.on(Events::Actuated) => {
				value.update_text(|t| t.content("")).await;
				let styles = [typing_style(true), setting_style(true, false)];
				// Text that isn't `off` or a cutoff from 0 to 1 changes nothing.
				if let Some(setting) = read_typed(ctx, &mut field, styles, 12.0).await.as_deref().and_then(parse_coverage) {
					ctx.with(move |shell: &mut Shell| shell.texture_edit = Some(setting)).await;
				}
				shown = None;
			},
			_ = field.on(Events::PointerEntered) => {
				field.update_container(|c| c.style(setting_style(true, true))).await;
			},
			_ = field.on(Events::PointerExited) => {
				field.update_container(|c| c.style(setting_style(true, false))).await;
			},
			_ = ctx.render() => {},
		}
		let current = settings(ctx).await;
		if shown != Some(current) {
			shown = Some(current);
			let text = current.map_or_else(|| "–".to_owned(), |settings| Property::Coverage.value(&settings));
			value.update_text(|t| t.content(&text)).await;
		}
	}
}

/// A value the row only shows, as the semantic and the color space, which the file name sets.
async fn value_text(ctx: &mut EvaluationContext<Shell>, control: &mut EvaluationContext<Shell>, property: Property) {
	let mut row = control
		.element("value")
		.container(|c| c.width(CONTROL_WIDTH.into()).height(FIELD_HEIGHT.into()).flow(flow::centered_row))
		.await;
	let current = settings(ctx).await;
	let text = current.map_or_else(|| "–".to_owned(), |settings| property.value(&settings));
	row.element("text").text(&text, |t| t.font_size(12.0).style([solid(TEXT)])).await;
	// The file name sets these, so they don't change while the texture is open.
	loop {
		ctx.render().await;
	}
}

/// The line under the properties that says where they're saved: the texture's sidecar, or, while it has none, that a
/// change creates it.
async fn footer(ctx: &mut EvaluationContext<Shell>, width: f32) {
	let mut line = ctx
		.element("line")
		.container(|c| c.width(width.into()).height(FOOTER_HEIGHT.into()).flow(flow::centered_row))
		.await;
	line.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
	let mut text = line
		.element("text")
		.text("", |t| t.font_size(11.0).style([solid(MUTED)]))
		.await;
	// Whether the settings were read and the texture has a sidecar, as last shown; `None` before the first frame.
	let mut shown = None;
	loop {
		// Only the state is compared each frame; the line is formatted when it changes.
		let state = ctx
			.with(|shell: &mut Shell| shell.layers.texture().map(|texture| texture.settings.map(|settings| settings.sidecar)))
			.await;
		if shown != Some(state) {
			shown = Some(state);
			let line = ctx
				.with(move |shell: &mut Shell| {
					let texture = shell.layers.texture()?;
					let file = texture.id.rsplit('/').next().unwrap_or(&texture.id);
					Some(match state? {
						Some(true) => format!("Saved in {file}.bead"),
						Some(false) => format!("Defaults; a change creates {file}.bead"),
						None => "The settings couldn't be read".to_owned(),
					})
				})
				.await;
			text.update_text(|t| t.content(line.as_deref().unwrap_or(""))).await;
		}
		ctx.render().await;
	}
}

#[cfg(test)]
mod tests {
	use super::{ImageSetting, fitted, parse_coverage};

	#[test]
	fn a_typed_coverage_is_off_or_a_cutoff_from_zero_to_one() {
		assert_eq!(parse_coverage(" off "), Some(ImageSetting::Coverage(None)));
		assert_eq!(parse_coverage("false"), Some(ImageSetting::Coverage(None)));
		assert_eq!(parse_coverage("0.5"), Some(ImageSetting::Coverage(Some(0.5))));
		assert_eq!(parse_coverage("1.5"), None);
		assert_eq!(parse_coverage("half"), None);
	}

	#[test]
	fn a_picture_fills_its_room_as_far_as_its_aspect_lets_it() {
		assert_eq!(fitted(100, 50, [400.0, 400.0]), [400.0, 200.0]);
		assert_eq!(fitted(800, 400, [400.0, 400.0]), [400.0, 200.0]);
		assert_eq!(fitted(400, 800, [400.0, 400.0]), [200.0, 400.0]);
	}
}
