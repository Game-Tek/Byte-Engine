//! A stored mesh's window: the mesh, on the editor's stage, fills it; a drawer on the right, titled with the mesh and
//! its folder, holds its size and totals, and a drawer on the left lists its primitives, each with its material and
//! counts.
//!
//! The layers are declared each time a mesh opens and when the window resizes, to fill it again, and removed when it
//! closes, by Escape or by closing the window. See [`crate::editor::preview`].

use resource_management::resources::mesh::{Mesh, Primitive};
use resource_management::types::{IndexStreamTypes, Stream, Streams};

use super::VALUE_FONT;
use super::level_slider::level_slider;
use super::preview_view::preview;
use super::resource_panels::{Drawer, Side, base, hold};
use super::{CORNER, FIELD_HEIGHT, MUTED, ROW_GAP, SQUIRCLE, Shell, TEXT, block, hairline, hex, solid, top_highlight};
use crate::ui::flow;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;

/// The height of one line of a primitive's card: its title, or two of its counts.
const CARD_LINE: f32 = 16.0;
/// The lines of counts under a primitive's title.
const CARD_FACT_LINES: usize = 3;
/// The space around a card's lines.
const CARD_PADDING: f32 = 6.0;
/// A card's height: its title and counts, padded.
const CARD_HEIGHT: f32 = CARD_LINE * (1 + CARD_FACT_LINES) as f32 + CARD_PADDING * 2.0;
/// The width of a card's number, so its material name starts at the same place on every card.
const CARD_NUMBER: f32 = 28.0;

/// Shows the open mesh's layers while a mesh is open, declaring them again when the window resizes.
pub(super) async fn mesh_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		// Waiting on the layer reads it only when the shell changed, so a closed view asks for no frames.
		let mut open = ctx.proactive(|shell: &Shell| shell.layers.mesh().is_some());
		while !*open.next().await {}
		let mut picture = base(ctx).await;
		preview(&mut picture).await;
		// The slider sits over the picture, in the base, which removes it with itself; a mesh with one detail level has
		// nothing to choose between.
		let levels = ctx
			.with(|shell: &mut Shell| shell.layers.mesh().map_or(0, |mesh| mesh.resource.level_count() as usize))
			.await;
		if levels > 1 {
			picture
				.element("levels")
				.component(async move |ctx| level_slider(ctx, levels).await)
				.await;
		}
		let mut drawers = [primitives_drawer(ctx).await, summary_drawer(ctx).await];
		hold(ctx, &mut drawers, |shell| shell.layers.mesh().is_some()).await;
		for drawer in drawers {
			drawer.remove().await;
		}
		picture.remove().await;
	}
}

/// How many indices `primitive` draws its triangles with at full detail, three a triangle.
fn index_count(primitive: &Primitive) -> usize {
	primitive
		.stream(Streams::Indices(IndexStreamTypes::Triangles), 0)
		.map_or(0, Stream::count)
}

/// `count` with its digits grouped in threes, such as `12,345`, so large counts read at a glance.
fn grouped(count: usize) -> String {
	let digits = count.to_string();
	let mut text = String::with_capacity(digits.len() + digits.len() / 3);
	for (index, digit) in digits.chars().enumerate() {
		if index > 0 && (digits.len() - index) % 3 == 0 {
			text.push(',');
		}
		text.push(digit);
	}
	text
}

/// The last part of a resource ID, past its folders and any `#` that names a part of its file, which is how a list
/// names it in little room.
fn short_name(id: &str) -> &str {
	id.rsplit(['/', '#']).next().unwrap_or(id)
}

/// The mesh's totals, as labels and values: its size's sides, the box that holds it, then its counts summed over its
/// primitives at full detail.
fn summary(mesh: &crate::editor::layers::Mesh) -> [(&'static str, String); 10] {
	let half = mesh.bounds.half_extents();
	let resource = &mesh.resource;
	let vertices = resource.primitives().map(|primitive| primitive.vertex_count as usize).sum();
	let indices: usize = resource.primitives().map(index_count).sum();
	let skeleton = if resource.skeleton.is_some() { "Yes" } else { "None" };
	[
		("Width", format!("{:.2} m", half.x() * 2.0)),
		("Height", format!("{:.2} m", half.y() * 2.0)),
		("Depth", format!("{:.2} m", half.z() * 2.0)),
		("Primitives", grouped(resource.primitives.len())),
		("Materials", grouped(resource.materials.len())),
		("Vertices", grouped(vertices)),
		("Indices", grouped(indices)),
		("Triangles", grouped(indices / 3)),
		("Detail levels", grouped(resource.level_count() as usize)),
		("Skeleton", skeleton.to_owned()),
	]
}

/// Declares the right drawer, titled with the mesh's file name and its folder, with the mesh's size, the sides of the
/// box that holds it, and its totals.
async fn summary_drawer(ctx: &mut EvaluationContext<Shell>) -> Drawer {
	let (id, facts) = ctx
		.with(|shell: &mut Shell| {
			let mesh = shell
				.layers
				.mesh()
				.expect("The mesh's drawers show only while a mesh is open.");
			(mesh.id.clone(), summary(mesh))
		})
		.await;
	let (name, folder) = id.rsplit_once('/').map_or((id.as_str(), ""), |(folder, name)| (name, folder));
	let mut drawer = Drawer::declare(ctx, Side::Right, name, folder).await;
	let width = drawer.width;
	let (size, totals) = facts.split_at(3);
	for (key, title, facts) in [("size", "Size", size), ("totals", "Mesh", totals)] {
		let (mut rows, _) = block(&mut drawer.content, key, title).await;
		for (label, value) in facts {
			let mut line = rows
				.element(*label)
				.container(|c| c.width(width.into()).height(FIELD_HEIGHT.into()).padding(2.0, 0.0).flow(flow::centered_row))
				.await;
			line.element("label")
				.container(|c| {
					c.width((width * 0.4).into())
						.height(FIELD_HEIGHT.into())
						.flow(flow::centered_row)
				})
				.await
				.element("text")
				.text(label, |t| t.font_size(12.0).style([solid(MUTED)]))
				.await;
			line.element("value")
				.text(value, |t| t.font_size(12.0).font(VALUE_FONT).style([solid(TEXT)]))
				.await;
		}
	}
	drawer
}

/// The `Card` struct is what a primitive's card shows: its title and material, and its counts, two to a line.
struct Card {
	/// The material's resource id, whose short name the card shows and an Alt-click on it copies, for code that names
	/// the material.
	material: String,
	facts: [(&'static str, String); CARD_FACT_LINES * 2],
}

impl Card {
	/// The card of `primitive` of `mesh`, its counts at full detail.
	fn new(mesh: &Mesh, primitive: &Primitive) -> Self {
		let indices = index_count(primitive);
		let full = primitive.lods.first();
		Self {
			material: mesh.material(primitive).id().as_str().to_owned(),
			facts: [
				("Vertices", grouped(primitive.vertex_count as usize)),
				("Indices", grouped(indices)),
				("Triangles", grouped(indices / 3)),
				("Meshlets", grouped(full.map_or(0, |level| level.meshlet_count as usize))),
				("Levels", grouped(primitive.lods.len())),
				("Skinned", if primitive.skin.is_some() { "Yes" } else { "No" }.to_owned()),
			],
		}
	}
}

/// Declares the left drawer, titled with how many primitives the mesh has, with a card for each, in order. The UI has
/// no scroll container, so the cards that don't fit the drawer's height are counted in its last line instead.
async fn primitives_drawer(ctx: &mut EvaluationContext<Shell>) -> Drawer {
	let cards = ctx
		.with(|shell: &mut Shell| {
			let mesh = &shell
				.layers
				.mesh()
				.expect("The mesh's drawers show only while a mesh is open.")
				.resource;
			mesh.primitives()
				.map(|primitive| Card::new(mesh, primitive))
				.collect::<Vec<_>>()
		})
		.await;
	let count = cards.len();
	let detail = if count == 1 {
		"1 primitive".to_owned()
	} else {
		format!("{} primitives", grouped(count))
	};
	let mut drawer = Drawer::declare(ctx, Side::Left, "Primitives", &detail).await;
	let (width, room) = (drawer.width, drawer.room);
	let fit = shown_cards(count, room);
	let mut list = drawer
		.content
		.element("list")
		.container(|c| {
			c.width(width.into())
				.height(room.max(0.0).into())
				.clip(true)
				.flow(flow::column_with_gap(ROW_GAP))
		})
		.await;
	for (index, card) in cards.iter().enumerate().take(fit) {
		primitive_card(&mut list, index, card, width).await;
	}
	if fit < count {
		let more = count - fit;
		let text = if more == 1 {
			"1 more primitive".to_owned()
		} else {
			format!("{} more primitives", grouped(more))
		};
		list.element("more")
			.container(|c| c.width(width.into()).height(CARD_LINE.into()).flow(flow::centered_row))
			.await
			.element("text")
			.text(&text, |t| t.font_size(11.0).style([solid(MUTED)]))
			.await;
	}
	drawer
}

/// How many of `count` cards show in a column `room` pixels high: all that fit, or, when some don't, those that fit
/// over the line that counts the rest.
fn shown_cards(count: usize, room: f32) -> usize {
	let step = CARD_HEIGHT + ROW_GAP as f32;
	let all = ((room + ROW_GAP as f32) / step).floor().max(0.0) as usize;
	if count <= all {
		count
	} else {
		((room + ROW_GAP as f32 - CARD_LINE - ROW_GAP as f32) / step).floor().max(0.0) as usize
	}
}

/// Declares the card of the primitive at `index` in `list`: its number and material over its counts, two to a line.
async fn primitive_card(list: &mut EvaluationContext<Shell>, index: usize, card: &Card, width: f32) {
	let inner = width - CARD_PADDING * 2.0;
	let mut frame = list
		.element(("primitive", index))
		.container(|c| {
			c.width(width.into())
				.height(CARD_HEIGHT.into())
				.corner_radius(CORNER * 0.6)
				.corner_exponent(SQUIRCLE)
				.style([solid(hex(0xFFFFFF, 0.03)), top_highlight(0.05), hairline(0.35)])
		})
		.await;
	let mut lines = frame
		.element("lines")
		.container(|c| {
			c.absolute_position(CARD_PADDING, CARD_PADDING)
				.width(inner.into())
				.height((CARD_HEIGHT - CARD_PADDING * 2.0).into())
				.flow(flow::column)
		})
		.await;
	let mut title = lines
		.element("title")
		.container(|c| c.width(inner.into()).height(CARD_LINE.into()).flow(flow::centered_row))
		.await;
	title
		.element("number")
		.container(|c| c.width(CARD_NUMBER.into()).height(CARD_LINE.into()).flow(flow::centered_row))
		.await
		.element("text")
		.text(&format!("#{index}"), |t| t.font_size(12.0).font(VALUE_FONT).style([solid(TEXT)]))
		.await;
	let (id, width) = (card.material.clone(), inner - CARD_NUMBER);
	title
		.element("material")
		.component(async move |ctx| material_name(ctx, &id, width).await)
		.await;
	// Each pair is a muted label and its value, in half the card's width.
	let half = inner * 0.5;
	for (line_index, pair) in card.facts.chunks(2).enumerate() {
		let mut line = lines
			.element(("facts", line_index))
			.container(|c| c.width(inner.into()).height(CARD_LINE.into()).flow(flow::centered_row))
			.await;
		for (label, value) in pair {
			let mut fact = line
				.element(*label)
				.container(|c| {
					c.width(half.into())
						.height(CARD_LINE.into())
						.flow(flow::centered_row_with_gap(6))
				})
				.await;
			fact.element("label")
				.text(label, |t| t.font_size(11.0).style([solid(MUTED)]))
				.await;
			fact.element("value")
				.text(value, |t| t.font_size(11.0).font(VALUE_FONT).style([solid(TEXT)]))
				.await;
		}
	}
}

/// Shows the short name of the material `id` names in a card's title, `width` pixels wide, and copies the whole `id`
/// to the clipboard when it's clicked with Alt held, until the card is removed.
async fn material_name(ctx: &mut EvaluationContext<Shell>, id: &str, width: f32) {
	let mut row = ctx
		.element("name")
		.container(|c| {
			c.width(width.into())
				.height(CARD_LINE.into())
				.flow(flow::centered_row)
				// The name takes clicks, so an Alt-click on it reaches it instead of the drawer behind.
				.hit_testable(true)
		})
		.await;
	row.element("text")
		.text(short_name(id), |t| t.font_size(11.0).style([solid(MUTED)]))
		.await;
	loop {
		row.on(Events::Actuated).await;
		ctx.with(|shell: &mut Shell| {
			if shell.held.contains(crate::editor::bindings::Key::Alt) {
				shell.copy = Some(id.to_owned());
			}
		})
		.await;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn counts_group_their_digits_in_threes() {
		assert_eq!(grouped(0), "0");
		assert_eq!(grouped(999), "999");
		assert_eq!(grouped(1_000), "1,000");
		assert_eq!(grouped(12_345_678), "12,345,678");
	}

	#[test]
	fn a_resource_is_named_by_the_last_part_of_its_id() {
		assert_eq!(short_name("models/robot.glb#Body"), "Body");
		assert_eq!(short_name("materials/steel.bema"), "steel.bema");
		assert_eq!(short_name("steel"), "steel");
	}

	#[test]
	fn cards_that_do_not_fit_leave_a_line_that_counts_them() {
		let step = CARD_HEIGHT + ROW_GAP as f32;
		let room = step * 3.0 - ROW_GAP as f32;
		assert_eq!(shown_cards(3, room), 3, "three fit exactly");
		assert_eq!(shown_cards(4, room), 2, "one gives way to the line that counts the other two");
		assert_eq!(shown_cards(0, room), 0);
	}
}
