//! A texture's window: the texture, scaled to fit beside the drawer, fills it, and a drawer on the right, titled with
//! the texture and its size in pixels, holds its image settings.
//!
//! The drawer shows what the texture bakes with: its semantic and color space, which its file name sets, and the
//! settings it takes, as switches and a typed coverage cutoff. A change goes to the editor, which writes it to the
//! texture's sidecar. A scroll over the picture zooms it; zoomed wider than the room beside the drawer, the drawer
//! slides away, and it comes back once the picture fits again. Both layers are declared each time a texture opens and
//! when the window resizes, and removed when it closes, by Escape or by closing the window.

use resource_management::asset::{ImageSetting, ResolvedImageSettings};
use resource_management::processors::image::{Semantic, gamma_from_semantic};
use resource_management::types::Gamma;

use super::VALUE_FONT;
use super::resource_panels::{Drawer, Side, base};
use super::{
	FIELD_HEIGHT, MUTED, Shell, TEXT, block, cap, channel, fits, labelled_line, open_room, read_typed, setting_style, solid,
	typing_style, value_well,
};
use crate::editor::drawer::arriving;
use crate::ui::Animation;
use crate::ui::flow;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::utils::r#async::select;

/// The width of a property's control at the end of its row: a switch, a field, or a value.
const CONTROL_WIDTH: f32 = 110.0;
/// A switch's width and height.
const SWITCH: [f32; 2] = [34.0, 18.0];
/// The gap between a switch's knob and the edge of its channel.
const KNOB_GAP: f32 = 2.0;
/// How long a switch's knob takes to slide across, in seconds.
const SLIDE: f32 = 0.16;
/// The height of the line under the properties that says where they're saved.
const FOOTER_HEIGHT: f32 = 16.0;
/// How much one scroll step zooms the picture in, as a factor of its size.
const ZOOM_STEP: f32 = 1.25;
/// The most screen pixels a texel of the texture's source takes, zoomed in all the way.
const MAX_PIXELS_PER_TEXEL: f32 = 8.0;

/// Shows the open texture's layers while a texture is open, declaring them again for each texture that opens and when
/// the window resizes.
pub(super) async fn texture_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		// Waiting on the layer reads it only when the shell changed, so a closed view asks for no frames.
		let mut open = ctx.proactive(|shell: &Shell| shell.layers.texture().is_some());
		while !*open.next().await {}
		let mut picture = base(ctx).await;
		let drawer = properties_drawer(ctx).await;
		show_texture(ctx, &mut picture, &drawer).await;
		drawer.remove().await;
		picture.remove().await;
	}
}

/// The size, in pixels, a `width` by `height` picture shows at to fill `room` as far as its aspect lets it: as large as
/// fits, larger or smaller than its own size.
fn fitted(width: u32, height: u32, room: [f32; 2]) -> [f32; 2] {
	let (width, height) = (width.max(1) as f32, height.max(1) as f32);
	let scale = (room[0] / width).min(room[1] / height);
	[width * scale, height * scale]
}

/// Shows the open texture in `base`, centered in the room the drawer leaves, zooming it as the pointer scrolls over it
/// and asking the drawer to make room while it's wider than that room, until the texture closes or the window resizes.
async fn show_texture(ctx: &mut EvaluationContext<Shell>, base: &mut EvaluationContext<Shell>, drawer: &Drawer) {
	let (viewport, drawers) = ctx.with(|shell: &mut Shell| (shell.viewport, shell.drawers)).await;
	let (window, height) = (viewport.x(), viewport.y());
	// The picture's pixels are copied into the UI's storage once, as it's declared.
	let (id, source, picture) = ctx
		.with(|shell: &mut Shell| {
			let texture = shell
				.layers
				.texture()
				.expect("The texture shows only while a texture is open.");
			let preview = &texture.preview;
			(
				texture.id.clone(),
				preview.source,
				(preview.width, preview.height, preview.pixels.clone()),
			)
		})
		.await;
	let (picture_width, picture_height, pixels) = picture;
	// Unzoomed, the picture fills the room between the drawers; zoomed in all the way, a source texel takes
	// `MAX_PIXELS_PER_TEXEL` screen pixels.
	let open = open_room(window, 1.0, drawers);
	let fit = fitted(picture_width, picture_height, [open[1] - open[0], height]);
	let most = (MAX_PIXELS_PER_TEXEL * source[0].max(1) as f32 / fit[0]).max(1.0);
	let mut zoom = 1.0f32;
	let mut holder = base
		.element("picture")
		.container(|c| c.absolute_position(0.0, 0.0).width(fit[0].into()).height(fit[1].into()))
		.await;
	let mut image = holder
		.element("image")
		.image(picture_width, picture_height, pixels, |i| {
			i.width(fit[0].into()).height(fit[1].into())
		})
		.await;
	let mut placed = None;
	let mut asked = None;
	// The picture shows until the texture closes, another opens, or the window resizes.
	let mut open = ctx.proactive(move |shell: &Shell| {
		shell.layers.texture().is_some_and(|texture| texture.id == id) && shell.viewport == viewport
	});
	loop {
		let size = [fit[0] * zoom, fit[1] * zoom];
		let wanted = fits(size[0], window, drawers);
		if asked != Some(wanted) {
			asked = Some(wanted);
			ctx.with(move |shell: &mut Shell| shell.drawer_open = wanted).await;
		}
		drawer.follow(wanted).await;
		let shown = drawer.shown.current().await;
		// Centered in the room the drawers leave, which widens as they slide away.
		let room = open_room(window, shown, drawers);
		let at = [(room[0] + room[1] - size[0]) * 0.5, (height - size[1]) * 0.5];
		if placed != Some((at, size)) {
			placed = Some((at, size));
			holder
				.update_container(|c| c.absolute_position(at[0], at[1]).width(size[0].into()).height(size[1].into()))
				.await;
			image.update_image(|i| i.width(size[0].into()).height(size[1].into())).await;
		}
		select! {
			open = open.next() => if !*open {
				break;
			},
			event = base.on(Events::Scrolled) => {
				// One step per scroll, as the graph canvas zooms, so a trackpad and a wheel move alike.
				if let Some(delta) = event.delta.filter(|delta| delta.y != 0.0) {
					let factor = if delta.y > 0.0 { ZOOM_STEP } else { 1.0 / ZOOM_STEP };
					zoom = (zoom * factor).clamp(1.0, most);
				}
			},
			// The picture follows the drawer as it slides, and so does what else centers in the room it leaves.
			shown = drawer.shown.moved() => ctx.with(move |shell: &mut Shell| shell.drawer_shown = shown).await,
		}
	}
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
			Self::Coverage => settings
				.settings
				.coverage
				.map_or_else(|| "off".to_owned(), |cutoff| format!("{cutoff:.2}")),
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

/// Declares the drawer of the open texture, titled with its file name and its size in pixels, with a row for each of
/// its image settings.
async fn properties_drawer(ctx: &mut EvaluationContext<Shell>) -> Drawer {
	let (name, size) = ctx
		.with(|shell: &mut Shell| {
			let texture = shell
				.layers
				.texture()
				.expect("The texture's drawer shows only while a texture is open.");
			let name = texture.id.rsplit('/').next().unwrap_or(&texture.id).to_owned();
			let source = texture.preview.source;
			(name, format!("{} × {}", source[0], source[1]))
		})
		.await;
	let mut drawer = Drawer::declare(ctx, Side::Right, &name, &size).await;
	let width = drawer.width;
	let (mut rows, _) = block(&mut drawer.content, "texture", "Texture").await;
	for property in Property::ALL {
		rows.element(property.label())
			.component(async move |ctx| property_row(ctx, property, width).await)
			.await;
	}
	drawer
		.content
		.element("footer")
		.component(async move |ctx| footer(ctx, width).await)
		.await;
	drawer
}

/// One property's row: its label, and its value, its switch, or, for the coverage, a field that takes a typed cutoff.
async fn property_row(ctx: &mut EvaluationContext<Shell>, property: Property, width: f32) {
	let mut line = labelled_line(ctx, property.label(), width, width - CONTROL_WIDTH).await;
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

/// A switch at the end of the row, made like a physical one: a domed knob in a channel cut into the panel, which slides
/// across as the setting turns over, filling the channel with the accent behind it while the setting is on. A click
/// turns the setting over.
async fn switch(ctx: &mut EvaluationContext<Shell>, control: &mut EvaluationContext<Shell>, property: Property) {
	let knob = SWITCH[1] - KNOB_GAP * 2.0;
	let travel = SWITCH[0] - knob - KNOB_GAP * 2.0;
	let mut track = control
		.element("switch")
		.container(|c| {
			c.absolute_position(CONTROL_WIDTH - SWITCH[0], (FIELD_HEIGHT - SWITCH[1]) * 0.5)
				.width(SWITCH[0].into())
				.height(SWITCH[1].into())
				.corner_radius(SWITCH[1] * 0.5)
				.hit_testable(true)
				.style(channel(0.0))
		})
		.await;
	let handle = track
		.element("knob")
		.container(|c| {
			c.absolute_position(KNOB_GAP, KNOB_GAP)
				.width(knob.into())
				.height(knob.into())
				.corner_radius(knob * 0.5)
				.style(cap())
		})
		.await;
	let mut on = ctx.proactive(move |shell: &Shell| {
		shell
			.layers
			.texture()
			.and_then(|texture| texture.settings)
			.and_then(|settings| property.switched(&settings))
			.unwrap_or(false)
	});
	on.next().await;
	// Where the knob stands, from 0 off to 1 on, which slides the knob and fills the channel behind it. It starts on its
	// side once the setting is read.
	let (track_id, handle_id) = (track.id(), handle.id());
	let motion = ctx
		.animation(Animation::new(0.0, arriving(SLIDE)), move |frame, v| {
			frame.update_container(track_id, |c| c.style(channel(v.current)));
			frame.update_container(handle_id, |c| c.absolute_position(KNOB_GAP + travel * v.current, KNOB_GAP));
		})
		.await;
	motion.snap(if on.latest() == Some(&true) { 1.0 } else { 0.0 }).await;
	loop {
		select! {
			_ = track.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| {
					let on = shell.layers.texture().and_then(|texture| texture.settings).and_then(|settings| property.switched(&settings));
					shell.texture_edit = on.and_then(|on| property.toggled(on));
				})
				.await;
			},
			// A turned-over setting slides the knob from wherever it stands.
			on = on.next() => motion.set_target(if *on { 1.0 } else { 0.0 }).await,
		}
	}
}

/// The coverage's field: a well that shows the cutoff, or `off`, and takes a typed one when clicked.
async fn coverage_field(ctx: &mut EvaluationContext<Shell>, control: &mut EvaluationContext<Shell>) {
	let (mut field, mut value) = value_well(control, CONTROL_WIDTH).await;
	let mut shown = None;
	let mut current = ctx.proactive(|shell: &Shell| shell.layers.texture().and_then(|texture| texture.settings));
	loop {
		select! {
			_ = current.next() => {},
			_ = field.on(Events::Actuated) => {
				value.update_text(|t| t.content("")).await;
				let styles = [typing_style(true), setting_style(true, false)];
				// Text that isn't `off` or a cutoff from 0 to 1 changes nothing.
				if let Some(setting) = read_typed(ctx, &mut field, styles, 12.0, "").await.as_deref().and_then(parse_coverage) {
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
		}
		let current = current.latest().copied().flatten();
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
		.container(|c| {
			c.width(CONTROL_WIDTH.into())
				.height(FIELD_HEIGHT.into())
				.flow(flow::centered_row)
		})
		.await;
	let current = settings(ctx).await;
	let text = current.map_or_else(|| "–".to_owned(), |settings| property.value(&settings));
	row.element("text")
		.text(&text, |t| t.font_size(12.0).font(VALUE_FONT).style([solid(TEXT)]))
		.await;
	// The file name sets these, so they don't change while the texture is open.
	std::future::pending::<()>().await;
}

/// The line under the properties that says where they're saved: the texture's sidecar, or, while it has none, that a
/// change creates it.
async fn footer(ctx: &mut EvaluationContext<Shell>, width: f32) {
	let mut line = ctx
		.element("line")
		.container(|c| {
			c.width(width.into())
				.height(FOOTER_HEIGHT.into())
				.padding(2.0, 0.0)
				.flow(flow::centered_row)
		})
		.await;
	let mut text = line
		.element("text")
		.text("", |t| t.font_size(11.0).style([solid(MUTED)]))
		.await;
	// Whether the settings were read and the texture has a sidecar, as last shown; `None` before the first read.
	let mut shown = None;
	// Only the state is compared as the shell changes; the line is formatted when it changes.
	let mut saved = ctx.proactive(|shell: &Shell| {
		shell
			.layers
			.texture()
			.map(|texture| texture.settings.map(|settings| settings.sidecar))
	});
	loop {
		let state = *saved.next().await;
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
	}
}

#[cfg(test)]
mod tests {
	use super::{ImageSetting, parse_coverage};

	#[test]
	fn a_typed_coverage_is_off_or_a_cutoff_from_zero_to_one() {
		assert_eq!(parse_coverage(" off "), Some(ImageSetting::Coverage(None)));
		assert_eq!(parse_coverage("false"), Some(ImageSetting::Coverage(None)));
		assert_eq!(parse_coverage("0.5"), Some(ImageSetting::Coverage(Some(0.5))));
		assert_eq!(parse_coverage("1.5"), None);
		assert_eq!(parse_coverage("half"), None);
	}
}
