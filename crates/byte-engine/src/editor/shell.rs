//! The editor's window chrome: a floating toolbar, the scene overview on the left, and the properties of the
//! selection on the right, with the game showing through the space between them.
//!
//! The look is a dark, tactile take on Linear: raised surfaces lit from above (a vertical gradient, a one-pixel
//! highlight along the top edge, a dark hairline around the edge, and a soft drop shadow), sunken wells for values,
//! and Byte Engine's green as the accent for the selection.

mod graph_view;
mod texture_view;
mod toasts;

use std::time::Instant;

use crate::core::factory::Handle;
use crate::editor::fields::{CAMERA, Change, LIGHT, Setting, TRANSFORM};
use crate::editor::gizmo::Tool;
use crate::editor::layers::{Layer, Layers};
use crate::editor::palette;
use crate::editor::scene::SceneEntity;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::{Events, Key};
use crate::ui::{
	Anchor, ConcreteLayer, CurvePath, FillRule, LinearGradient, Position, Size, Transform, UiPoint, flow,
};
use crate::utils::RGBA;
use crate::utils::r#async::select;

/// The `Shell` struct is the state the editor's panels lay out from, which [`Editor::frame`](crate::editor::Editor::frame)
/// updates before each evaluation.
pub(crate) struct Shell {
	/// The window's size in pixels.
	pub(crate) viewport: Size,
	/// The entities the scene overview lists, as the editor last read them from the inspector.
	pub(crate) scene: Vec<SceneEntity>,
	/// Advanced whenever an entity appears, disappears, or is renamed, so the list knows to rebuild.
	pub(crate) scene_revision: u64,
	/// The entity the properties panel shows, chosen by clicking its row.
	pub(crate) selected: Option<Handle>,
	/// What the search field holds.
	pub(crate) search: String,
	/// The entities the overview lists for the search, in order, which [`Editor::frame`](crate::editor::Editor::frame)
	/// finds; a change advances `scene_revision`.
	pub(crate) shown: Vec<Handle>,
	/// The editor's modes, from the scene up: what shows, and what takes input.
	pub(crate) layers: Layers,
	/// What the gizmo does to the selection, as the toolbar picked it.
	pub(crate) tool: Tool,
	/// The last change the properties panel made, and the entity it changed, until the editor posts it to the game.
	pub(crate) edit: Option<(Handle, Change)>,
	/// Advanced whenever another graph opens, so the canvas is declared again for it.
	pub(crate) graph_revision: u64,
	/// Whether the canvas asked to save the open graph, until the editor writes it through the inspector.
	pub(crate) save_graph: bool,
	/// The setting the texture's properties panel changed, until the editor writes it to the texture's sidecar.
	pub(crate) texture_edit: Option<resource_management::asset::ImageSetting>,
	/// The entity the properties panel's trash button deleted, until the editor posts the deletion to the game.
	pub(crate) delete: Option<Handle>,
	/// The game's warnings and errors, shown as toasts.
	pub(crate) toasts: crate::editor::toasts::Toasts,
}

impl Shell {
	pub(crate) fn new(viewport: Size) -> Self {
		Self {
			viewport,
			scene: Vec::new(),
			scene_revision: 0,
			selected: None,
			search: String::new(),
			shown: Vec::new(),
			layers: Layers::default(),
			tool: Tool::Move,
			edit: None,
			delete: None,
			graph_revision: 0,
			save_graph: false,
			texture_edit: None,
			toasts: Default::default(),
		}
	}

	/// Deletes the selected entity: drops it from the panel's copy of the scene and the selection at once, and from
	/// the game once the editor posts [`Self::delete`].
	fn delete_selection(&mut self) {
		let Some(selected) = self.selected.take() else {
			return;
		};
		self.scene.retain(|entity| entity.handle != selected);
		self.shown.retain(|handle| *handle != selected);
		self.scene_revision += 1;
		self.delete = Some(selected);
	}

	/// Changes the selected entity through [`Self::edit`], which the editor applies to the panel's copy and posts to
	/// the game.
	fn edit_selection(&mut self, change: Change) {
		if let Some(selected) = self.selected {
			self.edit = Some((selected, change));
		}
	}

	/// The game's camera, the one the editor picks and draws the gizmo through.
	pub(crate) fn camera(&self) -> Option<Handle> {
		self.scene
			.iter()
			.find(|entity| entity.camera.is_some())
			.map(|entity| entity.handle)
	}

	/// The selected entity, while it is still in the scene.
	pub(crate) fn selection(&self) -> Option<&SceneEntity> {
		let selected = self.selected?;
		self.scene.iter().find(|entity| entity.handle == selected)
	}
}

/// The gap between the panels and the window's edges.
const MARGIN: f32 = 10.0;
/// The gap between a panel's edge and its content; the UI has no padding, so content sits in an inset child.
const INSET: f32 = 8.0;
const SIDEBAR_WIDTH: f32 = 260.0;
const PROPERTIES_WIDTH: f32 = 300.0;
/// The gap between the blocks of the properties panel.
const BLOCK_GAP: f32 = 12.0;
/// The width of the name before each row of the transform block.
const ROW_LABEL_WIDTH: f32 = 54.0;
/// The colors of the X, Y, and Z axes, as the gizmo draws them: red, Byte Engine green, and blue.
const AXIS_COLORS: [RGBA; 3] = [hex(0xF2545C, 1.0), hex(0x40D866, 1.0), hex(0x4D8CFF, 1.0)];
const TOOLBAR_WIDTH: f32 = 336.0;
const TOOL_WIDTH: f32 = 64.0;
const TOOL_HEIGHT: f32 = 26.0;
const TOOLBAR_HEIGHT: f32 = 40.0;
const HEADER_HEIGHT: f32 = 28.0;
const ROW_HEIGHT: f32 = 26.0;
const FIELD_HEIGHT: f32 = 26.0;
const CORNER: f32 = 10.0;
/// A squircle's exponent, for the softer corners Linear's surfaces have.
const SQUIRCLE: f32 = 3.0;

const TEXT: RGBA = hex(0xE6E6E9, 1.0);
const MUTED: RGBA = hex(0x8A8F98, 1.0);
/// Muted text on the toolbar, lighter than [`MUTED`] since the game's frame shows through the toolbar's glass.
const TOOLBAR_MUTED: RGBA = hex(0xB9BCC4, 1.0);
/// Byte Engine's green, the docs site's `--be-green`, for the selection; [`ACCENT_RGB`] tints with it.
const ACCENT: RGBA = hex(ACCENT_RGB, 1.0);
const ACCENT_RGB: u32 = 0x41D866;
/// The accent lit from above, where a raised tile in it catches the light.
const ACCENT_LIGHT: RGBA = hex(0x80E599, 1.0);
const ROW_GAP: u32 = 2;

/// Builds the chrome, then follows the window's size, resizing the panels that span it.
pub(crate) async fn shell(root: &mut EvaluationContext<Shell>) {
	let viewport = root.with(|shell: &mut Shell| shell.viewport).await;

	let (mut sidebar, mut content) = sidebar(root, viewport).await;
	let mut fields = properties(root).await;
	toolbar(root).await;

	// The graph canvas goes over the game view, under the palette.
	root.element("graph")
		.component(async |ctx| graph_view::graph_view(ctx).await)
		.await;
	root.element("texture")
		.component(async |ctx| texture_view::texture_view(ctx).await)
		.await;
	root.element("palette").component(async |ctx| palette_view(ctx).await).await;
	root.element("toasts").component(async |ctx| toasts::toasts(ctx).await).await;

	let mut shown_list = None;
	let mut shown_viewport = viewport;
	let mut shown_revision = None;
	let mut shown_selection = None;
	let mut shown_texture = false;
	loop {
		// A texture's properties panel takes the entity's place while the texture is open.
		let texture = root.with(|shell: &mut Shell| shell.layers.texture().is_some()).await;
		if texture != shown_texture {
			shown_texture = texture;
			fields
				.panel
				.update_container(|c| c.opacity(if texture { 0.0 } else { 1.0 }).hit_testable(!texture))
				.await;
		}
		let (viewport, revision) = root.with(|shell: &mut Shell| (shell.viewport, shell.scene_revision)).await;
		if viewport != shown_viewport {
			shown_viewport = viewport;
			let height = sidebar_height(viewport);
			sidebar
				.update_container(|c| c.height(height.into()).style(panel(height, 1.0)))
				.await;
			// The list's height follows the sidebar's, so it is declared again.
			shown_revision = None;
		}
		if shown_revision != Some(revision) {
			shown_revision = Some(revision);
			shown_list = Some(list(&mut content, shown_list.take(), viewport).await);
		}

		// The panel follows the selection, its name as the list does, and its settings block; the fields follow their
		// values on their own.
		let selection = root
			.with(|shell: &mut Shell| shell.selection().map(|entity| (entity.handle, Part::of(entity))))
			.await
			.map(|selection| (selection, revision));
		if selection != shown_selection {
			shown_selection = selection;
			fields.show(root).await;
		}
		root.render().await;
	}
}

/// The sidebar spans the window's height; the UI cannot fill "the parent less the margins", so it is computed.
fn sidebar_height(viewport: Size) -> f32 {
	(viewport.y() - MARGIN * 2.0).max(HEADER_HEIGHT + INSET * 2.0)
}

/// The scene overview's panel and its header; [`list`] fills in the rows under them.
async fn sidebar(root: &mut EvaluationContext<Shell>, viewport: Size) -> (EvaluationContext<Shell>, EvaluationContext<Shell>) {
	let height = sidebar_height(viewport);
	let mut sidebar = root
		.element("sidebar")
		.container(|c| {
			c.position(Position::anchored(Anchor::Start(MARGIN), Anchor::Start(MARGIN)))
				.width(SIDEBAR_WIDTH.into())
				.height(height.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				// The panel takes presses, so a click on it never reaches the game view behind it.
				.hit_testable(true)
				.style(panel(height, 1.0))
		})
		.await;
	let mut content = inset(&mut sidebar, SIDEBAR_WIDTH, height).await;
	let width = SIDEBAR_WIDTH - INSET * 2.0;
	header(&mut content, "Scene", width).await;
	content
		.element("search")
		.component(async move |ctx| search_field(ctx, width).await)
		.await;
	(sidebar, content)
}

/// Replaces the `previous` scene list under the sidebar's `content` with one row per entity of the current scene, and
/// returns it for the next replacement.
///
/// Removing the old list and declaring a new one is how rows for entities that left the scene go away.
async fn list(
	content: &mut EvaluationContext<Shell>,
	previous: Option<EvaluationContext<Shell>>,
	viewport: Size,
) -> EvaluationContext<Shell> {
	let width = SIDEBAR_WIDTH - INSET * 2.0;
	// The space under the search field: the sidebar less its insets, the header, the rule, the field, and the column's
	// three gaps.
	let height = (sidebar_height(viewport) - INSET * 2.0 - HEADER_HEIGHT - 2.0 - FIELD_HEIGHT - 18.0).max(ROW_HEIGHT);
	if let Some(mut previous) = previous {
		previous.remove().await;
	}
	let mut list = content
		.element("list")
		.container(|c| {
			c.width(width.into())
				.height(height.into())
				// The UI has no scroll container, so rows past the bottom are cut.
				.clip(true)
				.flow(flow::column_with_gap(ROW_GAP))
		})
		.await;

	let (rows, searching) = list
		.with(|shell: &mut Shell| {
			let rows = shell
				.shown
				.iter()
				.filter_map(|handle| shell.scene.iter().find(|entity| entity.handle == *handle))
				.map(|entity| (entity.handle, entity.label().to_string(), entity.kind.clone()))
				.collect::<Vec<_>>();
			(rows, !shell.search.trim().is_empty())
		})
		.await;
	if rows.is_empty() {
		let message = if searching { "No matches" } else { "No entities" };
		list.element("empty")
			.container(|c| c.width(width.into()).height(ROW_HEIGHT.into()).flow(flow::centered_row))
			.await
			.text(message, |t| t.font_size(12.0).style([solid(MUTED)]))
			.await;
		return list;
	}
	for (handle, label, kind) in rows {
		list.element(format_args!("{}", handle.id()))
			.component(async move |row_context| row(row_context, handle, &label, &kind, width).await)
			.await;
	}
	list
}

/// The search field over the scene list: a sunken well whose text is [`Shell::search`], with a hint while it's empty.
///
/// It keeps the keyboard's focus, since it's the editor's only text field, so typing while the editor is open searches.
async fn search_field(ctx: &mut EvaluationContext<Shell>, width: f32) {
	const HINT: &str = "Search by name, or type:Camera";
	let mut well = ctx
		.element("well")
		.container(|c| {
			c.width(width.into())
				.height(FIELD_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::centered_row)
				.hit_testable(true)
				.style(well())
		})
		.await;
	// Leading space, since the UI has no padding.
	well.element("lead").container(|c| c.width(8.into()).height(1.into())).await;
	let mut input = well
		.element("input")
		.text_field("", |t| t.font_size(12.0).style([solid(TEXT)]))
		.await;
	let mut hint = well
		.element("hint")
		.text(HINT, |t| t.font_size(12.0).style([solid(MUTED)]))
		.await;
	input.request_focus().await;

	loop {
		let edit = input.on_text_edit().await;
		let search = ctx
			.with(move |shell: &mut Shell| {
				edit.edit.apply_to(&mut shell.search);
				shell.search.clone()
			})
			.await;
		input.update_text(|t| t.content(&search)).await;
		hint.update_text(|t| t.content(if search.is_empty() { HINT } else { "" }))
			.await;
	}
}

/// The command palette: while it's open, a search field over the meshes it finds, the highlighted one lit.
///
/// The panel is declared each time the palette opens and removed when it closes, so it starts from an empty search.
/// It takes the keyboard's focus while it shows, which goes back to the scene search once it's removed.
async fn palette_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		while ctx.with(|shell: &mut Shell| shell.layers.palette().is_none()).await {
			ctx.render().await;
		}
		let mut frame = palette_panel(ctx).await;
		frame.remove().await;
	}
}

/// Declares the open palette's panel and keeps it current until the palette closes, then returns it for removal.
///
/// The panel animates in as it opens and out once it closes, as [`palette::entering`] and [`palette::leaving`] place
/// it; it's returned once it has left.
async fn palette_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let width = palette::WIDTH - palette::INSET * 2.0;
	let height = palette::height(0);
	let start = palette::entering(0.0);
	let mut frame = ctx
		.element("frame")
		.container(|c| {
			c.position(Position::anchored(Anchor::Center(0.0), Anchor::Start(palette::TOP)))
				.width(palette::WIDTH.into())
				.height(height.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.hit_testable(true)
				.opacity(start.opacity)
				.transform(look_transform(start))
				.style(panel(height, 1.0))
		})
		.await;
	let mut content = frame
		.element("content")
		.container(|c| {
			c.absolute_position(palette::INSET, palette::INSET)
				.width(width.into())
				.height((height - palette::INSET * 2.0).into())
				.flow(flow::column_with_gap(palette::GAP as u32))
		})
		.await;
	let mut well = content
		.element("well")
		.container(|c| {
			c.width(width.into())
				.height(palette::FIELD_HEIGHT.into())
				.corner_radius(7.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::centered_row)
				.style(well())
		})
		.await;
	well.element("lead").container(|c| c.width(10.into()).height(1.into())).await;
	let mut input = well
		.element("input")
		.text_field("", |t| t.font_size(14.0).style([solid(TEXT)]))
		.await;
	let mut hint = well
		.element("hint")
		// The scope's prompt fills in with the first results.
		.text("", |t| t.font_size(14.0).style([solid(MUTED)]))
		.await;
	input.request_focus().await;

	let mut rows: Option<EvaluationContext<Shell>> = None;
	let mut shown = None;
	let opened = Instant::now();
	let mut shown_look = Some(start);
	loop {
		let revision = ctx
			.with(|shell: &mut Shell| {
				let layers = &shell.layers;
				let setting = layers.setting().map(|setting| setting.revision);
				layers.palette().map(|palette| (palette.revision, layers.revision, setting))
			})
			.await;
		let Some(revision) = revision else {
			break;
		};
		if shown != Some(revision) {
			shown = Some(revision);
			// A shortcut can switch the open palette to another scope, which says what it finds, and a setting opened
			// over it shows its own row and value.
			let Some((results, highlight, search, prompt)) =
				ctx.with(|shell: &mut Shell| palette_shown(&shell.layers)).await
			else {
				break;
			};
			// The panel grows with its results, so the rows are declared again under a resized panel.
			let height = palette::height(results.len());
			frame
				.update_container(|c| c.height(height.into()).style(panel(height, 1.0)))
				.await;
			content
				.update_container(|c| c.height((height - palette::INSET * 2.0).into()))
				.await;
			input.update_text(|t| t.content(&search)).await;
			hint.update_text(|t| t.content(if search.is_empty() { prompt } else { "" }))
				.await;
			if let Some(mut previous) = rows.take() {
				previous.remove().await;
			}
			let mut list = content
				.element("rows")
				.container(|c| c.width(width.into()).flow(flow::column))
				.await;
			for (index, entry) in results.iter().enumerate() {
				palette_row(&mut list, index, entry, index == highlight, width).await;
			}
			rows = Some(list);
		}
		let look = palette::entering(opened.elapsed().as_secs_f32());
		if shown_look != Some(look) {
			shown_look = Some(look);
			show_look(&mut frame, look).await;
		}
		select! {
			edit = input.on_text_edit() => {
				let text = ctx
					.with(move |shell: &mut Shell| {
						// The field holds the value of a setting being typed, or the palette's search otherwise.
						if let Some(setting) = shell.layers.setting_mut() {
							setting.type_edit(edit.edit);
							Some(setting.value.clone())
						} else if let Some(palette) = shell.layers.palette_mut() {
							edit.edit.apply_to(&mut palette.search);
							palette.find();
							Some(palette.search.clone())
						} else {
							None
						}
					})
					.await;
				// A text field shows only what it's given, so it's given what was typed.
				if let Some(text) = text {
					input.update_text(|t| t.content(&text)).await;
				}
			},
			_ = ctx.render() => {},
		}
	}
	// The palette closed: the panel leaves before it's removed.
	let closed = Instant::now();
	loop {
		let seconds = closed.elapsed().as_secs_f32();
		show_look(&mut frame, palette::leaving(seconds)).await;
		if seconds >= palette::EXIT {
			return frame;
		}
		ctx.render().await;
	}
}

/// Places the palette's panel as `look` says.
async fn show_look(frame: &mut EvaluationContext<Shell>, look: palette::Look) {
	frame
		.update_container(move |c| c.opacity(look.opacity).transform(look_transform(look)))
		.await;
}

/// The transform that moves the palette's panel `look.y` pixels down and scales it about its top edge.
fn look_transform(look: palette::Look) -> Transform {
	Transform::identity()
		.translate_y(look.y)
		.scale(look.scale)
		.origin(UiPoint::new(0.5, 0.0))
}

/// What the open palette shows: its rows, the highlighted one, the field's text, and what the field says while it's
/// empty. A setting being typed over it shows its own row and value instead.
fn palette_shown(layers: &Layers) -> Option<(Vec<palette::Entry>, usize, String, &'static str)> {
	if let Some(setting) = layers.setting() {
		let prompt = if setting.highlight.is_some() { "Type or pick a value" } else { "Type a new value" };
		return Some((setting.rows.clone(), setting.highlight.unwrap_or(0), setting.value.clone(), prompt));
	}
	let palette = layers.palette()?;
	Some((
		palette.results.clone(),
		palette.highlight,
		palette.search.clone(),
		palette.scope.prompt(),
	))
}

/// One palette entry: an icon tinted by what it is, its name, and its full ID when the name shortens it, lit when
/// highlighted.
async fn palette_row(
	parent: &mut EvaluationContext<Shell>,
	index: usize,
	entry: &palette::Entry,
	highlighted: bool,
	width: f32,
) {
	let mut row = parent
		.element(format_args!("{index}"))
		.container(|c| {
			c.width(width.into())
				.height(palette::ROW_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.flow(flow::centered_row_with_gap(10))
				.style(row_style(highlighted, false))
		})
		.await;
	row.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
	let [top, bottom] = kind_tint(entry.kind);
	row.element("icon")
		.container(|c| {
			c.width(12.into())
				.height(12.into())
				.corner_radius(3.0)
				.corner_exponent(SQUIRCLE)
				.style(tinted_icon(top, bottom))
		})
		.await;
	row.element("name")
		.text(entry.name(), |t| t.font_size(13.0).style([solid(TEXT)]))
		.await;
	// A setting's value, or what it takes while it's edited, says more than its name repeated would.
	let detail = if entry.detail.is_empty() { &entry.label } else { &entry.detail };
	if entry.name() != detail {
		row.element("id")
			.text(detail, |t| t.font_size(11.5).style([solid(MUTED)]))
			.await;
	}
}

/// The gradient a palette entry's icon takes, so each kind reads at a glance: meshes in the accent, as the scene's
/// rows are, indigo prefabs, amber lights, pink materials and their commands, blue textures, gray nodes, teal
/// settings, and violet levels.
fn kind_tint(kind: palette::Kind) -> [RGBA; 2] {
	match kind {
		palette::Kind::Mesh => [ACCENT_LIGHT, ACCENT],
		palette::Kind::Prefab => [hex(0x8A93F0, 1.0), hex(0x5E6AD2, 1.0)],
		palette::Kind::Light(_) => [hex(0xFFD27A, 1.0), hex(0xF5A524, 1.0)],
		palette::Kind::Graph | palette::Kind::Material | palette::Kind::NewMaterial | palette::Kind::NewVariant => {
			[hex(0xF29BD4, 1.0), hex(0xD9539F, 1.0)]
		}
		palette::Kind::Texture => [hex(0x8CC4FF, 1.0), hex(0x3D8BE0, 1.0)],
		palette::Kind::Node => [hex(0xB9BCC4, 1.0), hex(0x6E737C, 1.0)],
		palette::Kind::Setting(_) => [hex(0x7FE3E0, 1.0), hex(0x2BA8A4, 1.0)],
		palette::Kind::NewLevel | palette::Kind::Folder => [hex(0xC9A4FF, 1.0), hex(0x8B5CF6, 1.0)],
	}
}

/// One entity's row: it selects the entity when clicked, and lights up under the pointer and while selected.
async fn row(ctx: &mut EvaluationContext<Shell>, handle: Handle, label: &str, kind: &str, width: f32) {
	let mut frame = ctx
		.element("row")
		.container(|c| {
			c.width(width.into())
				.height(ROW_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.flow(flow::centered_row_with_gap(8))
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	frame.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
	frame
		.element("icon")
		.container(|c| {
			c.width(12.into())
				.height(12.into())
				.corner_radius(3.0)
				.corner_exponent(SQUIRCLE)
				.style(icon())
		})
		.await;
	frame
		.element("label")
		.text(label, |t| t.font_size(12.5).style([solid(TEXT)]))
		.await;
	if kind != label {
		frame
			.element("kind")
			.text(kind, |t| t.font_size(11.0).style([solid(MUTED)]))
			.await;
	}

	let mut hovered = false;
	let mut shown = (false, false);
	loop {
		select! {
			_ = frame.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| shell.selected = Some(handle)).await;
			},
			_ = frame.on(Events::PointerEntered) => hovered = true,
			_ = frame.on(Events::PointerExited) => hovered = false,
			_ = ctx.render() => {},
		}
		let selected = ctx.with(move |shell: &mut Shell| shell.selected == Some(handle)).await;
		if (selected, hovered) != shown {
			shown = (selected, hovered);
			frame.update_container(|c| c.style(row_style(selected, hovered))).await;
		}
	}
}

/// The height of a properties block of `rows` rows of fields: its header and rule over the rows, a gap between each.
const fn block_height(rows: usize) -> f32 {
	HEADER_HEIGHT + 2.0 + rows as f32 * FIELD_HEIGHT + (rows + 1) as f32 * 6.0
}

/// The properties panel's height: its insets around the entity block, the transform block while the selection has a
/// transform, and a settings block of `part_rows` rows while the selection has one.
fn properties_height(transform: bool, part_rows: usize) -> f32 {
	let transform = if transform {
		BLOCK_GAP + block_height(TRANSFORM.len())
	} else {
		0.0
	};
	let part = if part_rows > 0 {
		BLOCK_GAP + block_height(part_rows)
	} else {
		0.0
	};
	INSET * 2.0 + block_height(1) + transform + part
}

/// The `Part` struct names the settings block the panel shows under the selection's transform: a camera's or a
/// light's, with those of the block's settings the entity has, since a light shows its settings as authored.
#[derive(Clone, Copy)]
struct Part {
	handle: Handle,
	title: &'static str,
	settings: &'static [Setting],
	/// One bit per setting in `settings`, set for those the entity has.
	shown: u32,
}

impl Part {
	/// The settings block `entity` shows, or `None` for an entity that's neither a camera nor a light.
	fn of(entity: &SceneEntity) -> Option<Self> {
		let (title, settings) = match &entity.light {
			_ if entity.camera.is_some() => ("Camera", &CAMERA[..]),
			Some(light) => (light.kind.title(), &LIGHT[..]),
			None => return None,
		};
		let shown = settings
			.iter()
			.enumerate()
			.filter(|(_, setting)| setting.applies(entity))
			.fold(0, |shown, (index, _)| shown | 1 << index);
		Some(Self {
			handle: entity.handle,
			title,
			settings,
			shown,
		})
	}

	/// The settings the block shows, in order.
	fn settings(&self) -> impl Iterator<Item = &'static Setting> {
		let shown = self.shown;
		self.settings
			.iter()
			.enumerate()
			.filter(move |(index, _)| shown & 1 << index != 0)
			.map(|(_, setting)| setting)
	}

	/// How many rows the block's fields take, two to a row.
	fn rows(&self) -> usize {
		(self.shown.count_ones() as usize).div_ceil(2)
	}
}

/// Two parts are the same block when they're the same kind of block, for the same entity, with the same settings.
impl PartialEq for Part {
	fn eq(&self, other: &Self) -> bool {
		self.handle == other.handle && self.title == other.title && self.shown == other.shown
	}
}

/// The properties panel's parts that follow the selection: the panel, which grows to hold the blocks the selection
/// has, the name, the transform block, and the camera's or light's settings block.
struct Fields {
	panel: EvaluationContext<Shell>,
	blocks: EvaluationContext<Shell>,
	name: EvaluationContext<Shell>,
	/// The transform block, while the selection has a transform. Its fields follow the selection on their own.
	transform: Option<EvaluationContext<Shell>>,
	/// The settings block and the part it shows, while the selection is a camera or a light.
	part: Option<(Part, EvaluationContext<Shell>)>,
}

impl Fields {
	/// Shows the selected entity's name, or a dash without a selection, its transform block while it has a transform,
	/// and its settings block while it's a camera or a light.
	async fn show(&mut self, root: &mut EvaluationContext<Shell>) {
		let selection = root
			.with(|shell: &mut Shell| {
				shell
					.selection()
					.map(|entity| (entity.label().to_string(), Part::of(entity)))
			})
			.await;
		let name = selection.as_ref().map_or("–", |(label, _)| label.as_str());
		self.name.update_text(|t| t.content(name)).await;

		// The scene holds only entities with a transform, so every selection has one.
		let transform = selection.is_some();
		// A settings block belongs to one entity, so it's declared again for another one.
		let part = selection.and_then(|(_, part)| part);
		let part_changed = self.part.as_ref().map(|(shown, _)| *shown) != part;
		let transform_changed = self.transform.is_some() != transform;
		if !part_changed && !transform_changed {
			return;
		}
		// Blocks stack in the order they're declared, so the settings block also comes off when the transform block
		// comes back, to be declared again under it.
		if let Some((_, mut block)) = self.part.take() {
			block.remove().await;
		}
		if transform_changed {
			match self.transform.take() {
				Some(mut block) => block.remove().await,
				None => self.transform = Some(transform_block(&mut self.blocks).await),
			}
		}
		let height = properties_height(transform, part.map_or(0, |part| part.rows()));
		self.panel
			.update_container(|c| c.height(height.into()).style(panel(height, 1.0)))
			.await;
		if let Some(part) = part {
			self.part = Some((part, settings_block(&mut self.blocks, part).await));
		}
	}
}

/// The properties panel: a block for each part of the selected entity, the entity itself with its name, its
/// transform, and its camera's or light's settings.
async fn properties(root: &mut EvaluationContext<Shell>) -> Fields {
	let height = properties_height(false, 0);
	let mut panel_element = root
		.element("properties")
		.container(|c| {
			c.position(Position::anchored(Anchor::End(MARGIN), Anchor::Start(MARGIN)))
				.width(PROPERTIES_WIDTH.into())
				.height(height.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.hit_testable(true)
				.style(panel(height, 1.0))
		})
		.await;
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	// Inset like the other panels' content, but with the wider gap that sets the blocks apart.
	let mut blocks = panel_element
		.element("blocks")
		.container(|c| {
			c.absolute_position(INSET, INSET)
				.width(width.into())
				.height(properties_height(true, CAMERA.len().max(LIGHT.len()).div_ceil(2)).into())
				.flow(flow::column_with_gap(BLOCK_GAP as u32))
		})
		.await;

	// The entity block is the one every selection has, so its header holds the trash button; the other blocks come
	// and go with the parts the entity has.
	let (mut entity_block, mut entity_header) = block(&mut blocks, "entity", "Entity", 1).await;
	let name = value_well(&mut entity_block, width).await;
	entity_header
		.element("delete")
		.component(async move |ctx| delete_button(ctx, width).await)
		.await;

	Fields {
		panel: panel_element,
		blocks,
		name,
		transform: None,
		part: None,
	}
}

/// The transform block: a row of three fields for each of the position, rotation, and scale.
async fn transform_block(blocks: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	let (mut transform_block, _) = block(blocks, "transform", "Transform", TRANSFORM.len()).await;
	// Each row is one sunken well split three ways by two one-pixel dividers.
	let vector_width = width - ROW_LABEL_WIDTH - 6.0;
	let field_width = (vector_width - 2.0) / 3.0;
	for (row, (label, settings)) in TRANSFORM.iter().enumerate() {
		let mut line = transform_block
			.element(("row", row))
			.container(|c| {
				c.width(width.into())
					.height(FIELD_HEIGHT.into())
					.flow(flow::centered_row_with_gap(6))
			})
			.await;
		line.element("label")
			.container(|c| {
				c.width(ROW_LABEL_WIDTH.into())
					.height(FIELD_HEIGHT.into())
					.flow(flow::centered_row)
			})
			.await
			.text(*label, |t| t.font_size(11.0).style([solid(MUTED)]))
			.await;
		let mut vector = line
			.element("vector")
			.container(|c| {
				c.width(vector_width.into())
					.height(FIELD_HEIGHT.into())
					.corner_radius(6.0)
					.corner_exponent(SQUIRCLE)
					// The parts' fills stop at the well's rounded corners.
					.clip(true)
					.flow(flow::centered_row)
					.style(well())
			})
			.await;
		for (index, setting) in settings.iter().enumerate() {
			if index > 0 {
				vector
					.element(("divider", index))
					.container(|c| c.width(1.into()).height(14.into()).style([solid(hex(0xFFFFFF, 0.08))]))
					.await;
			}
			vector
				.element(setting.label)
				.component(async move |ctx| setting_field(ctx, setting, field_width, false).await)
				.await;
		}
	}
	transform_block
}

/// A block of the properties panel, `rows` rows of fields tall, under a header that names it; returns the block, to
/// which the caller adds the rows, and the header.
async fn block(
	parent: &mut EvaluationContext<Shell>,
	key: &str,
	title: &str,
	rows: usize,
) -> (EvaluationContext<Shell>, EvaluationContext<Shell>) {
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	let mut block = parent
		.element(key)
		.container(|c| {
			c.width(width.into())
				.height(block_height(rows).into())
				.flow(flow::column_with_gap(6))
		})
		.await;
	let header = header(&mut block, title, width).await;
	(block, header)
}

/// A camera's or light's settings block: the part's settings two to a row.
async fn settings_block(blocks: &mut EvaluationContext<Shell>, part: Part) -> EvaluationContext<Shell> {
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	let key = format!("{} {}", part.title, part.handle.id());
	let (mut block, _) = block(blocks, &key, part.title, part.rows()).await;
	let field_width = (width - 6.0) / 2.0;
	let mut line = None;
	for (index, setting) in part.settings().enumerate() {
		// Every other setting starts a new row.
		if index % 2 == 0 {
			line = Some(
				block
					.element(("row", index / 2))
					.container(|c| c.width(width.into()).height(FIELD_HEIGHT.into()).flow(flow::row_with_gap(6)))
					.await,
			);
		}
		let Some(line) = line.as_mut() else {
			unreachable!("The first setting starts a row.");
		};
		line.element(setting.label)
			.component(async move |ctx| setting_field(ctx, setting, field_width, true).await)
			.await;
	}
	block
}

/// The trash button, which deletes the selected entity from the game; faint while nothing is selected, and red under
/// the pointer.
async fn delete_button(ctx: &mut EvaluationContext<Shell>, width: f32) {
	const SIZE: f32 = 24.0;
	const ICON: u32 = 14;
	let mut button = ctx
		.element("button")
		.container(|c| {
			c.absolute_position(width - SIZE, (HEADER_HEIGHT - SIZE) * 0.5)
				.width(SIZE.into())
				.height(SIZE.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	let mut icon = button
		.element("icon")
		.path(|path| {
			path.outline(trash_can(ICON))
				.view_box([16.0, 16.0])
				.fill_rule(FillRule::EvenOdd)
				.style([solid(MUTED)])
		})
		.await;

	let mut hovered = false;
	let mut shown = None;
	loop {
		select! {
			_ = button.on(Events::Actuated) => {
				ctx.with(|shell: &mut Shell| shell.delete_selection()).await;
			},
			_ = button.on(Events::PointerEntered) => hovered = true,
			_ = button.on(Events::PointerExited) => hovered = false,
			_ = ctx.render() => {},
		}
		let selected = ctx.with(|shell: &mut Shell| shell.selection().is_some()).await;
		let look = (selected, hovered && selected);
		if shown != Some(look) {
			shown = Some(look);
			let color = match look {
				(false, _) => hex(0x8A8F98, 0.35),
				(true, true) => AXIS_COLORS[0],
				(true, false) => MUTED,
			};
			button.update_container(|c| c.style(row_style(false, look.1))).await;
			icon.update_path(|path| path.style([solid(color)])).await;
		}
	}
}

/// A trash can outlined in a 16 by 16 view box, drawn `size` pixels square: a lid with a handle over a hollow can with
/// two slats. Its contours fill by the even-odd rule, so the can's inner outline cuts it hollow.
fn trash_can(size: u32) -> CurvePath {
	let mut path = CurvePath::new(size.into(), size.into());
	let mut contour = |points: &[(f32, f32)]| {
		for (index, &point) in points.iter().enumerate() {
			path.push_line(point, points[(index + 1) % points.len()]);
		}
	};
	let rectangle = |left: f32, top: f32, right: f32, bottom: f32| [(left, top), (right, top), (right, bottom), (left, bottom)];
	// The lid and its handle.
	contour(&rectangle(2.0, 3.0, 14.0, 4.5));
	contour(&rectangle(6.0, 1.25, 10.0, 3.0));
	// The can, tapering toward its foot, and the hollow inside it.
	contour(&[(3.0, 5.5), (13.0, 5.5), (12.2, 15.0), (3.8, 15.0)]);
	contour(&[(4.5, 7.0), (11.5, 7.0), (10.9, 13.5), (5.1, 13.5)]);
	// The slats, inside the hollow.
	contour(&rectangle(6.4, 8.5, 7.3, 12.0));
	contour(&rectangle(8.7, 8.5, 9.6, 12.0));
	path
}

/// A sunken well that shows one value; returns the value's text, for the caller to rewrite.
async fn value_well(parent: &mut EvaluationContext<Shell>, width: f32) -> EvaluationContext<Shell> {
	let mut well = parent
		.element("well")
		.container(|c| {
			c.width(width.into())
				.height(FIELD_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::centered_row_with_gap(6))
				.style(well())
		})
		.await;
	// Leading space, since the UI has no padding.
	well.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
	well.element("value")
		.text("–", |t| t.font_size(12.0).style([solid(TEXT)]))
		.await
}

/// One setting's field for the selected entity: a well that scrubs the setting while the pointer drags across it, and
/// takes a typed value when clicked without dragging. A transform axis's label takes the axis's color.
///
/// A `framed` field is a well of its own; an unframed one is a part of a well its parent draws, such as one axis of a
/// vector.
async fn setting_field(ctx: &mut EvaluationContext<Shell>, setting: &'static Setting, width: f32, framed: bool) {
	let mut well = ctx
		.element("well")
		.container(|c| {
			c.width(width.into())
				.height(FIELD_HEIGHT.into())
				.corner_radius(if framed { 6.0 } else { 0.0 })
				.corner_exponent(SQUIRCLE)
				.clip(true)
				.flow(flow::centered_row_with_gap(6))
				.hit_testable(true)
				.style(setting_style(framed, false))
		})
		.await;
	// Leading space, since the UI has no padding.
	well.element("lead").container(|c| c.width(2.into()).height(1.into())).await;
	let label_color = setting.axis().map_or(MUTED, |axis| AXIS_COLORS[axis]);
	well.element("label")
		.text(setting.label, |t| t.font_size(11.0).style([solid(label_color)]))
		.await;
	let mut value = well
		.element("value")
		.text("–", |t| t.font_size(12.0).style([solid(TEXT)]))
		.await;

	// The selection when the pointer pressed, which a scrub measures from, and the setting's value then.
	let mut pressed: Option<(SceneEntity, Option<f32>)> = None;
	let mut dragged = false;
	let mut hovered = false;
	let mut shown_hover = false;
	// The value shown, `None` before the first frame; the inner `None` is no selection or an unset value.
	let mut shown: Option<Option<Option<f32>>> = None;
	let mut text = String::new();
	loop {
		select! {
			_ = well.on(Events::Grabbed) => {
				pressed = ctx
					.with(move |shell: &mut Shell| shell.selection().map(|entity| (entity.clone(), setting.value(entity))))
					.await;
				dragged = false;
			},
			event = well.on(Events::Dragged) => {
				dragged = true;
				if let (Some((entity, start)), Some(delta)) = (&pressed, event.delta) {
					// A value the entity refuses, such as a near plane past the far one, leaves it as it was.
					if let Some(change) = setting.with(entity, Some(setting.scrubbed(*start, delta.x))) {
						ctx.with(move |shell: &mut Shell| shell.edit_selection(change)).await;
					}
				}
			},
			_ = well.on(Events::DragEnded) => {
				if !dragged && pressed.is_some() {
					value.update_text(|t| t.content("")).await;
					type_setting(ctx, &mut well, setting, framed).await;
					shown = None;
				}
				pressed = None;
			},
			_ = well.on(Events::PointerEntered) => hovered = true,
			_ = well.on(Events::PointerExited) => hovered = false,
			_ = ctx.render() => {},
		}
		// The value is compared every frame, but formatted only when it changed.
		let current = ctx
			.with(move |shell: &mut Shell| shell.selection().map(|entity| setting.value(entity)))
			.await;
		if shown != Some(current) {
			shown = Some(current);
			match current {
				Some(value) => setting.display(value, &mut text),
				None => text.replace_range(.., "–"),
			}
			value.update_text(|t| t.content(&text)).await;
		}
		if hovered != shown_hover {
			shown_hover = hovered;
			well.update_container(|c| c.style(setting_style(framed, hovered))).await;
		}
	}
}

/// Takes a typed value for `setting` in the field's `well`. Text that isn't a number, or a value the selection
/// refuses, changes nothing.
async fn type_setting(
	ctx: &mut EvaluationContext<Shell>,
	well: &mut EvaluationContext<Shell>,
	setting: &'static Setting,
	framed: bool,
) {
	let styles = [typing_style(framed), setting_style(framed, false)];
	let Some(value) = read_typed(ctx, well, styles, 12.0)
		.await
		.and_then(|typed| setting.parse(&typed))
	else {
		return;
	};
	ctx.with(move |shell: &mut Shell| {
		if let Some(change) = shell.selection().and_then(|entity| setting.with(entity, value)) {
			shell.edit_selection(change);
		}
	})
	.await;
}

/// Reads text typed into `well`, which takes the keyboard's focus until Enter returns the text, or Escape or a press
/// outside the well returns `None`. The well wears the first of `styles` while it takes text, and the second after.
async fn read_typed(
	ctx: &mut EvaluationContext<Shell>,
	well: &mut EvaluationContext<Shell>,
	styles: [[ConcreteLayer; 3]; 2],
	font_size: f32,
) -> Option<String> {
	let [typing, rest] = styles;
	// While text is typed, the field takes Enter and Escape, so Escape cancels the typing instead of popping a layer.
	ctx.with(|shell: &mut Shell| shell.layers.push(Layer::Typing)).await;
	well.update_container(|c| c.style(typing)).await;
	let mut input = well
		.element("input")
		.text_field("", |t| t.font_size(font_size).style([solid(TEXT)]))
		.await;
	input.request_focus().await;
	let mut typed = String::new();
	let apply = loop {
		select! {
			edit = input.on_text_edit() => {
				edit.edit.apply_to(&mut typed);
				input.update_text(|t| t.content(&typed)).await;
			},
			_ = input.on_key(Key::Enter) => break true,
			_ = input.on_key(Key::Escape) => break false,
			_ = ctx.render() => {
				let pointer = ctx.pointer().await;
				let outside = well.geometry().await.is_some_and(|geometry| {
					let [x, y] = [pointer.position.x, pointer.position.y];
					x < geometry.x() || x > geometry.right() || y < geometry.y() || y > geometry.bottom()
				});
				if pointer.pressed && outside {
					break false;
				}
			},
		}
	};
	// Removing the field gives the keyboard back to the scene search.
	input.remove().await;
	ctx.with(|shell: &mut Shell| shell.layers.close(|layer| matches!(layer, Layer::Typing)))
		.await;
	well.update_container(|c| c.style(rest)).await;
	apply.then_some(typed)
}

/// The floating toolbar over the game view: the gizmo's tools, the chosen one pressed, and the game's name.
async fn toolbar(root: &mut EvaluationContext<Shell>) {
	let mut toolbar = root
		.element("toolbar")
		.container(|c| {
			c.position(Position::anchored(Anchor::Center(0.0), Anchor::Start(MARGIN)))
				.width(TOOLBAR_WIDTH.into())
				.height(TOOLBAR_HEIGHT.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.flow(flow::centered_row_with_gap(10))
				.hit_testable(true)
				// Translucent and blurred, so the game shows through it.
				.style(panel(TOOLBAR_HEIGHT, 0.72))
		})
		.await;
	let mut tools = toolbar
		.element("tools")
		.container(|c| {
			c.width((TOOL_WIDTH * 3.0 + 8.0).into())
				.height(TOOL_HEIGHT.into())
				.flow(flow::centered_row_with_gap(4))
		})
		.await;
	for tool in Tool::ALL {
		tools
			.element(tool.label())
			.component(async move |ctx| tool_button(ctx, tool).await)
			.await;
	}
	toolbar
		.element("divider")
		.container(|c| c.width(1.into()).height(18.into()).style([solid(hex(0x000000, 0.45))]))
		.await;
	toolbar
		.element("title")
		.text("Third Person", |t| t.font_size(13.0).style([solid(TOOLBAR_MUTED)]))
		.await;
}

/// One of the toolbar's tool buttons: pressed while its tool is chosen, lifted faintly under the pointer, and choosing
/// its tool when clicked.
async fn tool_button(ctx: &mut EvaluationContext<Shell>, tool: Tool) {
	let mut button = ctx
		.element("button")
		.container(|c| {
			c.width(TOOL_WIDTH.into())
				.height(TOOL_HEIGHT.into())
				.corner_radius(7.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(tool_style(false, false))
		})
		.await;
	let mut label = button
		.element("label")
		.text(tool.label(), |t| t.font_size(13.0).style([solid(TOOLBAR_MUTED)]))
		.await;

	let mut hovered = false;
	let mut shown = None;
	loop {
		select! {
			_ = button.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| shell.tool = tool).await;
			},
			_ = button.on(Events::PointerEntered) => hovered = true,
			_ = button.on(Events::PointerExited) => hovered = false,
			_ = ctx.render() => {},
		}
		let chosen = ctx.with(move |shell: &mut Shell| shell.tool == tool).await;
		if shown != Some((chosen, hovered)) {
			shown = Some((chosen, hovered));
			button.update_container(|c| c.style(tool_style(chosen, hovered))).await;
			let color = if chosen || hovered { TEXT } else { TOOLBAR_MUTED };
			label.update_text(|t| t.style([solid(color)])).await;
		}
	}
}

/// A child inset from `parent`'s edges by [`INSET`], stacking its children in a column; it stands in for padding.
async fn inset(parent: &mut EvaluationContext<Shell>, width: f32, height: f32) -> EvaluationContext<Shell> {
	parent
		.element("content")
		.container(|c| {
			c.absolute_position(INSET, INSET)
				.width((width - INSET * 2.0).into())
				.height((height - INSET * 2.0).into())
				.flow(flow::column_with_gap(6))
		})
		.await
}

/// A panel's title over a hairline that separates it from the content; returns the title's row, which can hold a
/// button at its end.
async fn header(parent: &mut EvaluationContext<Shell>, title: &str, width: f32) -> EvaluationContext<Shell> {
	let mut header = parent
		.element("header")
		.container(|c| c.width(width.into()).height(HEADER_HEIGHT.into()).flow(flow::centered_row))
		.await;
	header
		.element("title")
		.text(title, |t| t.font_size(13.0).style([solid(TEXT)]))
		.await;
	// An engraved line: a dark hairline over a lighter one, like a groove cut into the surface.
	parent
		.element("rule")
		.container(|c| {
			c.width(width.into()).height(2.into()).style([
				solid(hex(0x000000, 0.5)),
				ConcreteLayer::default()
					.color(hex(0xFFFFFF, 0.05).into())
					.inset_shadow([0.0, -1.0], 0.0),
			])
		})
		.await;
	header
}

/// A raised surface lit from above, `height` tall, with its fill at `alpha`; below 1 it also blurs what is behind.
fn panel(height: f32, alpha: f32) -> [ConcreteLayer; 4] {
	[
		ConcreteLayer::default()
			.color(hex(0x000000, 0.35).into())
			.drop_shadow([0.0, 6.0], 12.0),
		ConcreteLayer::default()
			.color(vertical(height, hex(0x26272D, alpha), hex(0x1E1F24, alpha)))
			.backdrop_blur(if alpha < 1.0 { 16.0 } else { 0.0 }),
		top_highlight(0.06),
		hairline(0.5),
	]
}

/// A control pushed into the surface: darker, lit from below, with the shadow inside.
fn pressed(height: f32) -> [ConcreteLayer; 3] {
	[
		ConcreteLayer::default().color(vertical(height, hex(0x1A1B1F, 1.0), hex(0x222328, 1.0))),
		ConcreteLayer::default()
			.color(hex(0x000000, 0.55).into())
			.inset_shadow([0.0, 1.0], 2.0),
		hairline(0.6),
	]
}

/// A setting's field: a sunken well of its own when `framed`, otherwise clear over its parent's well; lifted faintly
/// under the pointer to show it can be dragged.
fn setting_style(framed: bool, hovered: bool) -> [ConcreteLayer; 3] {
	if !framed {
		return row_style(false, hovered);
	}
	let [fill, shadow, edge] = well();
	if hovered {
		[solid(hex(0x1D1E22, 1.0)), shadow, edge]
	} else {
		[fill, shadow, edge]
	}
}

/// A setting's field while it takes typed text: a well of its own ringed in the accent color, or, as part of its
/// parent's well, tinted with it, since the parent's rounded edge would clip a ring.
fn typing_style(framed: bool) -> [ConcreteLayer; 3] {
	if !framed {
		return [solid(hex(ACCENT_RGB, 0.22)), top_highlight(0.0), hairline(0.0)];
	}
	let [fill, shadow, _] = well();
	[fill, shadow, ConcreteLayer::default().color(ACCENT.into()).stroke(1.0)]
}

/// A tool button: pressed while chosen, otherwise clear with a faint lift under the pointer.
fn tool_style(chosen: bool, hovered: bool) -> [ConcreteLayer; 3] {
	if chosen {
		pressed(TOOL_HEIGHT)
	} else {
		row_style(false, hovered)
	}
}

/// A scene row: clear at rest, a faint lift under the pointer, and, while selected, tinted with the accent and edged
/// in it, as the docs site marks what's active, so the light text stays readable over the brand's own green.
fn row_style(selected: bool, hovered: bool) -> [ConcreteLayer; 3] {
	if selected {
		[
			ConcreteLayer::default().color(vertical(ROW_HEIGHT, hex(ACCENT_RGB, 0.22), hex(ACCENT_RGB, 0.14))),
			top_highlight(0.08),
			ConcreteLayer::default().color(hex(ACCENT_RGB, 0.55).into()).stroke(1.0),
		]
	} else {
		// A transparent fill keeps the row a surface the pointer can reach while it paints nothing.
		let fill = if hovered { hex(0xFFFFFF, 0.05) } else { hex(0xFFFFFF, 0.0) };
		[solid(fill), top_highlight(if hovered { 0.04 } else { 0.0 }), hairline(0.0)]
	}
}

/// A small raised tile in the accent color that marks a row as an entity.
fn icon() -> [ConcreteLayer; 3] {
	tinted_icon(ACCENT_LIGHT, ACCENT)
}

/// A small raised tile lit from `top` to `bottom`.
fn tinted_icon(top: RGBA, bottom: RGBA) -> [ConcreteLayer; 3] {
	[
		ConcreteLayer::default().color(vertical(12.0, top, bottom)),
		top_highlight(0.25),
		hairline(0.45),
	]
}

/// A recessed well for a value.
fn well() -> [ConcreteLayer; 3] {
	[
		solid(hex(0x16171A, 1.0)),
		ConcreteLayer::default()
			.color(hex(0x000000, 0.5).into())
			.inset_shadow([0.0, 1.0], 1.5),
		hairline(0.55),
	]
}

/// A one-pixel light line along the top edge, as if the surface caught light from above.
fn top_highlight(alpha: f32) -> ConcreteLayer {
	// An inset shadow moved down by one pixel paints only the strip its hole leaves uncovered: the top edge.
	ConcreteLayer::default()
		.color(hex(0xFFFFFF, alpha).into())
		.inset_shadow([0.0, 1.0], 0.0)
}

/// A dark one-pixel line around the edge; strokes paint inside the shape, so it stays crisp.
fn hairline(alpha: f32) -> ConcreteLayer {
	ConcreteLayer::default().color(hex(0x000000, alpha).into()).stroke(1.0)
}

/// A top-to-bottom gradient over an element `height` tall.
fn vertical(height: f32, top: RGBA, bottom: RGBA) -> crate::ui::Color {
	LinearGradient::new([0.0, 0.0], [0.0, height], top, bottom).into()
}

fn solid(color: RGBA) -> ConcreteLayer {
	ConcreteLayer::default().color(color.into())
}

/// Converts a `0xRRGGBB` color, as design tools write it, and an alpha.
const fn hex(rgb: u32, alpha: f32) -> RGBA {
	RGBA::new(
		((rgb >> 16) & 0xFF) as f32 / 255.0,
		((rgb >> 8) & 0xFF) as f32 / 255.0,
		(rgb & 0xFF) as f32 / 255.0,
		alpha,
	)
}
