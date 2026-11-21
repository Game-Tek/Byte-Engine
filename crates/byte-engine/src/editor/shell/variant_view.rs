//! A material variant's window: a sphere wearing the variant fills it, and a drawer on the right, titled with the
//! variant and its parent, lists the variant's parameters, each in a field that takes a typed value.
//!
//! A typed value that reads for its parameter's kind becomes [`Shell::variant_edit`], which the editor writes to the
//! variant's file; see [`crate::editor::variant`].

use super::VALUE_FONT;
use super::preview_view::preview;
use super::resource_panels::{Drawer, Side, base, hold};
use super::{FIELD_HEIGHT, MUTED, SQUIRCLE, Shell, TEXT, block, read_typed, setting_style, solid, typing_style};
use crate::editor::variant::{self, Kind};
use crate::ui::flow;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::utils::r#async::select;

/// How much of a parameter's row its field takes; the name takes the rest.
const FIELD_SHARE: f32 = 0.6;
/// The height of the line under the parameters that says where they're saved.
const FOOTER_HEIGHT: f32 = 16.0;

/// Shows the open variant's layers while a variant is open, declaring them again when the window resizes.
pub(super) async fn variant_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		// Waiting on the layer reads it only when the shell changed, so a closed view asks for no frames.
		let mut open = ctx.proactive(|shell: &Shell| shell.layers.variant().is_some());
		while !*open.next().await {}
		let mut picture = base(ctx).await;
		preview(&mut picture).await;
		let drawers = [parameters_drawer(ctx).await];
		hold(ctx, &drawers, |shell| shell.layers.variant().is_some()).await;
		for drawer in drawers {
			drawer.remove().await;
		}
		picture.remove().await;
	}
}

/// Declares the drawer, titled with the variant and the material it specializes, with a row for each of the variant's
/// parameters.
async fn parameters_drawer(ctx: &mut EvaluationContext<Shell>) -> Drawer {
	let (name, parent, count) = ctx
		.with(|shell: &mut Shell| {
			let variant = shell
				.layers
				.variant()
				.expect("The parameters drawer shows only while a variant is open.");
			(
				variant.file().to_owned(),
				format!("variant of {}", variant.parent),
				variant.parameters.len(),
			)
		})
		.await;
	let mut drawer = Drawer::declare(ctx, Side::Right, &name, &parent).await;
	let width = drawer.width;
	let (mut rows, _) = block(&mut drawer.content, "parameters", "Parameters").await;
	for index in 0..count {
		rows.element(("parameter", index))
			.component(async move |ctx| parameter_row(ctx, index, width).await)
			.await;
	}
	drawer
		.content
		.element("footer")
		.component(async move |ctx| footer(ctx, &name, width).await)
		.await;
	drawer
}

/// The parameter at `index`, by its name and kind, while the variant is open.
async fn parameter(ctx: &mut EvaluationContext<Shell>, index: usize) -> Option<(String, Kind)> {
	ctx.with(move |shell: &mut Shell| {
		let parameter = shell.layers.variant()?.parameters.get(index)?;
		Some((parameter.name.clone(), parameter.kind))
	})
	.await
}

/// One parameter's row: its name, and a field that shows its value. A click takes a typed value, or for a texture
/// opens the palette on the textures to pick one from.
async fn parameter_row(ctx: &mut EvaluationContext<Shell>, index: usize, width: f32) {
	let Some((name, kind)) = parameter(ctx, index).await else {
		return;
	};
	let field_width = (width * FIELD_SHARE).floor();
	let mut line = ctx
		.element("line")
		.container(|c| c.width(width.into()).height(FIELD_HEIGHT.into()).flow(flow::centered_row))
		.await;
	let mut label = line
		.element("label")
		.container(|c| {
			c.width((width - field_width).into())
				.height(FIELD_HEIGHT.into())
				.padding(2.0, 0.0)
				.flow(flow::centered_row)
		})
		.await;
	label
		.element("text")
		.text(&name, |t| t.font_size(12.0).style([solid(MUTED)]))
		.await;
	let mut field = line
		.element("well")
		.container(|c| {
			c.width(field_width.into())
				.height(FIELD_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.clip(true)
				.padding(8.0, 0.0)
				.flow(flow::centered_row_with_gap(6))
				.hit_testable(true)
				.style(setting_style(true, false))
		})
		.await;
	let mut value = field
		.element("value")
		.text("", |t| t.font_size(12.0).font(VALUE_FONT).style([solid(TEXT)]))
		.await;
	let mut shown = None;
	let mut current = ctx.proactive(move |shell: &Shell| Some(shell.layers.variant()?.parameters.get(index)?.value.clone()));
	loop {
		select! {
			_ = current.next() => {},
			_ = field.on(Events::Actuated) => {
				// A texture is picked from the palette, by name, rather than typed.
				if kind == Kind::Texture {
					let scope = crate::editor::palette::Scope::Textures(name.clone());
					ctx.with(move |shell: &mut Shell| shell.ask_palette = Some(scope)).await;
					continue;
				}
				value.update_text(|t| t.content("")).await;
				let styles = [typing_style(true), setting_style(true, false)];
				// Text that doesn't read as the parameter's kind changes nothing.
				if let Some(typed) = read_typed(ctx, &mut field, styles, 12.0, "").await.as_deref().and_then(|typed| kind.parse(typed)) {
					let name = name.clone();
					ctx.with(move |shell: &mut Shell| shell.variant_edit = Some((name, typed))).await;
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
		let current = current.latest().cloned().flatten();
		if shown != Some(current.clone()) {
			let text = current.as_ref().map_or_else(|| "–".to_owned(), variant::shown);
			value.update_text(|t| t.content(&text)).await;
			shown = Some(current);
		}
	}
}

/// The line under the parameters that says which file, `file`, a change is saved in.
async fn footer(ctx: &mut EvaluationContext<Shell>, file: &str, width: f32) {
	let mut line = ctx
		.element("line")
		.container(|c| {
			c.width(width.into())
				.height(FOOTER_HEIGHT.into())
				.padding(2.0, 0.0)
				.flow(flow::centered_row)
		})
		.await;
	line.element("text")
		.text(&format!("A change saves to {file}"), |t| {
			t.font_size(11.0).style([solid(MUTED)])
		})
		.await;
	// The file doesn't change while the variant is open.
	std::future::pending::<()>().await;
}
