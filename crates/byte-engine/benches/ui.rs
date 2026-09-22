//! Guard the UI engine's frame loop against regressions with
//! `cargo bench -p byte-engine --bench ui`.
//!
//! Every workload is a screen that games ship: a settings menu, a gameplay
//! HUD, a leaderboard, a kanban board, a sign-in form, and a tooltip sliding
//! over a grid of static cards. A timed iteration
//! runs whole application ticks the way `sandbox/isometric` does, so results
//! read as frame costs against a 16.6 ms budget:
//!
//! 1. Queue the tick's pointer, wheel, and text input on the [`Engine`].
//! 2. Call [`Engine::evaluate`] with a frame allocator reset every tick.
//! 3. Retain hit geometry for the next tick's input.
//! 4. Call [`Engine::render`] and clone the render only when its revision moved.
//!
//! Iterations are cyclic: each one leaves the screen equivalent to where it
//! started, so every sample measures the same work. Fixture construction and
//! the cold first frame, which loads the system font, stay outside timing.
//! Setup replays one iteration and asserts that it took effect, so a broken
//! interaction fails loudly instead of benchmarking an idle frame. Allocation
//! counts print next to times; the profiler adds a small, constant overhead.
//!
//! Compare runs with the same machine, power state, and toolchain, and read
//! the median with its spread rather than the fastest sample.

use std::cell::Cell;

use byte_engine::{
	ui::{
		ConcreteLayer, ConcreteStyle, Container, ContainerContext as _, Context, Depth, ElementContext as _, Engine,
		EvaluationContext, Id, Key, Render, RenderRevision, Size, Sizing, Text, TextField, Transform, UiFuture, UiPoint,
		UiVector, flow, intersection::HitTest, primitive::Events,
	},
	utils::{RGBA, r#async::select_biased},
};
use divan::{Bencher, counter::ItemsCount};

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
	divan::main();
}

const WIDTH: f32 = 1920.0;
const HEIGHT: f32 = 1080.0;

/// The `Window` struct keeps what an application retains between UI ticks.
struct Window<C: 'static> {
	engine: Engine<C>,
	hits: HitTest,
	frame_allocator: bumpalo::Bump,
	size: Size,
	published: Option<RenderRevision>,
}

impl<C: 'static> Window<C> {
	/// Mounts a screen and runs its cold first frame.
	fn new<F>(model: C, root: F) -> Self
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> UiFuture<'ctx> + 'static,
	{
		let mut engine = Engine::with_context(model);
		engine.mount(root);
		let mut window = Self {
			engine,
			hits: HitTest::default(),
			frame_allocator: bumpalo::Bump::new(),
			size: Size::new(WIDTH, HEIGHT),
			published: None,
		};
		assert!(
			window.tick(),
			"The first UI frame published nothing. The most likely cause is a screen that failed to build."
		);
		window
	}

	fn model(&self) -> &C {
		self.engine.ctx()
	}

	/// Runs one application tick and returns whether it published a new render.
	fn tick(&mut self) -> bool {
		self.frame_allocator.reset();
		let mut snapshot = self.engine.evaluate(self.size, &self.frame_allocator);
		snapshot.retain_hit_test(&mut self.hits);
		let render = self.engine.render(&mut snapshot);
		if self.published == Some(render.revision()) {
			return false;
		}
		self.published = Some(render.revision());
		divan::black_box::<Render>(render.clone());
		true
	}

	fn ticks(&mut self, count: usize) {
		for _ in 0..count {
			self.tick();
		}
	}

	/// Converts a layout position to the normalized window coordinates input arrives in.
	fn normalized(&self, point: UiPoint) -> UiPoint {
		UiPoint::new(point.x / self.size.x() * 2.0 - 1.0, 1.0 - point.y / self.size.y() * 2.0)
	}

	/// Returns the center of a surface as the last published frame drew it, in layout units.
	fn center(&self, id: Id) -> UiPoint {
		let bounds = self
			.hits
			.bounds(id)
			.expect("A benchmark target is not on screen. The most likely cause is a layout change that moved it out of view.");
		UiPoint::new(bounds.x() + bounds.width() * 0.5, bounds.y() + bounds.height() * 0.5)
	}

	fn point_at(&mut self, point: UiPoint) {
		let position = self.normalized(point);
		self.engine.set_cursor_position(position);
	}

	/// Presses and releases the primary button at a layout position for the next tick.
	fn click_at(&mut self, point: UiPoint) {
		self.point_at(point);
		self.engine.update_click_state(true);
		self.engine.update_click_state(false);
	}

	fn click(&mut self, id: Id) {
		self.click_at(self.center(id));
	}
}

fn fill(r: f32, g: f32, b: f32, a: f32) -> ConcreteLayer {
	ConcreteLayer::default().color(RGBA::new(r, g, b, a).into())
}

/// A filled surface with a hairline border, the common panel and card look.
fn panel(r: f32, g: f32, b: f32) -> ConcreteStyle {
	ConcreteStyle::new()
		.layer(fill(r, g, b, 1.0))
		.layer(fill(1.0, 1.0, 1.0, 0.12).stroke(1.0))
}

fn text_color() -> ConcreteLayer {
	fill(0.92, 0.94, 0.97, 1.0)
}

fn muted_color() -> ConcreteLayer {
	fill(0.55, 0.6, 0.68, 1.0)
}

/// A non-interactive box that sizes and places one line of text.
fn cell<C: 'static>(parent: &mut EvaluationContext<C>, width: f32, height: f32, text: Text) -> EvaluationContext<C> {
	parent
		.element("cell")
		.container(
			Container::default()
				.width(width.into())
				.height(height.into())
				.flow(flow::centered_row)
				.hit_testable(false)
				.style(ConcreteStyle::new()),
		)
		.text(text)
}

fn emphasized_out(t: f32) -> f32 {
	1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
}

/// A settings menu: category sidebar, a clipped list of option rows with toggles, and a confirmation dialog.
mod settings {
	use super::*;

	const CATEGORIES: [&str; 8] = [
		"Gameplay",
		"Controls",
		"Key Bindings",
		"Audio",
		"Display",
		"Graphics",
		"Accessibility",
		"Network",
	];

	/// Option labels from a shipped graphics menu. Each category rotates them so switching changes every row.
	const OPTIONS: [&str; 40] = [
		"Window Mode",
		"Resolution",
		"Refresh Rate",
		"V-Sync",
		"Frame Rate Limit",
		"Field of View",
		"Brightness",
		"HDR",
		"Render Scale",
		"Upscaling",
		"Sharpening",
		"Anti-Aliasing",
		"Texture Quality",
		"Texture Filtering",
		"Shadow Quality",
		"Shadow Distance",
		"Ambient Occlusion",
		"Global Illumination",
		"Reflections",
		"Screen Space Reflections",
		"Volumetric Fog",
		"Motion Blur",
		"Depth of Field",
		"Bloom",
		"Lens Flare",
		"Film Grain",
		"Chromatic Aberration",
		"Vignette",
		"Foliage Density",
		"Grass Distance",
		"Level of Detail",
		"Particle Quality",
		"Water Quality",
		"Cloud Quality",
		"Crowd Density",
		"Decal Quality",
		"Tessellation",
		"Ray Traced Shadows",
		"Show FPS Counter",
		"Reduce Input Latency",
	];

	const ROW_HEIGHT: f32 = 56.0;
	const LIST_HEIGHT: f32 = 800.0;
	const ENTER_FRAMES: usize = 12;
	const EXIT_FRAMES: usize = 6;

	/// The `Model` struct shares the menu's selection and benchmark targets with the application.
	#[derive(Default)]
	struct Model {
		category: Cell<usize>,
		categories: Cell<[Option<Id>; CATEGORIES.len()]>,
		/// The toggle of the sixth option in the current list.
		toggle: Cell<Option<Id>>,
		list: Cell<Option<Id>>,
		reset: Cell<Option<Id>>,
		cancel: Cell<Option<Id>>,
		dialog_open: Cell<bool>,
	}

	fn window() -> Window<Model> {
		Window::new(Model::default(), |ctx| Box::pin(screen(ctx)))
	}

	async fn screen(ctx: &mut EvaluationContext<Model>) {
		let mut root = ctx.element("root").container(
			Container::default()
				.flow(flow::row)
				.hit_testable(false)
				.style(fill(0.05, 0.06, 0.08, 1.0)),
		);
		let mut sidebar = root.element("sidebar").container(
			Container::default()
				.width(280.0.into())
				.flow(flow::column_with_gap(4))
				.hit_testable(false)
				.style(panel(0.08, 0.09, 0.11)),
		);
		cell(
			&mut sidebar,
			280.0,
			72.0,
			Text::new("Settings").font_size(28.0).style(text_color()),
		);
		for index in 0..CATEGORIES.len() {
			sidebar
				.element("category")
				.component(move |ctx| Box::pin(category_button(ctx, index)));
		}

		let mut content = root.element("content").container(
			Container::default()
				.width(Sizing::Relative(3, 4))
				.flow(flow::column_with_gap(16))
				.hit_testable(false)
				.style(ConcreteStyle::new()),
		);
		let mut header = content.element("header").container(
			Container::default()
				.height(72.0.into())
				.flow(flow::centered_row)
				.hit_testable(false)
				.style(ConcreteStyle::new()),
		);
		let mut title = cell(
			&mut header,
			1000.0,
			72.0,
			Text::new(CATEGORIES[0]).font_size(32.0).style(text_color()),
		);
		header.element("reset").component(|ctx| Box::pin(reset_button(ctx)));

		loop {
			let category = ctx.ctx().category.get();
			title.update_text(|text| text.set_content(CATEGORIES[category]));
			content
				.element("list")
				.mount(move |ctx| Box::pin(option_list(ctx, category)))
				.await;
		}
	}

	async fn category_button(ctx: &mut EvaluationContext<Model>, index: usize) {
		let style = |selected: bool| {
			if selected {
				panel(0.2, 0.32, 0.55)
			} else {
				ConcreteStyle::new().layer(fill(0.0, 0.0, 0.0, 0.0))
			}
		};
		let mut button = ctx.element("button").container(
			Container::default()
				.width(264.0.into())
				.height(44.0.into())
				.corner_radius(8.0)
				.flow(flow::centered_row)
				.style(style(index == 0)),
		);
		button.element("label").text(
			Text::new(CATEGORIES[index])
				.font_size(17.0)
				.style(text_color())
				.transform(Transform::identity().translate_x(16.0)),
		);
		let mut ids = ctx.ctx().categories.get();
		ids[index] = Some(button.id());
		ctx.ctx().categories.set(ids);

		// Every button follows the shared selection, so a click restyles the old and new choice.
		let mut selected = index == 0;
		loop {
			select_biased! {
				_ = button.on(Events::Actuated) => ctx.ctx().category.set(index),
				_ = ctx.render() => {},
			}
			if selected != (ctx.ctx().category.get() == index) {
				selected = !selected;
				button.update_container(|button| button.set_style(style(selected)));
			}
		}
	}

	/// Shows one category's options until another category is selected.
	async fn option_list(ctx: &mut EvaluationContext<Model>, category: usize) {
		let mut viewport = ctx.element("viewport").container(
			Container::default()
				.height(LIST_HEIGHT.into())
				.corner_radius(12.0)
				.flow(flow::column)
				.style(panel(0.07, 0.08, 0.1)),
		);
		ctx.ctx().list.set(Some(viewport.id()));
		let mut rows = viewport.element("rows").container(
			Container::default()
				.height((ROW_HEIGHT * OPTIONS.len() as f32).into())
				.flow(flow::column)
				.hit_testable(false)
				.style(ConcreteStyle::new()),
		);
		for index in 0..OPTIONS.len() {
			let label = OPTIONS[(index + category * 5) % OPTIONS.len()];
			rows.element("option")
				.component(move |ctx| Box::pin(option_row(ctx, index, label)));
		}

		let max_scroll = ROW_HEIGHT * OPTIONS.len() as f32 - LIST_HEIGHT;
		let mut scroll = 0.0f32;
		loop {
			select_biased! {
				event = viewport.on(Events::Scrolled) => {
					let delta = event.delta.map_or(0.0, |delta| delta.y);
					scroll = (scroll - delta * ROW_HEIGHT).clamp(0.0, max_scroll);
					rows.update_container(|rows| rows.set_transform(Transform::identity().translate_y(-scroll)));
				},
				_ = ctx.render() => if ctx.ctx().category.get() != category {
					return;
				},
			}
		}
	}

	async fn option_row(ctx: &mut EvaluationContext<Model>, index: usize, label: &'static str) {
		let state = |on: bool| if on { "On" } else { "Off" };
		let track_style = |on: bool| {
			if on { panel(0.25, 0.6, 0.95) } else { panel(0.18, 0.2, 0.24) }
		};
		let knob_offset = |on: bool| Transform::identity().translate_x(if on { 27.0 } else { 3.0 });
		let mut on = index % 3 != 1;

		let mut row = ctx.element("row").container(
			Container::default()
				.height(ROW_HEIGHT.into())
				.flow(flow::centered_row)
				.hit_testable(false)
				.style(fill(1.0, 1.0, 1.0, if index.is_multiple_of(2) { 0.02 } else { 0.0 })),
		);
		cell(
			&mut row,
			900.0,
			ROW_HEIGHT,
			Text::new(label)
				.font_size(18.0)
				.style(text_color())
				.transform(Transform::identity().translate_x(24.0)),
		);
		let mut value = cell(
			&mut row,
			120.0,
			ROW_HEIGHT,
			Text::new(state(on)).font_size(16.0).style(muted_color()),
		);
		let mut track = row.element("toggle").container(
			Container::default()
				.width(52.0.into())
				.height(28.0.into())
				.corner_radius(14.0)
				.flow(flow::centered_row)
				.style(track_style(on)),
		);
		let mut knob = track.element("knob").container(
			Container::default()
				.width(22.0.into())
				.height(22.0.into())
				.corner_radius(11.0)
				.hit_testable(false)
				.transform(knob_offset(on))
				.style(fill(0.96, 0.97, 0.98, 1.0)),
		);
		if index == 5 {
			ctx.ctx().toggle.set(Some(track.id()));
		}

		loop {
			track.on(Events::Actuated).await;
			on = !on;
			value.update_text(|text| text.set_content(state(on)));
			track.update_container(|track| track.set_style(track_style(on)));
			knob.update_container(|knob| knob.set_transform(knob_offset(on)));
		}
	}

	async fn reset_button(ctx: &mut EvaluationContext<Model>) {
		let mut button = ctx.element("button").container(
			Container::default()
				.width(200.0.into())
				.height(44.0.into())
				.corner_radius(8.0)
				.flow(flow::center)
				.style(panel(0.16, 0.18, 0.22)),
		);
		button
			.element("label")
			.text(Text::new("Reset to Defaults").font_size(16.0).style(text_color()));
		ctx.ctx().reset.set(Some(button.id()));
		loop {
			button.on(Events::Actuated).await;
			ctx.element("dialog").mount(|ctx| Box::pin(confirm_dialog(ctx))).await;
		}
	}

	/// Fades a modal in over the menu, waits for a choice, and fades it out.
	async fn confirm_dialog(ctx: &mut EvaluationContext<Model>) {
		ctx.ctx().dialog_open.set(true);
		let mut backdrop = ctx.element("backdrop").container(
			Container::default()
				.depth(Depth::absolute(1))
				.flow(flow::center)
				.style(fill(0.0, 0.0, 0.0, 0.0)),
		);
		let mut dialog = backdrop.element("dialog").container(
			Container::default()
				.width(640.0.into())
				.height(260.0.into())
				.corner_radius(24.0)
				.flow(flow::centered_column)
				.style(panel(0.1, 0.11, 0.14))
				.opacity(0.0),
		);
		cell(
			&mut dialog,
			600.0,
			72.0,
			Text::new("Reset all settings?").font_size(28.0).style(text_color()),
		);
		cell(
			&mut dialog,
			600.0,
			72.0,
			Text::new("Every option in every category returns to its default value.")
				.font_size(16.0)
				.style(muted_color()),
		);
		let mut buttons = dialog.element("buttons").container(
			Container::default()
				.width(416.0.into())
				.height(56.0.into())
				.flow(flow::row_with_gap(16))
				.hit_testable(false)
				.style(ConcreteStyle::new()),
		);
		let mut choice = |name: &'static str, label: &'static str, style: ConcreteStyle| {
			let mut button = buttons.element(name).container(
				Container::default()
					.width(200.0.into())
					.height(48.0.into())
					.corner_radius(10.0)
					.flow(flow::center)
					.style(style),
			);
			button
				.element("label")
				.text(Text::new(label).font_size(18.0).style(text_color()));
			button
		};
		let mut cancel = choice("cancel", "Cancel", panel(0.16, 0.18, 0.22));
		let mut confirm = choice("confirm", "Reset", panel(0.7, 0.2, 0.2));
		ctx.ctx().cancel.set(Some(cancel.id()));

		// Frame-counted easing keeps every round trip the same length regardless of wall-clock time.
		let mut present = |t: f32| {
			backdrop.update_container(|backdrop| backdrop.set_style(fill(0.0, 0.0, 0.0, 0.6 * t)));
			dialog.update_container(|dialog| {
				dialog.set_transform(Transform::identity().translate_y((1.0 - t) * 24.0).scale(0.965 + 0.035 * t));
				dialog.set_opacity(t);
			});
		};
		for frame in 1..=ENTER_FRAMES {
			present(emphasized_out(frame as f32 / ENTER_FRAMES as f32));
			ctx.render().await;
		}
		select_biased! {
			_ = cancel.on(Events::Actuated) => {},
			_ = confirm.on(Events::Actuated) => {},
		}
		for frame in 1..=EXIT_FRAMES {
			present(1.0 - frame as f32 / EXIT_FRAMES as f32);
			ctx.render().await;
		}
		ctx.ctx().dialog_open.set(false);
	}

	/// Measures the frame a menu costs while nobody touches it.
	#[divan::bench]
	fn idle(bencher: Bencher) {
		let mut window = window();
		window.tick();
		assert!(!window.tick(), "An untouched settings menu republished its render.");
		bencher.bench_local(|| window.tick());
	}

	/// Measures flipping one option's toggle: a click, a restyled track, a moved knob, and a new label.
	#[divan::bench]
	fn toggle_option(bencher: Bencher) {
		let mut window = window();
		let toggle = window
			.model()
			.toggle
			.get()
			.expect("The option list did not report its toggle.");
		window.click(toggle);
		assert!(window.tick(), "Clicking an option's toggle changed nothing.");
		bencher.bench_local(|| {
			window.click(toggle);
			window.tick()
		});
	}

	/// Measures one wheel notch over the option list, scrolling down and back up.
	#[divan::bench]
	fn scroll_list(bencher: Bencher) {
		const NOTCHES: usize = 8;
		let mut window = window();
		let list = window
			.model()
			.list
			.get()
			.expect("The option list did not report its viewport.");
		window.point_at(window.center(list));
		window.engine.update_scroll_state(UiVector::new(0.0, -1.0));
		assert!(window.tick(), "Scrolling the option list changed nothing.");
		window.engine.update_scroll_state(UiVector::new(0.0, 1.0));
		window.tick();

		let mut notch = 0;
		bencher.bench_local(|| {
			let direction = if notch % (2 * NOTCHES) < NOTCHES { -1.0 } else { 1.0 };
			window.engine.update_scroll_state(UiVector::new(0.0, direction));
			notch += 1;
			window.tick()
		});
	}

	/// Measures selecting another category: its button restyles, then 40 option rows unmount and mount.
	#[divan::bench]
	fn switch_category(bencher: Bencher) {
		let mut window = window();
		let buttons = window
			.model()
			.categories
			.get()
			.map(|button| button.expect("A category button did not report its identity."));
		window.click(buttons[1]);
		window.ticks(2);
		assert_eq!(window.model().category.get(), 1, "Clicking a category did not select it.");
		assert!(!window.tick(), "Switching categories did not settle within two ticks.");

		let mut category = 1;
		bencher.counter(ItemsCount::new(2usize)).bench_local(|| {
			category = (category + 1) % CATEGORIES.len();
			window.click(buttons[category]);
			window.ticks(2);
		});
	}

	/// Measures dragging the window edge: the viewport changes size every tick.
	#[divan::bench]
	fn resize_window(bencher: Bencher) {
		let mut window = window();
		let sizes = [Size::new(WIDTH, HEIGHT), Size::new(1600, 900)];
		let mut index = 0;
		bencher.bench_local(|| {
			index = (index + 1) % sizes.len();
			window.size = sizes[index];
			window.tick()
		});
	}

	/// Measures a whole confirmation: open, fade in, click Cancel, fade out, unmount.
	#[divan::bench]
	fn confirm_dialog_round_trip(bencher: Bencher) {
		let mut window = window();
		let reset = window
			.model()
			.reset
			.get()
			.expect("The reset button did not report its identity.");
		let round_trip = |window: &mut Window<Model>| {
			window.click(reset);
			window.ticks(ENTER_FRAMES + 1);
			let cancel = window
				.model()
				.cancel
				.get()
				.expect("The dialog did not report its cancel button.");
			window.click(cancel);
			window.ticks(EXIT_FRAMES + 2);
		};
		round_trip(&mut window);
		assert!(!window.model().dialog_open.get(), "The confirmation dialog did not close.");
		assert!(!window.tick(), "The confirmation dialog did not settle after closing.");

		bencher
			.counter(ItemsCount::new(ENTER_FRAMES + EXIT_FRAMES + 3))
			.bench_local(|| round_trip(&mut window));
	}
}

/// A gameplay HUD over the scene: player vitals, minimap, ability bar, match clock, kill feed, and FPS counter.
mod hud {
	use std::f32::consts::TAU;

	use super::*;

	/// Frames before the simulated match repeats, so every sample sees the same mix of changes.
	const PERIOD: u32 = 360;
	const SLOTS: usize = 8;
	const COOLDOWN_FRAMES: u32 = 150;
	const SLOT_SIZE: f32 = 64.0;
	const MARKERS: usize = 24;
	const MOVING_MARKERS: usize = 8;
	const FEED_LINES: usize = 5;
	const FEED: [&str; 8] = [
		"Vex eliminated Marrow",
		"Kestrel captured Relay B",
		"Ash eliminated Tundra",
		"Relay C is contested",
		"Marrow revived Ash",
		"Vex eliminated Kestrel",
		"Double kill by Tundra",
		"Relay B lost",
	];

	/// The `Model` struct carries the simulation frame the HUD presents.
	#[derive(Default)]
	struct Model {
		frame: Cell<u32>,
	}

	/// The `Shown` struct remembers presented values so unchanged widgets skip updates.
	struct Shown {
		health: u32,
		mana: u32,
		cooldowns: [u32; SLOTS],
		seconds: u32,
		feed: u32,
		fps: u32,
	}

	fn bar<C: 'static>(parent: &mut EvaluationContext<C>, color: ConcreteLayer) -> EvaluationContext<C> {
		let mut track = parent.element("bar").container(
			Container::default()
				.width(240.0.into())
				.height(14.0.into())
				.corner_radius(7.0)
				.flow(flow::row)
				.hit_testable(false)
				.style(fill(0.0, 0.0, 0.0, 0.5)),
		);
		track.element("fill").container(
			Container::default()
				.width(240.0.into())
				.height(14.0.into())
				.corner_radius(7.0)
				.hit_testable(false)
				.style(color),
		)
	}

	/// Places a HUD group at a window position.
	fn anchored<C: 'static>(
		parent: &mut EvaluationContext<C>,
		name: &'static str,
		(x, y): (f32, f32),
		(width, height): (f32, f32),
		flow: impl flow::FlowFunction + 'static,
		style: impl Into<ConcreteStyle>,
	) -> EvaluationContext<C> {
		parent.element(name).container(
			Container::default()
				.absolute_position(x, y)
				.width(width.into())
				.height(height.into())
				.corner_radius(12.0)
				.flow(flow)
				.hit_testable(false)
				.style(style),
		)
	}

	// Keep the HUD as one component, as games write it: build every widget, then present game state each frame.
	#[allow(clippy::too_many_lines)]
	async fn hud(ctx: &mut EvaluationContext<Model>) {
		let mut root = ctx
			.element("root")
			.container(Container::default().hit_testable(false).style(ConcreteStyle::new()));

		let mut player = anchored(
			&mut root,
			"player",
			(24.0, 24.0),
			(372.0, 120.0),
			flow::row_with_gap(16),
			panel(0.04, 0.05, 0.07),
		);
		player.element("portrait").container(
			Container::default()
				.width(96.0.into())
				.height(96.0.into())
				.corner_radius(10.0)
				.hit_testable(false)
				.style(panel(0.3, 0.22, 0.18)),
		);
		let mut vitals = player.element("vitals").container(
			Container::default()
				.width(240.0.into())
				.height(112.0.into())
				.flow(flow::column_with_gap(4))
				.hit_testable(false)
				.style(ConcreteStyle::new()),
		);
		cell(
			&mut vitals,
			240.0,
			28.0,
			Text::new("Kestrel").font_size(20.0).style(text_color()),
		);
		let mut health_fill = bar(&mut vitals, fill(0.85, 0.25, 0.25, 1.0));
		let mut health_text = cell(&mut vitals, 240.0, 20.0, Text::new("").font_size(14.0).style(text_color()));
		let mut mana_fill = bar(&mut vitals, fill(0.25, 0.5, 0.95, 1.0));
		let mut mana_text = cell(&mut vitals, 240.0, 20.0, Text::new("").font_size(14.0).style(text_color()));

		let mut clock_panel = anchored(
			&mut root,
			"clock",
			(WIDTH * 0.5 - 200.0, 24.0),
			(400.0, 72.0),
			flow::centered_column,
			panel(0.04, 0.05, 0.07),
		);
		let mut clock = cell(
			&mut clock_panel,
			400.0,
			40.0,
			Text::new("").font_size(28.0).style(text_color()),
		);
		cell(
			&mut clock_panel,
			400.0,
			24.0,
			Text::new("Capture the relay (2/3)").font_size(15.0).style(muted_color()),
		);

		let mut minimap = anchored(
			&mut root,
			"minimap",
			(WIDTH - 264.0, 24.0),
			(240.0, 240.0),
			flow::center,
			panel(0.06, 0.09, 0.08),
		);
		let mut markers = Vec::with_capacity(MARKERS);
		for index in 0..MARKERS {
			let enemy = index < MOVING_MARKERS;
			let angle = index as f32 / MARKERS as f32 * TAU;
			let radius = 30.0 + (index % 4) as f32 * 20.0;
			markers.push(
				minimap.element("marker").container(
					Container::default()
						.width(10.0.into())
						.height(10.0.into())
						.corner_radius(5.0)
						.hit_testable(false)
						.transform(Transform::identity().translate(angle.cos() * radius, angle.sin() * radius))
						.style(if enemy {
							fill(0.95, 0.3, 0.3, 1.0)
						} else {
							fill(0.35, 0.85, 0.5, 1.0)
						}),
				),
			);
		}

		let bar_width = SLOTS as f32 * SLOT_SIZE + (SLOTS - 1) as f32 * 8.0;
		let mut abilities = anchored(
			&mut root,
			"abilities",
			((WIDTH - bar_width) * 0.5, HEIGHT - SLOT_SIZE - 32.0),
			(bar_width, SLOT_SIZE),
			flow::row_with_gap(8),
			ConcreteStyle::new(),
		);
		let mut slots = Vec::with_capacity(SLOTS);
		for (index, key) in ["1", "2", "3", "4", "Q", "E", "R", "F"].into_iter().enumerate() {
			let mut slot = abilities.element("slot").container(
				Container::default()
					.width(SLOT_SIZE.into())
					.height(SLOT_SIZE.into())
					.corner_radius(10.0)
					.flow(flow::center)
					.hit_testable(false)
					.style(panel(0.12 + index as f32 * 0.04, 0.14, 0.2)),
			);
			slot.element("key").text(
				Text::new(key)
					.font_size(12.0)
					.style(muted_color())
					.transform(Transform::identity().translate(-22.0, -22.0)),
			);
			let overlay = slot.element("cooldown").container(
				Container::default()
					.width(SLOT_SIZE.into())
					.height(0.0.into())
					.hit_testable(false)
					.style(fill(0.0, 0.0, 0.0, 0.6)),
			);
			let seconds = slot
				.element("seconds")
				.text(Text::new("").font_size(20.0).style(text_color()));
			slots.push((overlay, seconds));
		}

		let mut feed_panel = anchored(
			&mut root,
			"feed",
			(24.0, HEIGHT - 24.0 - FEED_LINES as f32 * 28.0),
			(360.0, FEED_LINES as f32 * 28.0),
			flow::column,
			ConcreteStyle::new(),
		);
		let mut feed: Vec<_> = (0..FEED_LINES)
			.map(|_| {
				cell(
					&mut feed_panel,
					360.0,
					28.0,
					Text::new("").font_size(16.0).style(text_color()),
				)
			})
			.collect();

		let mut fps_panel = anchored(
			&mut root,
			"fps",
			(WIDTH - 120.0, HEIGHT - 40.0),
			(96.0, 24.0),
			flow::centered_row,
			ConcreteStyle::new(),
		);
		let mut fps = fps_panel
			.element("value")
			.text(Text::new("").font_size(14.0).style(muted_color()));

		// Game state drives the HUD; only widgets whose presented value changed are touched.
		let mut shown: Option<Shown> = None;
		loop {
			let frame = ctx.ctx().frame.get() % PERIOD;
			let phase = frame as f32 / PERIOD as f32 * TAU;
			let state = Shown {
				health: (70.0 + 25.0 * phase.sin()).round() as u32,
				mana: (50.0 + 40.0 * (phase * 2.0).cos()).round() as u32,
				cooldowns: std::array::from_fn(|slot| {
					// Two abilities recharge at a time; the rest are ready.
					if slot == 1 || slot == 5 {
						(COOLDOWN_FRAMES - (frame + slot as u32 * 37) % COOLDOWN_FRAMES) / 6
					} else {
						0
					}
				}),
				seconds: frame / 60,
				feed: frame / 90,
				fps: frame / 15,
			};
			let previous = shown.take().unwrap_or(Shown {
				health: u32::MAX,
				mana: u32::MAX,
				cooldowns: [u32::MAX; SLOTS],
				seconds: u32::MAX,
				feed: u32::MAX,
				fps: u32::MAX,
			});

			if state.health != previous.health {
				health_fill.update_container(|bar| bar.width = (2.4 * state.health as f32).into());
				health_text.update_text(|text| text.set_content(format!("{} / 100", state.health)));
			}
			if state.mana != previous.mana {
				mana_fill.update_container(|bar| bar.width = (2.4 * state.mana as f32).into());
				mana_text.update_text(|text| text.set_content(format!("{} / 100", state.mana)));
			}
			// Enemy markers track their units every frame; friendly markers stay put.
			for (index, marker) in markers.iter_mut().take(MOVING_MARKERS).enumerate() {
				let angle = index as f32 / MOVING_MARKERS as f32 * TAU + phase;
				let radius = 40.0 + 50.0 * (phase * 3.0 + index as f32).sin().abs();
				marker.update_container(|marker| {
					marker.set_transform(Transform::identity().translate(angle.cos() * radius, angle.sin() * radius))
				});
			}
			for (slot, (overlay, seconds)) in slots.iter_mut().enumerate() {
				let remaining = state.cooldowns[slot];
				if remaining == previous.cooldowns[slot] {
					continue;
				}
				overlay.update_container(|overlay| {
					overlay.height = (SLOT_SIZE * remaining as f32 * 6.0 / COOLDOWN_FRAMES as f32).into()
				});
				seconds.update_text(|text| {
					if remaining == 0 {
						text.set_content("");
					} else {
						text.set_content(format!("{:.1}", remaining as f32 * 0.1));
					}
				});
			}
			if state.seconds != previous.seconds {
				clock.update_text(|text| text.set_content(format!("12:{:02}", 59 - state.seconds)));
			}
			if state.feed != previous.feed {
				for (line, text) in feed.iter_mut().enumerate() {
					let entry = FEED[(state.feed as usize + line) % FEED.len()];
					text.update_text(|text| text.set_content(entry));
				}
			}
			if state.fps != previous.fps {
				const RATES: [u32; 6] = [144, 143, 141, 144, 139, 142];
				fps.update_text(|text| text.set_content(format!("{} FPS", RATES[state.fps as usize % RATES.len()])));
			}
			shown = Some(state);
			ctx.render().await;
		}
	}

	/// Measures a HUD frame during play: moving minimap markers, draining bars, recharging abilities, and ticking text.
	#[divan::bench]
	fn gameplay(bencher: Bencher) {
		let mut window = Window::new(Model::default(), |ctx| Box::pin(hud(ctx)));
		for _ in 0..PERIOD {
			window.model().frame.set(window.model().frame.get() + 1);
			assert!(window.tick(), "A gameplay frame left the HUD unchanged.");
		}
		bencher.bench_local(|| {
			window.model().frame.set(window.model().frame.get() + 1);
			window.tick()
		});
	}
}

/// A leaderboard table in the style of js-framework-benchmark: many rows of text cells, one selection.
mod leaderboard {
	use super::*;

	const ROW_HEIGHT: f32 = 32.0;
	const TABLE_WIDTH: f32 = 1200.0;
	const BODY_HEIGHT: f32 = 900.0;
	const HANDLES: [&str; 12] = [
		"Kestrel", "Vex", "Marrow", "Ash", "Tundra", "Quill", "Ember", "Nomad", "Wisp", "Rook", "Sable", "Juniper",
	];
	const STATUS: [&str; 3] = ["In match", "Online", "Away"];

	/// The `Model` struct holds the table's size, visibility, and live score round.
	struct Model {
		rows: usize,
		visible: Cell<bool>,
		round: Cell<u32>,
		body: Cell<Option<Id>>,
	}

	fn window(rows: usize) -> Window<Model> {
		let model = Model {
			rows,
			visible: Cell::new(true),
			round: Cell::new(0),
			body: Cell::new(None),
		};
		Window::new(model, |ctx| Box::pin(host(ctx)))
	}

	/// Scores stay within a bounded set so live updates reach a steady state.
	fn score(index: usize, round: u32) -> u32 {
		40_000 - index as u32 * 13 + (round % 16) * 25
	}

	fn row_style(index: usize, selected: bool) -> ConcreteLayer {
		if selected {
			fill(0.2, 0.32, 0.55, 1.0)
		} else {
			fill(1.0, 1.0, 1.0, if index.is_multiple_of(2) { 0.03 } else { 0.0 })
		}
	}

	/// Opens the leaderboard while it is visible, like a menu navigating to and from it.
	async fn host(ctx: &mut EvaluationContext<Model>) {
		let mut root = ctx.element("root").container(
			Container::default()
				.flow(flow::center)
				.hit_testable(false)
				.style(fill(0.05, 0.06, 0.08, 1.0)),
		);
		loop {
			if ctx.ctx().visible.get() {
				root.element("board").mount(|ctx| Box::pin(board(ctx))).await;
			} else {
				ctx.render().await;
			}
		}
	}

	async fn board(ctx: &mut EvaluationContext<Model>) {
		let rows = ctx.ctx().rows;
		let mut table = ctx.element("table").container(
			Container::default()
				.width(TABLE_WIDTH.into())
				.height((BODY_HEIGHT + 48.0).into())
				.corner_radius(12.0)
				.flow(flow::column)
				.hit_testable(false)
				.style(panel(0.07, 0.08, 0.1)),
		);
		let mut header = table.element("header").container(
			Container::default()
				.height(48.0.into())
				.flow(flow::row)
				.hit_testable(false)
				.style(fill(1.0, 1.0, 1.0, 0.06)),
		);
		const COLUMNS: [(&str, f32); 4] = [("#", 80.0), ("Player", 560.0), ("Score", 280.0), ("Status", 280.0)];
		for (title, width) in COLUMNS {
			cell(
				&mut header,
				width,
				48.0,
				Text::new(title).font_size(15.0).style(muted_color()),
			);
		}
		// Rows share the body as their pointer target; the pointer's height picks the row.
		let mut body = table.element("body").container(
			Container::default()
				.height(BODY_HEIGHT.into())
				.flow(flow::column)
				.style(ConcreteStyle::new()),
		);
		ctx.ctx().body.set(Some(body.id()));

		let mut lines = Vec::with_capacity(rows);
		let round = ctx.ctx().round.get();
		for index in 0..rows {
			let mut row = body.element("row").container(
				Container::default()
					.height(ROW_HEIGHT.into())
					.flow(flow::row)
					.hit_testable(false)
					.style(row_style(index, false)),
			);
			let text = |content: String| Text::new(content).font_size(16.0).style(text_color());
			cell(&mut row, COLUMNS[0].1, ROW_HEIGHT, text((index + 1).to_string()));
			cell(
				&mut row,
				COLUMNS[1].1,
				ROW_HEIGHT,
				text(format!("{}{}", HANDLES[index % HANDLES.len()], index)),
			);
			let score_cell = cell(&mut row, COLUMNS[2].1, ROW_HEIGHT, text(score(index, round).to_string()));
			cell(&mut row, COLUMNS[3].1, ROW_HEIGHT, text(STATUS[index % STATUS.len()].into()));
			lines.push((row, score_cell));
		}

		let mut selected: Option<usize> = None;
		let mut shown_round = round;
		loop {
			select_biased! {
				_ = body.on(Events::Actuated) => {
					let Some(geometry) = body.geometry() else { continue };
					let pointer_y = (1.0 - ctx.pointer().position.y) * 0.5 * HEIGHT;
					let index = ((pointer_y - geometry.y()) / ROW_HEIGHT) as usize;
					if index < rows && selected != Some(index) {
						if let Some(previous) = selected {
							lines[previous].0.update_container(|row| row.set_style(row_style(previous, false)));
						}
						lines[index].0.update_container(|row| row.set_style(row_style(index, true)));
						selected = Some(index);
					}
				},
				_ = ctx.render() => {
					if !ctx.ctx().visible.get() {
						return;
					}
					let round = ctx.ctx().round.get();
					if round != shown_round {
						shown_round = round;
						for (index, (_, score_cell)) in lines.iter_mut().enumerate().step_by(10) {
							score_cell.update_text(|text| text.set_content(score(index, round).to_string()));
						}
					}
				},
			}
		}
	}

	/// Measures an untouched table; retained layout still visits every row.
	#[divan::bench(args = [100, 1000])]
	fn idle(bencher: Bencher, rows: usize) {
		let mut window = window(rows);
		window.tick();
		assert!(!window.tick(), "An untouched leaderboard republished its render.");
		bencher.bench_local(|| window.tick());
	}

	/// Measures a live score push that rewrites every tenth row's score.
	#[divan::bench(args = [100, 1000])]
	fn update_every_10th_row(bencher: Bencher, rows: usize) {
		let mut window = window(rows);
		window.model().round.set(1);
		assert!(window.tick(), "A score round left the leaderboard unchanged.");
		bencher.bench_local(|| {
			window.model().round.set(window.model().round.get() + 1);
			window.tick()
		});
	}

	/// Measures clicking a row: the old selection clears and the new row highlights.
	#[divan::bench(args = [100, 1000])]
	fn select_row(bencher: Bencher, rows: usize) {
		let mut window = window(rows);
		let body = window.model().body.get().expect("The leaderboard did not report its body.");
		let top = window.hits.bounds(body).expect("The leaderboard body is not on screen.");
		let row = |index: usize| UiPoint::new(top.x() + 200.0, top.y() + (index as f32 + 0.5) * ROW_HEIGHT);
		window.click_at(row(3));
		assert!(window.tick(), "Clicking a leaderboard row did not select it.");

		let mut index = 0;
		bencher.bench_local(|| {
			index += 1;
			window.click_at(row(3 + 4 * (index % 2)));
			window.tick()
		});
	}

	/// Measures navigating to the leaderboard and back: every row mounts, lays out, and unmounts.
	#[divan::bench(args = [100, 1000])]
	fn open_close(bencher: Bencher, rows: usize) {
		let mut window = window(rows);
		for visible in [false, true] {
			window.model().visible.set(visible);
			assert!(window.tick(), "Toggling the leaderboard did not change the screen.");
		}
		bencher.counter(ItemsCount::new(2usize)).bench_local(|| {
			for visible in [false, true] {
				window.model().visible.set(visible);
				window.tick();
			}
		});
	}
}

/// A kanban board whose cards are dragged between columns, as in `sandbox/ui`.
mod board {
	use std::cell::RefCell;

	use super::*;

	const COLUMNS: [&str; 3] = ["To do", "Doing", "Done"];
	const TASKS: [&str; 24] = [
		"Write the design doc",
		"Profile the layout pass",
		"Wire gamepad focus",
		"Localize the main menu",
		"Fix save slot overflow",
		"Tune enemy spawn waves",
		"Add photo mode",
		"Record foley pass",
		"Bake lighting for dock",
		"Rig the courier",
		"Cap frame pacing",
		"Review crash reports",
		"Controller remapping",
		"Subtitle sizing",
		"Shader warmup cache",
		"Streamer mode toggle",
		"Achievement icons",
		"Balance the shotgun",
		"Cloud save conflicts",
		"Patch notes draft",
		"Colorblind palettes",
		"Load screen tips",
		"Crossplay invites",
		"Certification checklist",
	];
	const BOARD_TOP: f32 = 160.0;
	const COLUMN_WIDTH: f32 = 280.0;
	const COLUMN_GAP: f32 = 24.0;
	const CARD_INSET: f32 = 12.0;
	const CARD_HEIGHT: f32 = 64.0;
	const CARD_GAP: f32 = 8.0;
	/// Pointer moves per gesture, a quarter second of motion at 60 Hz.
	const DRAG_TICKS: usize = 15;

	/// The `Model` struct exposes the board's identities and counts accepted drops.
	#[derive(Default)]
	struct Model {
		cards: RefCell<Vec<Id>>,
		columns: Cell<[Option<Id>; COLUMNS.len()]>,
		landed: Cell<u32>,
	}

	fn placed_at(x: f32, y: f32) -> Transform {
		Transform::identity().origin(UiPoint::zero()).translate(x, y)
	}

	/// Cards are flow children of their column; a grabbed card is lifted to the root and follows the pointer.
	async fn board(ctx: &mut EvaluationContext<Model>) {
		let mut root = ctx
			.element("root")
			.container(Container::default().hit_testable(false).style(fill(0.05, 0.06, 0.08, 1.0)));
		let root_id = root.id();
		let mut columns = std::array::from_fn::<_, { COLUMNS.len() }, _>(|index| {
			let mut column = root.element(COLUMNS[index]).container(
				Container::default()
					.absolute_position(160.0 + index as f32 * (COLUMN_WIDTH + COLUMN_GAP), BOARD_TOP)
					.width(COLUMN_WIDTH.into())
					.height(760.0.into())
					.corner_radius(10.0)
					.flow(flow::column_with_gap(CARD_GAP))
					.style(panel(0.08, 0.09, 0.1)),
			);
			cell(
				&mut column,
				COLUMN_WIDTH,
				44.0,
				Text::new(COLUMNS[index]).font_size(16.0).style(text_color()),
			);
			column
		});
		let column_ids = columns.each_ref().map(|column| column.id());
		ctx.ctx().columns.set(column_ids.map(Some));

		// Each card with the column it belongs to.
		let mut cards = Vec::with_capacity(TASKS.len());
		for (index, title) in TASKS.into_iter().enumerate() {
			let column = index % COLUMNS.len();
			let mut card = columns[column].element(title).container(
				Container::default()
					.width((COLUMN_WIDTH - CARD_INSET * 2.0).into())
					.height(CARD_HEIGHT.into())
					.corner_radius(8.0)
					.transform(placed_at(CARD_INSET, 0.0))
					.style(panel(0.16, 0.18, 0.22)),
			);
			card.text(
				Text::new(title)
					.font_size(15.0)
					.style(text_color())
					.transform(Transform::identity().translate(12.0, 22.0)),
			);
			ctx.ctx().cards.borrow_mut().push(card.id());
			cards.push((card, column));
		}

		// The lifted card and where it was when grabbed.
		let mut held: Option<(usize, UiPoint)> = None;
		loop {
			let [todo, doing, done] = &mut columns;
			// An idle board waits on the first card, which cannot end a gesture it is not in.
			let (lifted, _) = &mut cards[held.map_or(0, |(task, _)| task)];
			let mut ended = false;
			// Biased so a queued drop is taken before the end of the gesture that produced it.
			let landed = select_biased! {
				event = todo.on(Events::Dropped) => Some((0, event.source)),
				event = doing.on(Events::Dropped) => Some((1, event.source)),
				event = done.on(Events::Dropped) => Some((2, event.source)),
				_ = lifted.on(Events::DragEnded) => { ended = true; None },
				_ = ctx.render() => None,
			};
			if let Some((column, Some(source))) = landed
				&& let Some((_, home)) = cards.iter_mut().find(|(card, _)| card.id() == source)
			{
				*home = column;
				ctx.ctx().landed.set(ctx.ctx().landed.get() + 1);
			}

			let drag = ctx.drag();
			if held.is_none()
				&& let Some(capture) = drag.filter(|capture| capture.dragging)
				&& let Some(task) = cards.iter().position(|(card, _)| card.id() == capture.source)
			{
				let (card, _) = &mut cards[task];
				// Lift the card out of its column; an absolute depth keeps it from displacing the root's children.
				let origin = card
					.geometry()
					.map_or(UiPoint::zero(), |geometry| UiPoint::new(geometry.x(), geometry.y()));
				card.reparent(root_id);
				card.update_container(|card| card.depth = Depth::absolute(1));
				held = Some((task, origin));
			}
			if let Some((task, origin)) = held {
				let (card, home) = &mut cards[task];
				if ended {
					card.reparent(column_ids[*home]);
					card.update_container(|card| {
						card.depth = Depth::relative(1);
						card.set_transform(placed_at(CARD_INSET, 0.0));
					});
					held = None;
				} else if let Some(capture) = drag {
					card.update_container(|card| {
						card.set_transform(placed_at(
							origin.x + capture.position.x - capture.origin.x,
							origin.y + capture.position.y - capture.origin.y,
						));
					});
				}
			}
		}
	}

	/// Moves one card to the next column the way a user does: hit test, press, drag, release, settle.
	fn drag_card(window: &mut Window<Model>, card: Id, target: usize) {
		let column = window.model().columns.get()[target].expect("A board column did not report its identity.");
		let from = window.center(card);
		let to = window.center(column);
		assert!(
			window.engine.press(window.normalized(from)),
			"The dragged card is not the frontmost surface under the pointer."
		);
		for step in 1..=DRAG_TICKS {
			let t = step as f32 / DRAG_TICKS as f32;
			let position = UiPoint::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
			window.point_at(position);
			window.engine.drag_to(window.normalized(position));
			window.tick();
		}
		window.engine.release(window.normalized(to));
		window.ticks(2);
	}

	/// Measures a whole card move between columns, including the two reparents.
	#[divan::bench]
	fn drag_card_between_columns(bencher: Bencher) {
		let mut window = Window::new(Model::default(), |ctx| Box::pin(board(ctx)));
		let card = window.model().cards.borrow()[0];
		drag_card(&mut window, card, 1);
		assert_eq!(window.model().landed.get(), 1, "Dragging a card did not drop it on a column.");
		assert!(!window.tick(), "The board did not settle after a drop.");

		let mut column = 1;
		bencher.counter(ItemsCount::new(DRAG_TICKS + 2)).bench_local(|| {
			column = (column + 1) % COLUMNS.len();
			drag_card(&mut window, card, column);
		});
	}
}

/// A sign-in form with a focused email field that the player types into.
mod sign_in {
	use super::*;

	const EMAIL: &str = "kestrel.player@example.com";

	/// The `Model` struct reports how much of the address the form holds.
	#[derive(Default)]
	struct Model {
		typed: Cell<usize>,
	}

	fn field<C: 'static>(parent: &mut EvaluationContext<C>, name: &'static str, content: &str) -> EvaluationContext<C> {
		let mut frame = parent.element(name).container(
			Container::default()
				.width(440.0.into())
				.height(44.0.into())
				.corner_radius(8.0)
				.flow(flow::centered_row)
				.style(panel(0.05, 0.06, 0.08)),
		);
		frame.element("field").text_field(
			TextField::new(content)
				.font_size(18.0)
				.style(text_color())
				.transform(Transform::identity().translate_x(12.0)),
		)
	}

	async fn form(ctx: &mut EvaluationContext<Model>) {
		let mut root = ctx.element("root").container(
			Container::default()
				.flow(flow::center)
				.hit_testable(false)
				.style(fill(0.05, 0.06, 0.08, 1.0)),
		);
		let mut form = root.element("form").container(
			Container::default()
				.width(480.0.into())
				.height(440.0.into())
				.corner_radius(16.0)
				.flow(flow::centered_column)
				.hit_testable(false)
				.style(panel(0.09, 0.1, 0.12)),
		);
		cell(
			&mut form,
			440.0,
			64.0,
			Text::new("Sign in").font_size(32.0).style(text_color()),
		);
		cell(
			&mut form,
			440.0,
			32.0,
			Text::new("Use the account you play with on every platform.")
				.font_size(14.0)
				.style(muted_color()),
		);
		cell(
			&mut form,
			440.0,
			28.0,
			Text::new("Email").font_size(14.0).style(muted_color()),
		);
		let mut email = field(&mut form, "email", "");
		cell(
			&mut form,
			440.0,
			28.0,
			Text::new("Password").font_size(14.0).style(muted_color()),
		);
		field(&mut form, "password", "••••••••••");
		let mut remember = form.element("remember").container(
			Container::default()
				.width(440.0.into())
				.height(48.0.into())
				.flow(flow::centered_row)
				.hit_testable(false)
				.style(ConcreteStyle::new()),
		);
		remember.element("check").container(
			Container::default()
				.width(20.0.into())
				.height(20.0.into())
				.corner_radius(4.0)
				.style(panel(0.25, 0.6, 0.95)),
		);
		cell(
			&mut remember,
			400.0,
			48.0,
			Text::new("Keep me signed in")
				.font_size(15.0)
				.style(text_color())
				.transform(Transform::identity().translate_x(12.0)),
		);
		let mut submit = form.element("submit").container(
			Container::default()
				.width(440.0.into())
				.height(48.0.into())
				.corner_radius(10.0)
				.flow(flow::center)
				.style(panel(0.25, 0.5, 0.9)),
		);
		submit.text(Text::new("Sign In").font_size(18.0).style(text_color()));

		// The application owns the text; the field reports edits and displays the result.
		email.request_focus();
		let mut content = String::new();
		loop {
			let edit = email.on_text_edit().await;
			edit.edit.apply_to(&mut content);
			email.update_text_field(|field| field.set_content(&content));
			ctx.ctx().typed.set(content.len());
		}
	}

	/// Types one keystroke per tick: the address character by character, then backspace to empty.
	fn keystroke(window: &mut Window<Model>, step: usize) {
		let position = step % (2 * EMAIL.len());
		if position < EMAIL.len() {
			window.engine.input_character(EMAIL.as_bytes()[position] as char);
			window.tick();
		} else {
			window.engine.update_key_state(Key::Backspace, true);
			window.engine.delete_text_backward();
			window.tick();
			window.engine.update_key_state(Key::Backspace, false);
		}
	}

	/// Measures a tick with one keystroke in a focused text field.
	#[divan::bench]
	fn type_email(bencher: Bencher) {
		let mut window = Window::new(Model::default(), |ctx| Box::pin(form(ctx)));
		for step in 0..2 * EMAIL.len() {
			keystroke(&mut window, step);
			let expected = if step < EMAIL.len() {
				step + 1
			} else {
				2 * EMAIL.len() - step - 1
			};
			assert_eq!(
				window.model().typed.get(),
				expected,
				"A keystroke did not reach the email field."
			);
		}

		let mut step = 0;
		bencher.bench_local(|| {
			keystroke(&mut window, step);
			step += 1;
		});
	}
}

/// A tooltip sliding over a grid of many static, hit-testable cards, as a hover hint does
/// in `sandbox/ui`: one small subtree moves by a visual transform while everything else
/// holds, so the cost is what the engine spends outside the moved subtree.
mod tooltip {
	use super::*;

	const CARDS: usize = 600;
	const COLUMNS: usize = 30;
	/// Frames before the tooltip's path repeats, so every sample sees the same motion.
	const PERIOD: u32 = 200;

	/// The `Model` struct carries the frame that places the tooltip.
	#[derive(Default)]
	struct Model {
		frame: Cell<u32>,
	}

	async fn screen(ctx: &mut EvaluationContext<Model>) {
		let mut root = ctx
			.element("root")
			.container(Container::default().size(Sizing::Relative(1, 1)).hit_testable(false));
		for index in 0..CARDS {
			let mut card = root.element("card").container(
				Container::default()
					.absolute_position((index % COLUMNS * 60) as u32, (index / COLUMNS * 48) as u32)
					.width(56.into())
					.height(44.into())
					.clip(true)
					.corner_radius(4.0)
					.style(panel(0.16, 0.18, 0.22)),
			);
			if index % 3 == 0 {
				card.element("label").text(Text::new("Card").font_size(12.0));
			}
		}
		let mut tooltip = root.element("tooltip").container(
			Container::default()
				.absolute_position(0u32, 0u32)
				.width(160.into())
				.height(56.into())
				.clip(true)
				.corner_radius(6.0)
				.style(panel(0.16, 0.18, 0.22)),
		);
		tooltip.element("label").text(Text::new("Tooltip").font_size(14.0));
		tooltip
			.element("hint")
			.container(Container::default().size(20.into()).style(panel(0.16, 0.18, 0.22)));
		loop {
			let frame = (ctx.ctx().frame.get() % PERIOD) as f32;
			tooltip.update_container(|value| {
				value.set_transform(Transform::identity().translate(frame * 7.0, frame * 3.5));
			});
			ctx.render().await;
		}
	}

	#[divan::bench]
	fn move_over_cards(bencher: Bencher) {
		let mut window = Window::new(Model::default(), |ctx| Box::pin(screen(ctx)));
		window.model().frame.set(window.model().frame.get() + 1);
		assert!(window.tick(), "Moving the tooltip left the screen unchanged.");
		bencher.bench_local(|| {
			window.model().frame.set(window.model().frame.get() + 1);
			window.tick()
		});
	}
}
