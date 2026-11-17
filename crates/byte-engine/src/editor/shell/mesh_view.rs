//! A stored mesh's window: the mesh, on the editor's stage, fills it, and a drawer on the right, titled with the mesh and
//! its folder, holds its size.
//!
//! Both layers are declared each time a mesh opens and when the window resizes, to fill it again, and removed when it
//! closes, by Escape or by closing the window. See [`crate::editor::preview`].

use super::preview_view::preview;
use super::resource_panels::{Drawer, base};
use super::{FIELD_HEIGHT, MUTED, Shell, TEXT, block, solid};
use crate::ui::flow;
use crate::ui::layout::context::{Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;

/// Shows the open mesh's layers while a mesh is open, declaring them again when the window resizes.
pub(super) async fn mesh_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		while ctx.with(|shell: &mut Shell| shell.layers.mesh().is_none()).await {
			ctx.render().await;
		}
		let mut picture = base(ctx).await;
		preview(&mut picture).await;
		let mut drawer = size_drawer(ctx).await;
		drawer.hold(ctx, |shell| shell.layers.mesh().is_some()).await;
		drawer.remove().await;
		picture.remove().await;
	}
}

/// Declares the drawer, titled with the mesh's file name and its folder, with the mesh's width, height, and depth, the
/// sides of the box that holds it.
async fn size_drawer(ctx: &mut EvaluationContext<Shell>) -> Drawer {
	let (id, half) = ctx
		.with(|shell: &mut Shell| {
			let mesh = shell.layers.mesh().expect("The size drawer shows only while a mesh is open.");
			(mesh.id.clone(), mesh.bounds.half_extents())
		})
		.await;
	let (name, folder) = id.rsplit_once('/').map_or((id.as_str(), ""), |(folder, name)| (name, folder));
	let mut drawer = Drawer::declare(ctx, name, folder).await;
	let width = drawer.width;
	let sides = [("Width", half.x()), ("Height", half.y()), ("Depth", half.z())];
	let (mut rows, _) = block(&mut drawer.content, "size", "Size", sides.len()).await;
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
	drawer
}
