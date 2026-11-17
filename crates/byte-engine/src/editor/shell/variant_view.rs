//! A material variant's window: a main panel titled with the variant and its parent, which previews the variant on a
//! sphere, and the variant's parameters docked on the right, each in a field that takes a typed value.
//!
//! A typed value that reads for its parameter's kind becomes [`Shell::variant_edit`], which the editor writes to the
//! variant's file; see [`crate::editor::variant`].

use super::preview_view::preview;
use super::resource_panels::{docked_panel, main_panel, picture_room};
use super::{FIELD_HEIGHT, MUTED, SQUIRCLE, Shell, TEXT, block, read_typed, row_style, setting_style, solid, typing_style};
use crate::editor::layers::Layer;
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

/// Shows the open variant's panels while a variant is open, declaring them again when the window resizes.
pub(super) async fn variant_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		while ctx.with(|shell: &mut Shell| shell.layers.variant().is_none()).await {
			ctx.render().await;
		}
		let mut parameters = parameters_panel(ctx).await;
		let mut frame = variant_panel(ctx).await;
		frame.remove().await;
		parameters.remove().await;
	}
}

/// Declares the main panel, titled with the variant and the material it specializes, with the variant's preview under
/// its title, and keeps it until the variant closes or the window resizes, then returns it for removal.
async fn variant_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	let (name, parent) = ctx
		.with(|shell: &mut Shell| {
			let variant = shell.layers.variant().expect("The variant panel shows only while a variant is open.");
			(variant.file().to_owned(), format!("variant of {}", variant.parent))
		})
		.await;
	let (mut frame, mut close, _) = main_panel(ctx, &name, &parent).await;
	preview(&mut frame, picture_room(viewport)).await;
	let mut close_hovered = false;
	loop {
		select! {
			_ = close.on(Events::Actuated) => {
				ctx.with(|shell: &mut Shell| shell.layers.close(|layer| matches!(layer, Layer::Variant(_)))).await;
			},
			_ = close.on(Events::PointerEntered) => close_hovered = true,
			_ = close.on(Events::PointerExited) => close_hovered = false,
			_ = ctx.render() => {},
		}
		close.update_container(|c| c.style(row_style(false, close_hovered))).await;
		let open = ctx
			.with(|shell: &mut Shell| shell.layers.variant().is_some() && shell.viewport == viewport)
			.await;
		if !open {
			return frame;
		}
	}
}

/// Declares the docked panel with a row for each of the variant's parameters, and returns it for removal.
async fn parameters_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let (frame, mut content, width) = docked_panel(ctx).await;
	let (count, file) = ctx
		.with(|shell: &mut Shell| {
			let variant = shell.layers.variant().expect("The parameters panel shows only while a variant is open.");
			(variant.parameters.len(), variant.file().to_owned())
		})
		.await;
	let (mut rows, _) = block(&mut content, "parameters", "Parameters", count).await;
	for index in 0..count {
		rows.element(("parameter", index))
			.component(async move |ctx| parameter_row(ctx, index, width).await)
			.await;
	}
	content
		.element("footer")
		.component(async move |ctx| footer(ctx, &file, width).await)
		.await;
	frame
}

/// The parameter at `index`, by its name and kind, while the variant is open.
async fn parameter(ctx: &mut EvaluationContext<Shell>, index: usize) -> Option<(String, Kind)> {
	ctx.with(move |shell: &mut Shell| {
		let parameter = shell.layers.variant()?.parameters.get(index)?;
		Some((parameter.name.clone(), parameter.kind))
	})
	.await
}

/// One parameter's row: its name, and a field that shows its value and takes a typed one when clicked.
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
		.container(|c| c.width((width - field_width).into()).height(FIELD_HEIGHT.into()).flow(flow::centered_row))
		.await;
	label.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
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
				// Text that doesn't read as the parameter's kind changes nothing.
				if let Some(typed) = read_typed(ctx, &mut field, styles, 12.0).await.as_deref().and_then(|typed| kind.parse(typed)) {
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
			_ = ctx.render() => {},
		}
		let current = ctx
			.with(move |shell: &mut Shell| {
				Some(shell.layers.variant()?.parameters.get(index)?.value.clone())
			})
			.await;
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
		.container(|c| c.width(width.into()).height(FOOTER_HEIGHT.into()).flow(flow::centered_row))
		.await;
	line.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
	line.element("text")
		.text(&format!("A change saves to {file}"), |t| t.font_size(11.0).style([solid(MUTED)]))
		.await;
	loop {
		ctx.render().await;
	}
}
