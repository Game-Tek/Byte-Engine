//! A stored mesh's window: a main panel titled with the mesh, which previews it on the editor's stage, and its size
//! docked on the right.
//!
//! Both panels are declared each time a mesh opens and removed when it closes, by its ×, by Escape, or when the window
//! resizes, to fill the room it leaves. See [`crate::editor::preview`].

use super::preview_view::preview;
use super::resource_panels::{docked_panel, main_panel, picture_room};
use super::{FIELD_HEIGHT, MUTED, Shell, TEXT, block, row_style, solid};
use crate::editor::layers::Layer;
use crate::ui::flow;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::utils::r#async::select;

/// Shows the open mesh's panels while a mesh is open, declaring them again when the window resizes.
pub(super) async fn mesh_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		while ctx.with(|shell: &mut Shell| shell.layers.mesh().is_none()).await {
			ctx.render().await;
		}
		let mut size = size_panel(ctx).await;
		let mut frame = mesh_panel(ctx).await;
		frame.remove().await;
		size.remove().await;
	}
}

/// Declares the main panel, titled with the mesh's file name and its folder, with its preview under the title, and
/// keeps it until the mesh closes or the window resizes, then returns it for removal.
async fn mesh_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	let id = ctx
		.with(|shell: &mut Shell| shell.layers.mesh().expect("The mesh panel shows only while a mesh is open.").id.clone())
		.await;
	let (name, folder) = id.rsplit_once('/').map_or((id.as_str(), ""), |(folder, name)| (name, folder));
	let (mut frame, mut close, _) = main_panel(ctx, name, folder).await;
	preview(&mut frame, picture_room(viewport)).await;
	let mut close_hovered = false;
	loop {
		select! {
			_ = close.on(Events::Actuated) => {
				ctx.with(|shell: &mut Shell| shell.layers.close(|layer| matches!(layer, Layer::Mesh(_)))).await;
			},
			_ = close.on(Events::PointerEntered) => close_hovered = true,
			_ = close.on(Events::PointerExited) => close_hovered = false,
			_ = ctx.render() => {},
		}
		close.update_container(|c| c.style(row_style(false, close_hovered))).await;
		let open = ctx
			.with(|shell: &mut Shell| shell.layers.mesh().is_some() && shell.viewport == viewport)
			.await;
		if !open {
			return frame;
		}
	}
}

/// Declares the docked panel with the mesh's width, height, and depth, the sides of the box that holds it, and returns
/// it for removal.
async fn size_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let (frame, mut content, width) = docked_panel(ctx).await;
	let half = ctx
		.with(|shell: &mut Shell| {
			let mesh = shell.layers.mesh().expect("The size panel shows only while a mesh is open.");
			mesh.bounds.half_extents()
		})
		.await;
	let sides = [("Width", half.x()), ("Height", half.y()), ("Depth", half.z())];
	let (mut rows, _) = block(&mut content, "size", "Size", sides.len()).await;
	for (label, half) in sides {
		let mut line = rows
			.element(label)
			.container(|c| c.width(width.into()).height(FIELD_HEIGHT.into()).flow(flow::centered_row))
			.await;
		line.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
		line.element("label")
			.container(|c| c.width((width * 0.4).into()).height(FIELD_HEIGHT.into()).flow(flow::centered_row))
			.await
			.element("text")
			.text(label, |t| t.font_size(12.0).style([solid(MUTED)]))
			.await;
		line.element("value")
			.text(&format!("{:.2} m", half * 2.0), |t| t.font_size(12.0).style([solid(TEXT)]))
			.await;
	}
	frame
}
