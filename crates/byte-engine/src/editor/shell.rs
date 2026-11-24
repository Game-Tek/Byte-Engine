//! The editor's window chrome: a floating toolbar, the scene overview on the left, and the properties of the
//! selection on the right, with the game showing through the space between them.
//!
//! The look is a dark, tactile take on Linear: raised surfaces lit from above (a vertical gradient, a one-pixel
//! highlight along the top edge, a dark hairline around the edge, and a soft drop shadow), sunken wells for values,
//! and Byte Engine's green as the accent for the selection.

mod graph_view;
mod gym_view;
mod hint_bar;
mod level_slider;
mod mesh_view;
mod preview_view;
mod resource_panels;
mod texture_view;
mod toasts;
mod variant_view;

use std::rc::Rc;

pub(crate) use resource_panels::{fits, open_room};

use crate::core::factory::Handle;
use crate::editor::drawer;
use crate::editor::fields::{CAMERA, Change, LIGHT, Setting, TRANSFORM};
use crate::editor::gizmo::Tool;
use crate::editor::icons;
use crate::editor::layers::{Layer, Layers};
use crate::editor::palette;
use crate::editor::scene::SceneEntity;
use crate::ui::animation::Smoothing;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::{Events, Key};
use crate::ui::{
	Anchor, AnimationHandle, ConcreteLayer, LayoutExtent, LinearGradient, Position, Size, Sizing, Transform, UiPoint, Vector,
	VectorView, draw_vector, flow,
};
use crate::utils::RGBA;
use crate::utils::r#async::select;

/// The `Shell` struct is the state the editor's panels lay out from, which [`Editor::frame`](crate::editor::Editor::frame)
/// updates before each evaluation.
///
/// An application's own panels lay out in an [`EvaluationContext`] over it, through a
/// [`Panel`](crate::editor::Panel); its state is the editor's own.
pub struct Shell {
	/// The window's layout size, its pixels over [`Self::scale`]; see [Scale](/docs/reference/gui#scale).
	pub(crate) viewport: Size,
	/// The window's pixels per layout unit, which sizes what the editor draws in pixels, such as a preview's image.
	pub(crate) scale: f32,
	/// The entities the scene overview lists, as the editor last read them from the inspector.
	pub(crate) scene: Vec<SceneEntity>,
	/// Advanced whenever an entity appears, disappears, or is renamed, so the list knows to rebuild.
	pub(crate) scene_revision: u64,
	/// The entity the properties panel shows, chosen by clicking its row.
	pub(crate) selected: Option<Handle>,
	/// What the search field holds.
	pub(crate) search: String,
	/// Whether a key asked the search field to take text, as a click on it does, until the field takes the focus.
	pub(crate) search_scene: bool,
	/// The entities the overview lists for the search, in order, which [`Editor::frame`](crate::editor::Editor::frame)
	/// finds; a change advances `scene_revision`.
	pub(crate) shown: Vec<Handle>,
	/// The editor's modes, from the scene up: what shows, and what takes input.
	pub(crate) layers: Layers,
	/// What the gizmo does to the selection, as the toolbar picked it.
	pub(crate) tool: Tool,
	/// The last change the properties panel made, and the entity it changed, until the editor posts it to the game.
	pub(crate) edit: Option<(Handle, Change)>,
	/// Whether the canvas asked to save the open graph, until the editor writes it through the inspector.
	pub(crate) save_graph: bool,
	/// The setting the texture's properties panel changed, until the editor writes it to the texture's sidecar.
	pub(crate) texture_edit: Option<resource_management::asset::ImageSetting>,
	/// The editor's own camera, while the game window shows it instead of the game's.
	pub(crate) view_camera: Option<Handle>,
	/// The parameter the variant's panel changed, by name, with its new value, until the editor writes it to the
	/// variant's file.
	pub(crate) variant_edit: Option<(String, serde_json::Value)>,
	/// The palette a panel asked for, such as the textures a variant's parameter can show, until the editor opens it.
	pub(crate) ask_palette: Option<crate::editor::palette::Scope>,
	/// The image the gym being browsed is drawn into, once the renderer opened it.
	pub(crate) gym_image: Option<crate::ghi::ImageHandle>,
	/// Whether the gym's Place button was clicked, until the editor carries the pick into the scene.
	pub(crate) place_gym: bool,
	/// The image the window's preview is drawn into, once the renderer opened it.
	pub(crate) preview: Option<crate::ghi::ImageHandle>,
	/// Where a drag over the preview started and where it reached, in arcball coordinates, until the editor turns the
	/// preview's subject by it; see [`crate::editor::preview::Framing::turn`].
	pub(crate) turn: Option<([f32; 2], [f32; 2])>,
	/// How many steps the preview was scrolled closer, until the editor moves its camera by them.
	pub(crate) zoom: f32,
	/// Whether the preview window's drawers should show: while what the window shows fits between them.
	pub(crate) drawer_open: bool,
	/// How much of the preview window's drawers show, from 0 hidden to 1 in place, which centers what the window shows
	/// in the room they leave.
	pub(crate) drawer_shown: f32,
	/// The widths of the preview window's left and right drawers, 0 for one it doesn't have, which the room between
	/// them is measured by.
	pub(crate) drawers: [f32; 2],
	/// The detail level the preview draws its mesh at, which the level slider sets, or `None` to select levels by
	/// distance.
	pub(crate) detail_level: Option<u32>,
	/// The entity the properties panel's trash button deleted, until the editor posts the deletion to the game.
	pub(crate) delete: Option<Handle>,
	/// The game's warnings and errors, shown as toasts.
	pub(crate) toasts: crate::editor::toasts::Toasts,
	/// The keys held down, as the editor keeps them; Alt turns a click on a material's name into copying its resource
	/// id.
	pub(crate) held: crate::editor::bindings::Held,
	/// Text a click asked to put on the clipboard, until the editor puts it there.
	pub(crate) copy: Option<String>,
	/// The panels the application registered for its component types, which the entity panel shows at its bottom.
	pub(crate) panels: Vec<crate::editor::panels::Registration>,
	/// The messages the application's panels posted, by entity and type, with their JSON payloads, until the editor
	/// posts them through the inspector.
	pub(crate) posts: Vec<(Handle, &'static str, String)>,
	/// Where the pointer dragging the gizmo is, in normalized window coordinates, or `None` while nothing is dragged.
	/// The editor writes it only when it changes, so the hint bar wakes only then.
	pub(crate) dragged: Option<[f32; 2]>,
	/// Whether a press ran a binding since the last modifiers went down, which finished what they started.
	pub(crate) finished: bool,
	/// The open palette's height, as it last fitted its results, which the editor tests the pointer against; see
	/// [`palette::contains`].
	pub(crate) palette_height: f32,
	/// Whether the scene drawer has left the window, which a closed editor draws until.
	pub(crate) drawer_gone: bool,
	/// The icons the editor draws, by purpose; see [`crate::editor::icons`].
	pub(crate) icons: Rc<[(&'static str, Vector)]>,
}

impl Shell {
	pub(crate) fn new(viewport: Size, icons: Rc<[(&'static str, Vector)]>) -> Self {
		Self {
			viewport,
			icons,
			scale: 1.0,
			scene: Vec::new(),
			scene_revision: 0,
			selected: None,
			search: String::new(),
			search_scene: false,
			shown: Vec::new(),
			layers: Layers::default(),
			tool: Tool::Move,
			edit: None,
			delete: None,
			save_graph: false,
			texture_edit: None,
			variant_edit: None,
			ask_palette: None,
			gym_image: None,
			place_gym: false,
			view_camera: None,
			preview: None,
			turn: None,
			zoom: 0.0,
			drawer_open: true,
			drawer_shown: 0.0,
			drawers: [0.0; 2],
			detail_level: None,
			toasts: Default::default(),
			held: Default::default(),
			copy: None,
			panels: Vec::new(),
			posts: Vec::new(),
			dragged: None,
			finished: false,
			palette_height: 0.0,
			drawer_gone: true,
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

	/// The camera the game window shows, the one the editor picks and draws the gizmo through: the editor's own while
	/// it flies, or the game's.
	pub(crate) fn camera(&self) -> Option<Handle> {
		self.view_camera.or_else(|| {
			self.scene
				.iter()
				.find(|entity| entity.camera.is_some())
				.map(|entity| entity.handle)
		})
	}

	/// Which registered panels `entity` shows, one bit per [`Self::panels`] entry: those of the component types it has,
	/// among the first 64 registered.
	fn panels_of(&self, entity: &SceneEntity) -> u64 {
		let types = &entity.types;
		self.panels
			.iter()
			.take(64)
			.enumerate()
			.filter(|(_, panel)| types.contains(&panel.component))
			.fold(0, |shown, (index, _)| shown | 1 << index)
	}

	/// The selected entity, while it is still in the scene.
	pub(crate) fn selection(&self) -> Option<&SceneEntity> {
		let selected = self.selected?;
		self.scene.iter().find(|entity| entity.handle == selected)
	}
}

/// The gap between the panels and the window's edges.
const MARGIN: f32 = 10.0;
/// The gap between a panel's edge and its content. The drawers keep it in an inset child, since their content also
/// starts past the part tucked behind the window's edge, which even padding on both sides can't express.
const INSET: f32 = 8.0;
/// The width of the scene drawer that shows, against the window's left edge.
const SIDEBAR_WIDTH: f32 = 260.0;
/// How far the drawer reaches past the window's left edge, which hides its left corners, so only the corners facing
/// the scene are round.
const DRAWER_TUCK: f32 = 24.0;
/// How far left the drawer moves to hide, its shadow included.
const DRAWER_HIDDEN: f32 = SIDEBAR_WIDTH + 32.0;
const PROPERTIES_WIDTH: f32 = 300.0;
/// The width of what a properties block holds, inside the panel's insets.
pub(crate) const CONTENT_WIDTH: f32 = PROPERTIES_WIDTH - INSET * 2.0;
/// The gap between the blocks of the properties panel.
const BLOCK_GAP: f32 = 12.0;
/// The width of the name before each row of the transform block.
const ROW_LABEL_WIDTH: f32 = 54.0;
/// The colors of the X, Y, and Z axes, as the gizmo draws them: red, Byte Engine green, and blue.
const AXIS_COLORS: [RGBA; 3] = [hex(0xF2545C, 1.0), hex(0x40D866, 1.0), hex(0x4D8CFF, 1.0)];
const TOOL_WIDTH: f32 = 64.0;
pub(crate) const TOOL_HEIGHT: f32 = 26.0;
const TOOLBAR_HEIGHT: f32 = 40.0;
const HEADER_HEIGHT: f32 = 28.0;
const ROW_HEIGHT: f32 = 26.0;
const FIELD_HEIGHT: f32 = 26.0;
const CORNER: f32 = 10.0;
/// A squircle's exponent, for the softer corners Linear's surfaces have.
const SQUIRCLE: f32 = 3.0;

/// The font name values draw with, such as numbers and asset IDs, which [`crate::editor::install`] gives the font the
/// game chose for them.
pub(crate) const VALUE_FONT: &str = "editor-values";

pub(crate) const TEXT: RGBA = hex(0xE6E6E9, 1.0);
const MUTED: RGBA = hex(0x8A8F98, 1.0);
/// Muted text on the toolbar, lighter than [`MUTED`] since the game's frame shows through the toolbar's glass.
const TOOLBAR_MUTED: RGBA = hex(0xB9BCC4, 1.0);
/// Byte Engine's green, the docs site's `--be-green`, for the selection; [`ACCENT_RGB`] tints with it.
const ACCENT: RGBA = hex(ACCENT_RGB, 1.0);
const ACCENT_RGB: u32 = 0x41D866;
/// The accent lit from above, where a raised tile in it catches the light.
const ACCENT_LIGHT: RGBA = hex(0x80E599, 1.0);
const ROW_GAP: u32 = 2;

/// Builds the chrome, then follows the scene, the selection, and whether the editor is open.
pub(crate) async fn shell(root: &mut EvaluationContext<Shell>) {
	let (sidebar, mut content) = sidebar(root).await;
	let (mut outline, slide, mut empty) = scene_list(&mut content).await;
	let mut fields = properties(root).await;
	root.element("toolbar").component(async |ctx| toolbar(ctx).await).await;

	// The gym's panel goes under the palette, which finds the gym's waypoints over it.
	root.element("gym").component(async |ctx| gym_view::gym_view(ctx).await).await;
	root.element("palette").component(async |ctx| palette_view(ctx).await).await;
	root.element("toasts").component(async |ctx| toasts::toasts(ctx).await).await;
	root.element("hints")
		.component(async |ctx| hint_bar::hint_bar(ctx).await)
		.await;

	let mut outline_shown = false;
	let mut shown_selection = None;
	// The editor starts closed, with the drawer hidden; the engine moves it whenever the editor opens or closes.
	let sidebar_id = sidebar.id();
	let drawer_shown = root
		.animation(drawer::shown(0.0), move |frame, v| {
			frame.update_container(sidebar_id, |c| c.transform(drawer_transform(v.current)))
		})
		.await;
	let mut gone = true;
	// The loop wakes only when what it shows changes: whether the editor is open, the scene, or the selection, with the
	// panel's settings block and the game's panels it has.
	let mut state = root.proactive(|shell: &Shell| {
		let selection = shell
			.selection()
			.map(|entity| (entity.handle, Part::of(entity), shell.panels_of(entity)));
		// The selected entity's row in the list, which the list's outline slides to.
		let row = shell
			.selected
			.and_then(|selected| shell.shown.iter().position(|shown| *shown == selected));
		// Whether the list is empty, and if so whether a search emptied it.
		let emptied = shell.shown.is_empty().then(|| !shell.search.trim().is_empty());
		(shell.layers.open(), shell.scene_revision, selection, row, emptied)
	});
	loop {
		// What woke the loop.
		enum Woke {
			State,
			Moved,
		}
		let woke = select! {
			_ = state.next() => Woke::State,
			_ = drawer_shown.moved() => Woke::Moved,
		};
		match woke {
			// The drawer left: the editor's render pass can stop drawing.
			Woke::Moved => {
				if drawer_shown.current().await == 0.0 && !gone {
					gone = true;
					root.with(|shell: &mut Shell| shell.drawer_gone = true).await;
				}
			}
			Woke::State => {
				let Some(&(open, revision, selection, row, emptied)) = state.latest() else {
					continue;
				};
				// A drawer turned back partway starts from where it stands.
				drawer_shown.update(|shown| drawer::head(shown, open, drawer::ENTER, drawer::EXIT)).await;
				if open && gone {
					gone = false;
					root.with(|shell: &mut Shell| shell.drawer_gone = false).await;
				}
				let message = match emptied {
					Some(true) => "No matches",
					Some(false) => "No entities",
					None => "",
				};
				empty.update_text(|t| t.content(message)).await;
				let offset = row.map(|row| row as f32 * (ROW_HEIGHT + ROW_GAP as f32));
				follow(&mut outline, &slide, &mut outline_shown, offset).await;
				// The panel follows the selection, its name as the list does, and its settings block; the fields
				// follow their values on their own.
				let selection = selection.map(|selection| (selection, revision));
				if selection != shown_selection {
					shown_selection = selection;
					fields.show(root).await;
				}
			}
		}
	}
}

/// Builds a resource window's interface: the material graph's canvas, the texture's preview, the variant's
/// parameters, or the mesh's preview, whichever is at the bottom of its layers, under the palette that opens over it.
pub(crate) async fn window(root: &mut EvaluationContext<Shell>) {
	root.element("graph")
		.component(async |ctx| graph_view::graph_view(ctx).await)
		.await;
	root.element("texture")
		.component(async |ctx| texture_view::texture_view(ctx).await)
		.await;
	root.element("variant")
		.component(async |ctx| variant_view::variant_view(ctx).await)
		.await;
	root.element("mesh")
		.component(async |ctx| mesh_view::mesh_view(ctx).await)
		.await;
	root.element("palette").component(async |ctx| palette_view(ctx).await).await;
	// The views run on their own; the root only keeps them mounted.
	std::future::pending::<()>().await;
}

/// The transform that shows `shown` of the drawer, from 0, hidden past the window's left edge, to 1, in place.
fn drawer_transform(shown: f32) -> Transform {
	Transform::identity().translate_x((shown - 1.0) * DRAWER_HIDDEN)
}

/// The scene drawer, against the window's left edge and as tall as the window, and its header; [`list`] fills in the
/// rows under them.
async fn sidebar(root: &mut EvaluationContext<Shell>) -> (EvaluationContext<Shell>, EvaluationContext<Shell>) {
	let mut sidebar = root
		.element("sidebar")
		.container(|c| {
			c.position(Position::anchored(Anchor::Start(-DRAWER_TUCK), Anchor::Start(0.0)))
				.width((SIDEBAR_WIDTH + DRAWER_TUCK).into())
				.height(Sizing::full())
				// The padding offers the content the height inside the insets.
				.padding(0.0, INSET)
				.transform(drawer_transform(0.0))
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				// The panel takes presses, so a click on it never reaches the game view behind it.
				.hit_testable(true)
				.style(panel(1.0))
		})
		.await;
	let width = SIDEBAR_WIDTH - INSET * 2.0;
	// The content starts past the part tucked behind the window's edge.
	let mut content = sidebar
		.element("content")
		.container(|c| {
			c.absolute_position(DRAWER_TUCK + INSET, INSET)
				.width(width.into())
				.height(Sizing::full())
				.flow(flow::column_with_gap(6))
		})
		.await;
	header(&mut content, "Scene", width).await;
	content
		.element("search")
		.component(async move |ctx| search_field(ctx, width).await)
		.await;
	(sidebar, content)
}

/// Declares the scene list under the sidebar's `content`: one row per entity the search shows, kept in step with the
/// scene by [`EvaluationContext::each`]. Returns the outline that marks the selected row, with its motion, and the
/// message that shows while the list is empty.
async fn scene_list(content: &mut EvaluationContext<Shell>) -> (EvaluationContext<Shell>, Slide, EvaluationContext<Shell>) {
	let width = SIDEBAR_WIDTH - INSET * 2.0;
	let mut list = content
		.element("list")
		.container(|c| {
			// The list takes the drawer's height under the header and the search field.
			c.width(width.into())
				.height(Sizing::fill())
				// The UI has no scroll container, so rows past the bottom are cut.
				.clip(true)
				.flow(flow::column_with_gap(ROW_GAP))
		})
		.await;
	let (outline, slide) = highlight(&mut list, Position::absolute(0, 0), width, ROW_HEIGHT).await;
	// Out of the list's flow, so it takes no row's place.
	let empty = list
		.element("empty")
		.container(|c| {
			c.absolute_position(0, 0)
				.width(width.into())
				.height(ROW_HEIGHT.into())
				.flow(flow::centered_row)
		})
		.await
		.text("", |t| t.font_size(12.0).style([solid(MUTED)]))
		.await;
	list.each(
		|shell: &Shell| &shell.shown,
		async move |ctx, handle| row(ctx, handle, width).await,
	)
	.await;
	(outline, slide, empty)
}

/// The search field over the scene list: a sunken well whose text is [`Shell::search`], with a hint while it's empty.
///
/// It takes the keyboard's focus only once it's clicked, so until then the keys fly the editor's camera. Enter, Escape,
/// or a press outside the well gives them back and keeps the search.
async fn search_field(ctx: &mut EvaluationContext<Shell>, width: f32) {
	const HINT: &str = "Search by name, or type:Camera";
	let mut field = ctx
		.element("well")
		.container(|c| {
			c.width(width.into())
				.height(FIELD_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.padding(8.0, 0.0)
				.flow(flow::centered_row)
				.hit_testable(true)
				.pointer(crate::rendering::Pointer::Text)
				.style(well())
		})
		.await;
	let mut input = field
		.element("input")
		.text_field("", |t| t.font_size(12.0).style([solid(TEXT)]))
		.await;
	let mut hint = field
		.element("hint")
		.text(HINT, |t| t.font_size(12.0).style([solid(MUTED)]))
		.await;
	// A click on the well, or the `/` key through the shell, starts the typing.
	let mut asked = ctx.proactive(|shell: &Shell| shell.search_scene);

	loop {
		select! {
			_ = field.on(Events::Actuated) => {},
			wanted = asked.next() => if !*wanted { continue },
		}
		// While the search takes text, it keeps the keys, and Escape stops the typing instead of closing the editor.
		ctx.with(|shell: &mut Shell| {
			shell.search_scene = false;
			shell.layers.push(Layer::Typing);
		})
		.await;
		field.update_container(|c| c.style(typing_style(true))).await;
		input.request_focus().await;
		loop {
			select! {
				edit = input.on_text_edit() => {
					let search = ctx
						.with(move |shell: &mut Shell| {
							edit.edit.apply_to(&mut shell.search);
							shell.search.clone()
						})
						.await;
					input.update_text(|t| t.content(&search)).await;
					hint.update_text(|t| t.content(if search.is_empty() { HINT } else { "" })).await;
				},
				_ = input.on_key(Key::Enter) => break,
				_ = input.on_key(Key::Escape) => break,
				// A press outside the well takes the focus, which ends the typing.
				_ = input.focus_change(true) => break,
			}
		}
		input.release_focus().await;
		ctx.with(|shell: &mut Shell| shell.layers.close(|layer| matches!(layer, Layer::Typing)))
			.await;
		field.update_container(|c| c.style(well())).await;
	}
}

/// The command palette: while it's open, a search field over the meshes it finds, the highlighted one lit.
///
/// The panel is declared each time the palette opens and removed when it closes, so it starts from an empty search.
/// It takes the keyboard's focus while it shows, and its removal takes the focus with it.
async fn palette_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		// Waiting on the layer reads it only when the shell changed, so a closed view asks for no frames.
		let mut open = ctx.proactive(|shell: &Shell| shell.layers.palette().is_some());
		while !*open.next().await {}
		let mut frame = palette_panel(ctx).await;
		frame.remove().await;
	}
}

/// Declares the open palette's panel and keeps it current until the palette closes, then returns it for removal.
///
/// The panel arrives as it opens and leaves once it closes, placed by [`look`] from how much of it shows; it's returned
/// once it has left.
async fn palette_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let width = palette::WIDTH - palette::INSET * 2.0;
	// The panel grows with its results, and the shell keeps the height it grows to in `Shell::palette_height`.
	let mut height = palette::FIELD_HEIGHT + palette::INSET * 2.0;
	let mut frame = ctx
		.element("frame")
		.container(|c| {
			c.position(Position::anchored(Anchor::Center(0.0), Anchor::Start(palette::TOP)))
				.width(palette::WIDTH.into())
				.height(Sizing::fit())
				.padding(palette::INSET, palette::INSET)
				.flow(flow::column_with_gap(palette::GAP as u32))
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.hit_testable(true)
				.opacity(0.0)
				.transform(look(0.0))
				.style(panel(1.0))
		})
		.await;
	let mut well = frame
		.element("well")
		.container(|c| {
			c.width(width.into())
				.height(palette::FIELD_HEIGHT.into())
				.corner_radius(7.0)
				.corner_exponent(SQUIRCLE)
				.padding(10.0, 0.0)
				.flow(flow::centered_row)
				.style(well())
		})
		.await;
	let mut input = well
		.element("input")
		.text_field("", |t| t.font_size(14.0).style([solid(TEXT)]))
		.await;
	let mut hint = well
		.element("hint")
		// The scope's prompt fills in with the first results.
		.text("", |t| t.font_size(14.0).style([solid(MUTED)]))
		.await;
	// One highlight behind the rows slides to the highlighted one. It's declared before the rows, so they draw over
	// it, and it moves by its transform, so following the highlight never lays the panel out again.
	let rows_top = palette::INSET + palette::FIELD_HEIGHT + palette::GAP;
	let (mut highlight_frame, slide) = highlight(
		&mut frame,
		Position::absolute(palette::INSET, rows_top),
		width,
		palette::ROW_HEIGHT,
	)
	.await;
	// The panel arrives as the engine moves it, while the loop waits for the rest.
	let frame_id = frame.id();
	let arrival = ctx
		.animation(drawer::shown(0.0), move |frame, v| {
			frame.update_container(frame_id, |c| c.opacity(v.current).transform(look(v.current)))
		})
		.await;
	arrival.update(|arrival| drawer::head(arrival, true, palette::ENTER, palette::EXIT)).await;
	// Whether the highlight shows, so it appears where it belongs instead of sliding in from the first row.
	let mut highlight_shown = false;
	input.request_focus().await;
	ctx.with(move |shell: &mut Shell| shell.palette_height = height).await;
	// The frame's extent the shell was last told.
	let mut known = LayoutExtent::default();

	let mut rows: Option<EvaluationContext<Shell>> = None;
	let mut shown = None;
	// What the palette shows follows the scope's results and a setting typed over it; it closes once it reads `None`.
	let mut state = ctx.proactive(|shell: &Shell| {
		let layers = &shell.layers;
		let setting = layers.setting().map(|setting| setting.revision);
		layers.palette().map(|palette| (palette.revision, layers.revision, setting))
	});
	state.next().await;
	loop {
		let Some(Some(revision)) = state.latest().copied() else {
			break;
		};
		if shown != Some(revision) {
			shown = Some(revision);
			// A shortcut can switch the open palette to another scope, which says what it finds, and a setting opened
			// over it shows its own row and value.
			let Some((results, highlight, search, prompt)) = ctx.with(|shell: &mut Shell| palette_shown(&shell.layers)).await
			else {
				break;
			};
			input.update_text(|t| t.content(&search)).await;
			hint.update_text(|t| t.content(if search.is_empty() { prompt } else { "" }))
				.await;
			if let Some(mut previous) = rows.take() {
				previous.remove().await;
			}
			// Without results there's no list, so no gap opens under the field.
			if !results.is_empty() {
				let mut list = frame
					.element("rows")
					.container(|c| c.width(width.into()).height(Sizing::fit()).flow(flow::column))
					.await;
				for (index, entry) in results.iter().enumerate() {
					palette_row(&mut list, index, entry, width).await;
				}
				rows = Some(list);
			}
			let offset = (!results.is_empty()).then(|| highlight as f32 * palette::ROW_HEIGHT);
			follow(&mut highlight_frame, &slide, &mut highlight_shown, offset).await;
		}
		select! {
			_ = state.next() => {},
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
			// The palette keeps the keyboard while it's open, so a press elsewhere that takes the focus gives it back.
			_ = input.focus_change(true) => input.request_focus().await,
			extent = frame.extent_change(known) => {
				known = extent;
				height = extent.size.y();
				ctx.with(move |shell: &mut Shell| shell.palette_height = height).await;
			},
		}
	}
	// The palette closed: the panel leaves from wherever it stands before it's removed.
	arrival.update(|arrival| drawer::head(arrival, false, palette::ENTER, palette::EXIT)).await;
	arrival.settled().await;
	frame
}

/// Declares a list's highlight under `parent` at `position`, where its first row starts: a selected row's look, hidden
/// until [`follow`] shows it. Declared before the rows, it draws under them.
async fn highlight(
	parent: &mut EvaluationContext<Shell>,
	position: Position,
	width: f32,
	height: f32,
) -> (EvaluationContext<Shell>, Slide) {
	let highlight = parent
		.element("highlight")
		.container(|c| {
			c.position(position)
				.width(width.into())
				.height(height.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.opacity(0.0)
				.style(row_style(true, false))
		})
		.await;
	let id = highlight.id();
	let slide = parent
		.animation(
			// On the curve the camera glides on as it frames an entity, so a highlight and the camera arrive together.
			crate::ui::Animation::new(0.0, drawer::following()),
			move |frame, v| frame.update_container(id, |c| c.transform(Transform::identity().translate_y(v.current))),
		)
		.await;
	(highlight, slide)
}

/// How far a list's highlight stands below the list's first row, which the engine slides it by; see [`highlight`].
type Slide = AnimationHandle<Shell, f32, Smoothing>;

/// Sends a list's highlight `offset` down from its first row, or hides it for `None`. A highlight that wasn't showing
/// appears in place; one that was slides there through `slide`. It moves by its transform, so following the highlight
/// never lays the list out again.
async fn follow(highlight: &mut EvaluationContext<Shell>, slide: &Slide, shown: &mut bool, offset: Option<f32>) {
	match (offset, *shown) {
		(None, _) => {
			highlight.update_container(|c| c.opacity(0.0)).await;
			*shown = false;
		}
		(Some(offset), false) => {
			slide.snap(offset).await;
			highlight
				.update_container(|c| c.opacity(1.0).transform(Transform::identity().translate_y(offset)))
				.await;
			*shown = true;
		}
		(Some(offset), true) => slide.set_target(offset).await,
	}
}

/// The transform that places the palette's panel while `shown` of it shows: it drops in from a little above and grows
/// from a little smaller about its top edge as it arrives, and lifts and shrinks the same way as it leaves.
fn look(shown: f32) -> Transform {
	Transform::identity()
		.translate_y(-10.0 * (1.0 - shown))
		.scale(0.96 + 0.04 * shown)
		.origin(UiPoint::new(0.5, 0.0))
}

/// What the open palette shows: its rows, the highlighted one, the field's text, and what the field says while it's
/// empty. A setting being typed over it shows its own row and value instead.
fn palette_shown(layers: &Layers) -> Option<(Vec<palette::Entry>, usize, String, &'static str)> {
	if let Some(setting) = layers.setting() {
		let prompt = if setting.highlight.is_some() {
			"Type or pick a value"
		} else {
			"Type a new value"
		};
		return Some((
			setting.rows.clone(),
			setting.highlight.unwrap_or(0),
			setting.value.clone(),
			prompt,
		));
	}
	let palette = layers.palette()?;
	Some((
		palette.results.clone(),
		palette.highlight,
		palette.search.clone(),
		palette.scope.prompt(),
	))
}

/// One palette entry: an icon tinted by what it is, its name, and its full ID when the name shortens it. The panel's
/// highlight lights it from behind.
async fn palette_row(parent: &mut EvaluationContext<Shell>, index: usize, entry: &palette::Entry, width: f32) {
	let mut row = parent
		.element(format_args!("{index}"))
		.container(|c| {
			c.width(width.into())
				.height(palette::ROW_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.padding(12.0, 0.0)
				.flow(flow::centered_row_with_gap(10))
		})
		.await;
	let [top, bottom] = kind_tint(entry.kind);
	if icon_view(
		&mut row,
		entry.kind.icon(),
		12.0,
		[ConcreteLayer::default().color(vertical(top, bottom))],
	)
	.await
	.is_none()
	{
		row.element("icon")
			.container(|c| {
				c.width(12.into())
					.height(12.into())
					.corner_radius(3.0)
					.corner_exponent(SQUIRCLE)
					.style(tinted_icon(top, bottom))
			})
			.await;
	}
	row.element("name")
		.text(entry.name(), |t| t.font_size(13.0).style([solid(TEXT)]))
		.await;
	// A setting's value, or what it takes while it's edited, says more than its name repeated would.
	let detail = if entry.detail.is_empty() {
		&entry.label
	} else {
		&entry.detail
	};
	if entry.name() != detail {
		row.element("id")
			.text(detail, |t| t.font_size(11.5).font(VALUE_FONT).style([solid(MUTED)]))
			.await;
	}
}

/// The gradient a palette entry's icon takes, so each kind reads at a glance: meshes and the scene's entities in the
/// accent, as the scene's rows are, indigo prefabs, amber lights, pink materials and their commands, blue textures, gray nodes, teal
/// settings, and violet levels and waypoints.
fn kind_tint(kind: palette::Kind) -> [RGBA; 2] {
	match kind {
		palette::Kind::Mesh | palette::Kind::MeshPreview | palette::Kind::Entity(_) => [ACCENT_LIGHT, ACCENT],
		palette::Kind::Prefab => [hex(0x8A93F0, 1.0), hex(0x5E6AD2, 1.0)],
		palette::Kind::Light(_) => [hex(0xFFD27A, 1.0), hex(0xF5A524, 1.0)],
		palette::Kind::Graph
		| palette::Kind::Variant
		| palette::Kind::Material
		| palette::Kind::NewMaterial
		| palette::Kind::NewVariant => [hex(0xF29BD4, 1.0), hex(0xD9539F, 1.0)],
		palette::Kind::Texture => [hex(0x8CC4FF, 1.0), hex(0x3D8BE0, 1.0)],
		palette::Kind::Node => [hex(0xB9BCC4, 1.0), hex(0x6E737C, 1.0)],
		palette::Kind::Setting(_) => [hex(0x7FE3E0, 1.0), hex(0x2BA8A4, 1.0)],
		palette::Kind::NewLevel
		| palette::Kind::Folder
		| palette::Kind::BrowseGym
		| palette::Kind::Gym
		| palette::Kind::Waypoint(_)
		| palette::Kind::SaveWaypoint => [hex(0xC9A4FF, 1.0), hex(0x8B5CF6, 1.0)],
	}
}

/// One entity's row: it selects the entity when clicked, lifts under the pointer, and follows the entity's name.
async fn row(ctx: &mut EvaluationContext<Shell>, handle: Handle, width: f32) {
	let mut frame = ctx
		.element("row")
		.container(|c| {
			c.width(width.into())
				.height(ROW_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.padding(10.0, 0.0)
				.flow(flow::centered_row_with_gap(8))
				.hit_testable(true)
				.pointer(crate::rendering::Pointer::Hand)
				.style(row_style(false, false))
		})
		.await;
	if icon_view(
		&mut frame,
		icons::ENTITY,
		12.0,
		[ConcreteLayer::default().color(vertical(ACCENT_LIGHT, ACCENT))],
	)
	.await
	.is_none()
	{
		frame
			.element("icon")
			.container(|c| {
				c.width(12.into())
					.height(12.into())
					.corner_radius(3.0)
					.corner_exponent(SQUIRCLE)
					.style(tinted_icon(ACCENT_LIGHT, ACCENT))
			})
			.await;
	}
	let mut label = frame
		.element("label")
		.text("", |t| t.font_size(12.5).style([solid(TEXT)]))
		.await;
	let mut kind = frame
		.element("kind")
		.text("", |t| t.font_size(11.0).style([solid(MUTED)]))
		.await;
	// The scene's revision, which a rename advances, so the row reads its names only when they may have changed.
	let mut revision = ctx.proactive(|shell: &Shell| shell.scene_revision);

	// The list's outline marks the selected row, so a row only lifts under the pointer.
	loop {
		let hovered = select! {
			_ = frame.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| shell.selected = Some(handle)).await;
				continue;
			},
			_ = revision.next() => {
				let names = ctx
					.with(move |shell: &mut Shell| {
						let entity = shell.scene.iter().find(|entity| entity.handle == handle)?;
						Some((entity.label().to_string(), entity.kind.clone()))
					})
					.await;
				if let Some((name, type_name)) = names {
					label.update_text(|t| t.content(&name)).await;
					// The type shows beside the name only when it says something the name doesn't.
					kind.update_text(|t| t.content(if type_name != name { type_name.as_str() } else { "" })).await;
				}
				continue;
			},
			_ = frame.on(Events::PointerEntered) => true,
			_ = frame.on(Events::PointerExited) => false,
		};
		frame.update_container(|c| c.style(row_style(false, hovered))).await;
	}
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
	/// The panel, which fits the blocks it stacks.
	panel: EvaluationContext<Shell>,
	/// The transform block, while the selection has a transform. Its fields follow the selection on their own.
	transform: Option<EvaluationContext<Shell>>,
	/// The settings block and the part it shows, while the selection is a camera or a light.
	part: Option<(Part, EvaluationContext<Shell>)>,
	/// The application's panel blocks the selection shows, and the entity and registered panels they're for; see
	/// [`Shell::panels_of`].
	panels: ((Option<Handle>, u64), Vec<EvaluationContext<Shell>>),
}

impl Fields {
	/// Shows the selected entity's transform block while it has a transform, its settings block while it's a camera or a light, and the application's panels for its component types.
	async fn show(&mut self, root: &mut EvaluationContext<Shell>) {
		let selection = root
			.with(|shell: &mut Shell| {
				let entity = shell.selection()?;
				let shown = shell.panels_of(entity);
				// Each panel is started with its own function, so the list is read here, while the shell is at hand.
				let panels = shell
					.panels
					.iter()
					.enumerate()
					.filter(|(index, _)| shown & 1 << index != 0)
					.map(|(_, panel)| (panel.title, Rc::clone(&panel.mount)))
					.collect::<Vec<_>>();
				Some((entity.label().to_string(), Part::of(entity), entity.handle, shown, panels))
			})
			.await;
		let (selection, handle, shown_panels, panels) = match selection {
			Some((label, part, handle, shown, panels)) => (Some((label, part)), Some(handle), shown, panels),
			None => (None, None, 0, Vec::new()),
		};

		// The scene holds only entities with a transform, so every selection has one.
		let transform = selection.is_some();
		// A settings block belongs to one entity, so it's declared again for another one.
		let part = selection.and_then(|(_, part)| part);
		let part_changed = self.part.as_ref().map(|(shown, _)| *shown) != part;
		let transform_changed = self.transform.is_some() != transform;
		// A panel belongs to one entity, as a settings block does, so another selection starts it again.
		let panels_changed = part_changed || transform_changed || self.panels.0 != (handle, shown_panels);
		if !part_changed && !transform_changed && !panels_changed {
			return;
		}
		// Blocks stack in the order they're declared, so the blocks under one that changes come off too, to be
		// declared again under it: the panels under the settings block, and both under the transform block.
		for mut block in self.panels.1.drain(..) {
			block.remove().await;
		}
		if part_changed || transform_changed {
			if let Some((_, mut block)) = self.part.take() {
				block.remove().await;
			}
		}
		if transform_changed {
			match self.transform.take() {
				Some(mut block) => block.remove().await,
				None => self.transform = Some(transform_block(&mut self.panel).await),
			}
		}
		if let Some(part) = part
			&& self.part.is_none()
		{
			self.part = Some((part, settings_block(&mut self.panel, part).await));
		}
		self.panels.0 = (handle, shown_panels);
		if let Some(handle) = handle {
			for (index, (title, mount)) in panels.into_iter().enumerate() {
				self.panels
					.1
					.push(panel_block(&mut self.panel, handle, index, title, mount).await);
			}
		}
	}
}

/// The properties panel: a block for each part of the selected entity, the entity itself with its name, its
/// transform, and its camera's or light's settings.
async fn properties(root: &mut EvaluationContext<Shell>) -> Fields {
	let mut panel_element = root
		.element("properties")
		.container(|c| {
			c.position(Position::anchored(Anchor::End(MARGIN), Anchor::Start(MARGIN)))
				.width(PROPERTIES_WIDTH.into())
				// As tall as the blocks the selection has, inset like the other panels' content but with the wider
				// gap that sets the blocks apart.
				.height(Sizing::fit())
				.padding(INSET, INSET)
				.flow(flow::column_with_gap(BLOCK_GAP as u32))
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.hit_testable(true)
				.style(panel(1.0))
		})
		.await;
	let width = PROPERTIES_WIDTH - INSET * 2.0;

	// The entity block is the one every selection has, so its header holds the trash button; the other blocks come
	// and go with the parts the entity has.
	let (mut entity_block, mut entity_header) = block(&mut panel_element, "entity", "Entity").await;
	entity_block
		.element("name")
		.component(async move |ctx| name_field(ctx, width).await)
		.await;
	entity_header
		.element("delete")
		.component(async move |ctx| delete_button(ctx, width).await)
		.await;

	Fields {
		panel: panel_element,
		transform: None,
		part: None,
		panels: ((None, 0), Vec::new()),
	}
}

/// The transform block: a row of three fields for each of the position, rotation, and scale.
async fn transform_block(blocks: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	let (mut transform_block, _) = block(blocks, "transform", "Transform").await;
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

/// A block of the properties panel under a header that names it, as tall as the rows the caller adds; returns the
/// block, to which the caller adds the rows, and the header.
async fn block(
	parent: &mut EvaluationContext<Shell>,
	key: &str,
	title: &str,
) -> (EvaluationContext<Shell>, EvaluationContext<Shell>) {
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	let mut block = parent
		.element(key)
		.container(|c| c.width(width.into()).height(Sizing::fit()).flow(flow::column_with_gap(6)))
		.await;
	let header = header(&mut block, title, width).await;
	(block, header)
}

/// A camera's or light's settings block: the part's settings two to a row.
async fn settings_block(blocks: &mut EvaluationContext<Shell>, part: Part) -> EvaluationContext<Shell> {
	let width = PROPERTIES_WIDTH - INSET * 2.0;
	let key = format!("{} {}", part.title, part.handle.id());
	let (mut block, _) = block(blocks, &key, part.title).await;
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

/// An application's panel block: its title over a column as tall as what the panel adds, where the panel's function
/// runs for `entity` until the block is removed.
async fn panel_block(
	blocks: &mut EvaluationContext<Shell>,
	entity: Handle,
	index: usize,
	title: &str,
	mount: crate::editor::panels::Mount,
) -> EvaluationContext<Shell> {
	let key = format!("panel {index} {}", entity.id());
	let (mut block, _) = block(blocks, &key, title).await;
	let mut column = block
		.element("column")
		.container(|c| {
			c.width(CONTENT_WIDTH.into())
				.height(Sizing::fit())
				.flow(flow::column_with_gap(6))
		})
		.await;
	column
		.element("panel")
		.component(async move |ctx| mount(ctx, entity).await)
		.await;
	block
}

/// The trash button, which deletes the selected entity from the game; faint while nothing is selected, and red under
/// the pointer.
async fn delete_button(ctx: &mut EvaluationContext<Shell>, width: f32) {
	const SIZE: f32 = 24.0;
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
				.pointer(crate::rendering::Pointer::Hand)
				.style(row_style(false, false))
		})
		.await;
	let mut icon = icon_view(&mut button, icons::DELETE_ENTITY, 14.0, [solid(MUTED)])
		.await
		.expect("The editor draws its own icon to delete an entity.");

	let mut hovered = false;
	let mut shown = None;
	let mut selected = ctx.proactive(|shell: &Shell| shell.selection().is_some());
	loop {
		select! {
			_ = button.on(Events::Actuated) => {
				ctx.with(|shell: &mut Shell| shell.delete_selection()).await;
			},
			_ = button.on(Events::PointerEntered) => hovered = true,
			_ = button.on(Events::PointerExited) => hovered = false,
			_ = selected.next() => {},
		}
		let selected = selected.latest().copied().unwrap_or_default();
		let look = (selected, hovered && selected);
		if shown != Some(look) {
			shown = Some(look);
			let color = match look {
				(false, _) => hex(0x8A8F98, 0.35),
				(true, true) => AXIS_COLORS[0],
				(true, false) => MUTED,
			};
			button.update_container(|c| c.style(row_style(false, look.1))).await;
			icon.tint(color).await;
		}
	}
}

/// Draws the icon for `purpose` under `parent`, `size` units wide, its plain layers filled with `fill`, and returns it
/// to recolor; `None` when no icon goes there, so the place draws its own tile. See [`crate::editor::icons`].
pub(crate) async fn icon_view<const N: usize>(
	parent: &mut EvaluationContext<Shell>,
	purpose: &'static str,
	size: f32,
	fill: [ConcreteLayer; N],
) -> Option<VectorView<Shell>> {
	let icons = parent.with(|shell: &mut Shell| shell.icons.clone()).await;
	let icon = icons::find(&icons, purpose)?;
	Some(draw_vector(parent, "icon", icon, size, fill).await)
}

/// The selected entity's name, in a well that takes a new name when clicked, starting from what it shows: the name, or
/// the type of an entity without one. A dash shows without a selection; an empty name changes nothing, since a name
/// can't be empty.
async fn name_field(ctx: &mut EvaluationContext<Shell>, width: f32) {
	let mut well = ctx
		.element("well")
		.container(|c| {
			c.width(width.into())
				.height(FIELD_HEIGHT.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.clip(true)
				.padding(8.0, 0.0)
				.flow(flow::centered_row_with_gap(6))
				.hit_testable(true)
				.pointer(crate::rendering::Pointer::Text)
				.style(setting_style(true, false))
		})
		.await;
	let mut text = well
		.element("name")
		.text("–", |t| t.font_size(12.0).style([solid(TEXT)]))
		.await;
	let mut hovered = false;
	let mut shown_hover = false;
	let mut shown = None;
	// What the well shows and typing starts from, and the name it replaces.
	let mut current = ctx.proactive(|shell: &Shell| {
		shell
			.selection()
			.map(|entity| (entity.label().to_owned(), entity.name.clone()))
	});
	loop {
		select! {
			_ = well.on(Events::Actuated) => {
				let Some((label, name)) = current.latest().cloned().flatten() else {
					continue;
				};
				text.update_text(|t| t.content("")).await;
				let styles = [typing_style(true), setting_style(true, false)];
				let typed = read_typed(ctx, &mut well, styles, 12.0, &label).await;
				if let Some(typed) = typed.map(|typed| typed.trim().to_owned()).filter(|typed| !typed.is_empty() && *typed != name) {
					ctx.with(move |shell: &mut Shell| shell.edit_selection(Change::Name(typed))).await;
				}
				shown = None;
			},
			_ = well.on(Events::PointerEntered) => hovered = true,
			_ = well.on(Events::PointerExited) => hovered = false,
			_ = current.next() => {},
		}
		let label = current.latest().cloned().flatten().map(|(label, _)| label);
		if shown != Some(label.clone()) {
			text.update_text(|t| t.content(label.as_deref().unwrap_or("–"))).await;
			shown = Some(label);
		}
		if hovered != shown_hover {
			shown_hover = hovered;
			well.update_container(|c| c.style(setting_style(true, hovered))).await;
		}
	}
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
				.padding(8.0, 0.0)
				.flow(flow::centered_row_with_gap(6))
				.hit_testable(true)
				.pointer(crate::rendering::Pointer::Text)
				.style(setting_style(framed, false))
		})
		.await;
	let label_color = setting.axis().map_or(MUTED, |axis| AXIS_COLORS[axis]);
	well.element("label")
		.text(setting.label, |t| t.font_size(11.0).style([solid(label_color)]))
		.await;
	let mut value = well
		.element("value")
		.text("–", |t| t.font_size(12.0).font(VALUE_FONT).style([solid(TEXT)]))
		.await;

	// The selection when the pointer pressed, which a scrub measures from, and the setting's value then.
	let mut pressed: Option<(SceneEntity, Option<f32>)> = None;
	let mut dragged = false;
	let mut hovered = false;
	let mut shown_hover = false;
	// The value shown, `None` before the first frame; the inner `None` is no selection or an unset value.
	let mut shown: Option<Option<Option<f32>>> = None;
	let mut text = String::new();
	let mut current = ctx.proactive(move |shell: &Shell| shell.selection().map(|entity| setting.value(entity)));
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
			_ = current.next() => {},
		}
		// The value is formatted only when it changed.
		let current = current.latest().copied().flatten();
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
	let Some(value) = read_typed(ctx, well, styles, 12.0, "")
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

/// Reads text typed into `well`, starting from `initial`, which takes the keyboard's focus until Enter returns the
/// text, or Escape or a press outside the well returns `None`. The well wears the first of `styles` while it takes text, and the second after.
async fn read_typed(
	ctx: &mut EvaluationContext<Shell>,
	well: &mut EvaluationContext<Shell>,
	styles: [[ConcreteLayer; 3]; 2],
	font_size: f32,
	initial: &str,
) -> Option<String> {
	let [typing, rest] = styles;
	// While text is typed, the field takes Enter and Escape, so Escape cancels the typing instead of popping a layer.
	ctx.with(|shell: &mut Shell| shell.layers.push(Layer::Typing)).await;
	well.update_container(|c| c.style(typing)).await;
	let mut input = well
		.element("input")
		.text_field(initial, |t| t.font_size(font_size).style([solid(TEXT)]))
		.await;
	input.request_focus().await;
	let mut typed = initial.to_owned();
	let apply = loop {
		select! {
			edit = input.on_text_edit() => {
				edit.edit.apply_to(&mut typed);
				input.update_text(|t| t.content(&typed)).await;
			},
			_ = input.on_key(Key::Enter) => break true,
			_ = input.on_key(Key::Escape) => break false,
			// A press outside the well takes the focus, which cancels the typing.
			_ = input.focus_change(true) => break false,
		}
	};
	// Removing the field takes its focus with it.
	input.remove().await;
	ctx.with(|shell: &mut Shell| shell.layers.close(|layer| matches!(layer, Layer::Typing)))
		.await;
	well.update_container(|c| c.style(rest)).await;
	apply.then_some(typed)
}

/// The floating toolbar over the game view: the gizmo's tools, the chosen one pressed. It shows only while an entity is
/// selected, since the tools act on the selection: it slides down into place as one is selected, and back up and out
/// once none is, as a drawer does; see [`drawer::head`].
async fn toolbar(ctx: &mut EvaluationContext<Shell>) {
	let mut toolbar = ctx
		.element("toolbar")
		.container(|c| {
			c.position(Position::anchored(Anchor::Center(0.0), Anchor::Start(MARGIN)))
				// As wide as its tools, with the same room beside them as above and below.
				.width(Sizing::fit())
				.height(TOOLBAR_HEIGHT.into())
				.padding((TOOLBAR_HEIGHT - TOOL_HEIGHT) * 0.5, 0.0)
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.flow(flow::centered_row_with_gap(4))
				.hit_testable(true)
				// Translucent and blurred, so the game shows through it.
				.style(panel(0.72))
				.transform(toolbar_transform(0.0))
				.opacity(0.0)
		})
		.await;
	for tool in Tool::ALL {
		toolbar
			.element(tool.label())
			.component(async move |ctx| tool_button(ctx, tool).await)
			.await;
	}

	// The engine slides the toolbar in as an entity is selected and out once none is.
	let id = toolbar.id();
	let shown = ctx
		.animation(drawer::shown(0.0), move |frame, v| {
			frame.update_container(id, |c| c.transform(toolbar_transform(v.current)).opacity(v.current))
		})
		.await;
	let mut selected = ctx.proactive(|shell: &Shell| shell.selection().is_some());
	loop {
		// A selection that changes before the toolbar arrives turns it around from where it is.
		let selected = *selected.next().await;
		shown.update(|shown| drawer::head(shown, selected, drawer::ENTER, drawer::EXIT)).await;
	}
}

/// The transform that shows `shown` of the toolbar, from 0, hidden above the window's top edge, to 1, in place.
fn toolbar_transform(shown: f32) -> Transform {
	// It rises past the margin above it and its own height, its shadow included.
	Transform::identity().translate_y((shown - 1.0) * (MARGIN + TOOLBAR_HEIGHT + 16.0))
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
				.pointer(crate::rendering::Pointer::Hand)
				.style(tool_style(false, false))
		})
		.await;
	let mut label = button
		.element("label")
		.text(tool.label(), |t| t.font_size(13.0).style([solid(TOOLBAR_MUTED)]))
		.await;

	let mut hovered = false;
	let mut shown = None;
	let mut chosen = ctx.proactive(move |shell: &Shell| shell.tool == tool);
	loop {
		select! {
			_ = button.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| shell.tool = tool).await;
			},
			_ = button.on(Events::PointerEntered) => hovered = true,
			_ = button.on(Events::PointerExited) => hovered = false,
			_ = chosen.next() => {},
		}
		let chosen = chosen.latest().copied().unwrap_or_default();
		if shown != Some((chosen, hovered)) {
			shown = Some((chosen, hovered));
			button.update_container(|c| c.style(tool_style(chosen, hovered))).await;
			let color = if chosen || hovered { TEXT } else { TOOLBAR_MUTED };
			label.update_text(|t| t.style([solid(color)])).await;
		}
	}
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

/// A raised surface lit from above, with its fill at `alpha`; below 1 it also blurs what is behind.
fn panel(alpha: f32) -> [ConcreteLayer; 4] {
	[
		ConcreteLayer::default()
			.color(hex(0x000000, 0.35).into())
			.drop_shadow([0.0, 6.0], 12.0),
		ConcreteLayer::default()
			.color(vertical(hex(0x26272D, alpha), hex(0x1E1F24, alpha)))
			.backdrop_blur(if alpha < 1.0 { 16.0 } else { 0.0 }),
		top_highlight(0.06),
		hairline(0.5),
	]
}

/// A control pushed into the surface: darker, lit from below, with the shadow inside.
fn pressed() -> [ConcreteLayer; 3] {
	[
		ConcreteLayer::default().color(vertical(hex(0x1A1B1F, 1.0), hex(0x222328, 1.0))),
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
	if chosen { pressed() } else { row_style(false, hovered) }
}

/// A surface raised above the panel it sits on, lit from above and edged in a dark hairline, so it reads as a key or
/// a button to press: a keycap in the hint bar, an application panel's button. It brightens under the pointer.
pub(crate) fn raised(hovered: bool) -> [ConcreteLayer; 3] {
	let [top, bottom] = if hovered { [0x45464E, 0x34353C] } else { [0x3A3B42, 0x2C2D33] };
	[
		ConcreteLayer::default().color(vertical(hex(top, 1.0), hex(bottom, 1.0))),
		top_highlight(0.1),
		hairline(0.6),
	]
}

/// A scene row: clear at rest, a faint lift under the pointer, and, while selected, tinted with the accent and edged
/// in it, as the docs site marks what's active, so the light text stays readable over the brand's own green.
fn row_style(selected: bool, hovered: bool) -> [ConcreteLayer; 3] {
	if selected {
		[
			ConcreteLayer::default().color(vertical(hex(ACCENT_RGB, 0.22), hex(ACCENT_RGB, 0.14))),
			top_highlight(0.08),
			ConcreteLayer::default().color(hex(ACCENT_RGB, 0.55).into()).stroke(1.0),
		]
	} else {
		// A transparent fill keeps the row a surface the pointer can reach while it paints nothing.
		let fill = if hovered { hex(0xFFFFFF, 0.05) } else { hex(0xFFFFFF, 0.0) };
		[solid(fill), top_highlight(if hovered { 0.04 } else { 0.0 }), hairline(0.0)]
	}
}

/// A small raised tile lit from `top` to `bottom`.
fn tinted_icon(top: RGBA, bottom: RGBA) -> [ConcreteLayer; 3] {
	[
		ConcreteLayer::default().color(vertical(top, bottom)),
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

/// A channel cut into the panel, as a switch or a slider runs in: dark and shadowed from above, with light catching its bottom lip, and
/// filled with the accent as far as `on`, from 0 off to 1 on.
fn channel(on: f32) -> [ConcreteLayer; 5] {
	[
		solid(hex(0x0E0F11, 1.0)),
		// The accent fill, darker toward the top, where the channel's lip shades it.
		ConcreteLayer::default().color(vertical(hex(darker(ACCENT_RGB), on), hex(ACCENT_RGB, on))),
		// The channel's depth: a shadow cast down from its upper edge.
		ConcreteLayer::default()
			.color(hex(0x000000, 0.75).into())
			.inset_shadow([0.0, 2.0], 2.5),
		// The light the panel's surface catches on the channel's lower lip.
		ConcreteLayer::default()
			.color(hex(0xFFFFFF, 0.08).into())
			.inset_shadow([0.0, -1.0], 0.0),
		hairline(0.7),
	]
}

/// A knob, as a switch or a slider moves: a dome lit from above, bright at its crown and shaded toward its base, that casts a
/// soft shadow into the channel under it.
fn cap() -> [ConcreteLayer; 4] {
	[
		ConcreteLayer::default()
			.color(hex(0x000000, 0.6).into())
			.drop_shadow([0.0, 1.5], 1.5),
		ConcreteLayer::default().color(vertical(hex(0xF7F7F9, 1.0), hex(0xA9ADB6, 1.0))),
		top_highlight(0.9),
		hairline(0.45),
	]
}

/// The `0xRRGGBB` color `rgb` at two thirds of its brightness.
fn darker(rgb: u32) -> u32 {
	let channel = |shift: u32| ((rgb >> shift) & 0xFF) * 2 / 3;
	channel(16) << 16 | channel(8) << 8 | channel(0)
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

/// A top-to-bottom gradient over the whole element, whatever its height.
fn vertical(top: RGBA, bottom: RGBA) -> crate::ui::Color {
	LinearGradient::relative([0.0, 0.0], [0.0, 1.0], top, bottom).into()
}

pub(crate) fn solid(color: RGBA) -> ConcreteLayer {
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

#[cfg(test)]
mod tests {
	use super::darker;

	#[test]
	fn the_editor_asks_for_frames_only_while_its_drawer_moves() {
		use crate::editor::layers::Layer;
		use crate::time::MediaTime;
		use crate::ui::{Engine, Size};

		let size = Size::new(800, 600);
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(super::Shell::new(size, super::icons::merged(&[])));
		engine.mount(async |ctx| super::shell(ctx).await);
		// Settles the editor one tenth of a second at a time, and returns how many frames it asked for.
		let settle = |engine: &mut Engine<super::Shell>| {
			let mut frames = 0;
			while engine.next_tick().is_some_and(|tick| tick <= std::time::Instant::now()) {
				assert!(frames < 20, "The editor kept asking for frames after its drawer arrived.");
				engine.advance_clock(MediaTime::from_seconds_f32(0.1));
				engine.evaluate(size, &allocator);
				frames += 1;
			}
			frames
		};
		engine.evaluate(size, &allocator);
		settle(&mut engine);
		assert!(engine.ctx().drawer_gone, "A closed editor's drawer isn't gone.");

		engine.ctx_mut().layers.push(Layer::Scene);
		assert!(settle(&mut engine) > 1, "The drawer didn't move in over several frames.");
		assert!(!engine.ctx().drawer_gone, "An open editor's drawer is gone.");

		engine.ctx_mut().layers.pop();
		assert!(settle(&mut engine) > 1, "The drawer didn't move out over several frames.");
		assert!(engine.ctx().drawer_gone, "The drawer isn't gone once it left.");
	}

	#[test]
	fn an_entity_shows_the_panels_of_the_components_it_has() {
		use std::rc::Rc;

		use crate::core::factory::Factory;
		use crate::editor::panels::Registration;
		use crate::editor::scene::SceneEntity;
		use crate::gameplay::Transform;

		let registration = |component: &'static str| Registration {
			component,
			title: component,
			mount: Rc::new(|_, _| Box::pin(async {})),
		};
		let mut shell = super::Shell::new(crate::ui::Size::new(800, 600), super::icons::merged(&[]));
		shell.panels = vec![
			registration("game::Npc"),
			registration("game::Door"),
			registration("game::Health"),
		];
		let npc = SceneEntity {
			handle: Factory::<u8>::new().create(0),
			name: String::new(),
			kind: String::new(),
			transform: Transform::identity(),
			camera: None,
			light: None,
			types: vec!["game::Health", "game::Npc"],
			properties: Vec::new(),
		};
		assert_eq!(
			shell.panels_of(&npc),
			0b101,
			"in registration order, whatever order the entity has them in"
		);
		assert_eq!(
			shell.panels_of(&SceneEntity {
				types: Vec::new(),
				..npc
			}),
			0
		);
	}

	#[test]
	fn a_darker_color_keeps_its_hue() {
		assert_eq!(darker(0x41D866), 0x2B9044);
		assert_eq!(darker(0xFFFFFF), 0xAAAAAA);
	}
}
