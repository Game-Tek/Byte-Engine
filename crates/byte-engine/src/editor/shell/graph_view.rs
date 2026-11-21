//! The graph canvas: while a material graph is open, it fills the graph's window and draws its nodes and wires.
//!
//! Dragging the canvas pans it, and scrolling zooms it about the pointer. A node's values are fields: dragging across
//! a number changes it, and clicking one takes typed text, where empty text returns the value to its default.
//! Dragging out of a node's output draws a wire that connects to the input it's dropped on; dragging a wire off an
//! input moves it, or disconnects it when dropped on empty canvas. A node's × removes it, and the palette adds nodes.
//! A bar floating over the canvas's top-left corner names the graph and says whether it has unsaved changes; its Save
//! button, or Cmd+S, asks the editor to write the graph. Closing the window, or Escape, closes the graph. The canvas is
//! declared each time a graph opens and when the window resizes, and removed when it closes; an edit that changes the
//! nodes, their rows, or the wires declares the nodes again. Zooming scales the layer that holds them.

use std::cell::Cell;
use std::rc::Rc;

use serde_json::Value as Json;

use super::VALUE_FONT;
use super::resource_panels::base;
use super::{
	ACCENT, AXIS_COLORS, CORNER, MARGIN, MUTED, SQUIRCLE, Shell, TEXT, hairline, hex, panel, read_typed, row_style, solid,
	top_highlight, typing_style, vertical,
};
use crate::editor::graph::{Graph, HEADER_HEIGHT, NODE_WIDTH, PendingWire, ROW_HEIGHT, Row, Source};
use crate::ui::layout::context::{ContainerContext, Context, ElementContext, ElementKey};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::primitive::Events;
use crate::ui::{Anchor, ConcreteLayer, Position, Sizing, Transform, UiPoint, flow};
use crate::utils::RGBA;
use crate::utils::r#async::select;

/// The height of the bar that names the graph, over the canvas's top-left corner.
const BAR_HEIGHT: f32 = 36.0;
/// The smallest and largest zoom.
const ZOOM_RANGE: (f32, f32) = (0.3, 2.0);
/// How much one scroll step zooms in or out.
const ZOOM_STEP: f32 = 1.15;
/// The width of the bar's Save button.
const SAVE_WIDTH: f32 = 48.0;
/// A plug's diameter, in graph units. A socket is the half of it cut into the node's edge, and a wire's end holds the
/// other half, outside the edge, so a connected socket and its wire close into one round plug.
const SOCKET: f32 = 14.0;
/// The width of the ring around a connected plug. The halves under it are drawn that much smaller, so their edges sit
/// under the ring's solid part instead of showing a fringe of their color past its anti-aliased edge, as a border's
/// background is clipped to its padding box.
const RING: f32 = 1.0;
/// How much dragging one pixel across a number changes it.
const SCRUB_STEP: f64 = 0.01;
/// The color of a value the file leaves out, which its kind gives.
const DEFAULT_VALUE: RGBA = hex(0x8A8F98, 0.6);

/// Shows the open graph's canvas while a graph is open, declaring it again for each graph that opens.
pub(super) async fn graph_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		// Waiting on the layer reads it only when the shell changed, so a closed view asks for no frames.
		let mut open = ctx.proactive(|shell: &Shell| shell.layers.graph().is_some());
		while !*open.next().await {}
		let mut root = graph_canvas(ctx).await;
		root.remove().await;
	}
}

/// The `View` struct is where the canvas looks: how far the graph is panned, in pixels, and how much it's zoomed.
#[derive(Clone, Copy, Debug)]
struct View {
	pan: [f32; 2],
	zoom: f32,
}

impl View {
	/// The view that fits `bounds`, the graph's left, top, right, and bottom, into a canvas `size` pixels large,
	/// centered, and never zoomed in past 1.
	fn fitting(bounds: [f32; 4], size: [f32; 2]) -> Self {
		let [left, top, right, bottom] = bounds;
		let (width, height) = ((right - left).max(1.0), (bottom - top).max(1.0));
		let zoom = ((size[0] / width).min(size[1] / height) * 0.9).clamp(ZOOM_RANGE.0, 1.0);
		View {
			pan: [
				(size[0] - width * zoom) * 0.5 - left * zoom,
				(size[1] - height * zoom) * 0.5 - top * zoom,
			],
			zoom,
		}
	}

	/// The transform that places the graph's layer: panned by `pan` and scaled by `zoom` about its top-left corner, so
	/// the graph is laid out in its own units and the zoom costs no new declarations.
	fn transform(self) -> Transform {
		Transform::identity()
			.origin(UiPoint::zero())
			.translate(self.pan[0], self.pan[1])
			.scale(self.zoom)
	}

	/// The view after zooming by `factor` about `anchor`, a point of the canvas in pixels, which stays over the same
	/// point of the graph.
	fn zoomed(self, factor: f32, anchor: [f32; 2]) -> Self {
		let zoom = (self.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
		let scale = zoom / self.zoom;
		View {
			pan: [
				anchor[0] - (anchor[0] - self.pan[0]) * scale,
				anchor[1] - (anchor[1] - self.pan[1]) * scale,
			],
			zoom,
		}
	}
}

/// Declares the open graph's canvas and its bar and keeps them current until the graph closes, then returns the layer
/// holding both for removal.
async fn graph_canvas(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	// The canvas is the window's whole content; the bar floats over its top-left corner.
	let canvas_size = [viewport.x(), viewport.y()];
	let mut root = ctx
		.element("graph")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(canvas_size[0].into())
				.height(canvas_size[1].into())
		})
		.await;
	let mut canvas = base(&mut root).await;
	let (title, id) = ctx
		.with(|shell: &mut Shell| {
			let graph = shell
				.layers
				.graph()
				.expect("The graph canvas shows only while a graph is open.");
			(graph.file_name().to_string(), graph.id.clone())
		})
		.await;
	// Declared after the canvas, beside it, so it draws over the nodes the canvas holds and takes its own presses.
	let mut bar = root
		.element("bar")
		.container(|c| {
			c.position(Position::anchored(Anchor::Start(MARGIN), Anchor::Start(MARGIN)))
				.width(Sizing::fit())
				.height(BAR_HEIGHT.into())
				.padding(14.0, 0.0)
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.hit_testable(true)
				.flow(flow::centered_row_with_gap(8))
				.style(panel(0.9))
		})
		.await;
	bar.element("name")
		.text(&title, |t| t.font_size(13.0).style([solid(TEXT)]))
		.await;
	bar.element("id").text(&id, |t| t.font_size(11.0).style([solid(MUTED)])).await;
	let mut status = bar
		.element("status")
		.text("", |t| t.font_size(11.0).style([solid(MUTED)]))
		.await;
	let mut save = bar
		.element("save")
		.container(|c| {
			c.width(SAVE_WIDTH.into())
				.height(24.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	let mut save_label = save
		.element("label")
		.text("Save", |t| t.font_size(12.0).style([solid(MUTED)]))
		.await;

	let bounds = ctx
		.with(|shell: &mut Shell| shell.layers.graph().map(Graph::bounds))
		.await
		.unwrap_or_default();
	let mut view = View::fitting(bounds, canvas_size);
	// The wire handles read the zoom when they turn a drag's pixels into graph units.
	let zoom = Rc::new(Cell::new(view.zoom));
	let mut world = draw_world(ctx, &mut canvas, view, &zoom).await;

	let mut pan_start = view.pan;
	let mut save_hovered = false;
	// The graph's revisions the bar and canvas last showed: its rows', and its values' and status's.
	let mut shown_shape = ctx
		.with(|shell: &mut Shell| shell.layers.graph().map_or(0, |graph| graph.shape_revision))
		.await;
	let mut shown_status = None;
	// The wire being dragged, as the canvas last drew it, and its curve.
	let mut shown_pending: Option<([f32; 2], [f32; 2])> = None;
	let mut pending_curve: Option<EvaluationContext<Shell>> = None;
	// What the canvas and bar show of the shell: whether the graph is open, its revisions, the window's size, and the wire
	// being dragged, from its source's output to its loose end.
	let mut state = ctx.proactive(|shell: &Shell| {
		let graph = shell.layers.graph();
		let pending = graph.and_then(|graph| {
			let wire = graph.pending?;
			Some((graph.nodes.get(wire.from)?.output(), wire.end))
		});
		(
			graph.is_some(),
			graph.map_or(0, |graph| graph.shape_revision),
			graph.map_or(0, |graph| graph.revision),
			shell.viewport,
			pending,
		)
	});
	loop {
		select! {
			_ = canvas.on(Events::Grabbed) => pan_start = view.pan,
			event = canvas.on(Events::Dragged) => {
				if let Some(delta) = event.delta {
					view.pan = [pan_start[0] + delta.x, pan_start[1] + delta.y];
					world.update_container(|c| c.transform(view.transform())).await;
				}
			},
			event = canvas.on(Events::Scrolled) => {
				if let Some(delta) = event.delta.filter(|delta| delta.y != 0.0) {
					let pointer = ctx.pointer().await.position;
					let anchor = canvas
						.geometry()
						.await
						.map(|geometry| geometry.logical)
						.map_or([0.0, 0.0], |geometry| [pointer.x - geometry.x(), pointer.y - geometry.y()]);
					let factor = if delta.y > 0.0 { ZOOM_STEP } else { 1.0 / ZOOM_STEP };
					view = view.zoomed(factor, anchor);
					zoom.set(view.zoom);
					world.update_container(|c| c.transform(view.transform())).await;
				}
			},
			_ = save.on(Events::Actuated) => {
				ctx.with(|shell: &mut Shell| shell.save_graph = true).await;
			},
			_ = save.on(Events::PointerEntered) => save_hovered = true,
			_ = save.on(Events::PointerExited) => save_hovered = false,
			_ = state.next() => {},
		}
		save.update_container(|c| c.style(row_style(false, save_hovered))).await;
		// Another branch can win before the first read resolves; the next pass shows the state then.
		let Some(&(open, shape, status_revision, now, pending)) = state.latest() else {
			continue;
		};
		// The graph closing removes the canvas, and the window resizing declares it again, fitted to the window.
		if !open || now != viewport {
			return root;
		}
		if shape != shown_shape {
			shown_shape = shape;
			world.remove().await;
			world = draw_world(ctx, &mut canvas, view, &zoom).await;
			// The curve went with the old nodes.
			(shown_pending, pending_curve) = (None, None);
		}
		if pending != shown_pending {
			shown_pending = pending;
			if let Some(mut curve) = pending_curve.take() {
				curve.remove().await;
			}
			if let Some((start, end)) = pending {
				pending_curve = Some(pending_wire(&mut world, start, end).await);
			}
		}
		// The status shows why the last edit or save failed, or that the graph has unsaved changes.
		if shown_status != Some(status_revision) {
			shown_status = Some(status_revision);
			let (text, color, unsaved) = ctx
				.with(|shell: &mut Shell| {
					let graph = shell.layers.graph()?;
					Some(match &graph.notice {
						Some(notice) => (first_sentence(notice).to_string(), AXIS_COLORS[0], graph.unsaved),
						None if graph.unsaved => ("Unsaved changes".to_string(), MUTED, true),
						None => (String::new(), MUTED, false),
					})
				})
				.await
				.unwrap_or_default();
			status.update_text(|t| t.content(&text).style([solid(color)])).await;
			save_label
				.update_text(|t| t.style([solid(if unsaved { TEXT } else { MUTED })]))
				.await;
		}
	}
}

/// Declares the graph's nodes and wires under `canvas`, in graph units, in a layer the view's transform pans and
/// scales; returns the layer, which panning and zooming transform again. The wire handles read the live `zoom`.
async fn draw_world(
	ctx: &mut EvaluationContext<Shell>,
	canvas: &mut EvaluationContext<Shell>,
	view: View,
	zoom: &Rc<Cell<f32>>,
) -> EvaluationContext<Shell> {
	let (nodes, links, wires, [_, _, right, bottom]) = ctx
		.with(|shell: &mut Shell| {
			let graph = shell
				.layers
				.graph()
				.expect("The graph canvas draws only while a graph is open.");
			// Each node, and whether a wire leaves its output.
			let nodes = graph
				.nodes
				.iter()
				.enumerate()
				.map(|(index, node)| (node.clone(), graph.wires.iter().any(|wire| wire.from == index)))
				.collect::<Vec<_>>();
			let wires = graph.wires.iter().map(|wire| graph.wire_ends(wire)).collect::<Vec<_>>();
			(nodes, graph.wires.clone(), wires, graph.bounds())
		})
		.await;

	let mut world = canvas
		.element("world")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width((right + SOCKET).max(1.0).into())
				.height((bottom + SOCKET).max(1.0).into())
				.transform(view.transform())
				.clip(false)
		})
		.await;

	// The wires go under the nodes, as one curve: each runs out of its output to the right and into its input from
	// the left.
	if !wires.is_empty() {
		world
			.element("wires")
			.curve(|curve| {
				let mut curve = curve
					.width((right + SOCKET).into())
					.height((bottom + SOCKET).into())
					.style(ConcreteLayer::default().color(hex(0x8A8F98, 0.85).into()).stroke(2.0));
				for ([x0, y0], [x1, y1]) in &wires {
					let bend = ((x1 - x0).abs() * 0.5).max(40.0);
					curve = curve.cubic((*x0, *y0), (x0 + bend, *y0), (x1 - bend, *y1), (*x1, *y1));
				}
				curve
			})
			.await;
	}
	// Each wire's ends hold the outer halves of the plugs its sockets show the inner halves of.
	for (index, (from, to)) in wires.iter().enumerate() {
		plug(&mut world, ("plug out", index), *from, Half::Right).await;
		plug(&mut world, ("plug in", index), *to, Half::Left).await;
	}

	for (index, (graph_node, feeds)) in nodes.iter().enumerate() {
		let ([x, y], height) = (graph_node.position, graph_node.height());
		let mut node = world
			.element(("node", index))
			.container(|c| {
				c.absolute_position(x, y)
					.width(NODE_WIDTH.into())
					.height(height.into())
					.corner_radius(8.0)
					.corner_exponent(SQUIRCLE)
					.clip(false)
					.style([
						ConcreteLayer::default()
							.color(hex(0x000000, 0.35).into())
							.drop_shadow([0.0, 4.0], 10.0),
						ConcreteLayer::default().color(vertical(hex(0x2C2D33, 1.0), hex(0x232429, 1.0))),
						top_highlight(0.07),
						hairline(0.55),
					])
			})
			.await;
		let mut header = node
			.element("header")
			.container(|c| {
				c.absolute_position(0.0, 0.0)
					.width(NODE_WIDTH.into())
					.height(HEADER_HEIGHT.into())
					.padding(10.0, 0.0)
					.flow(flow::centered_row_with_gap(6))
			})
			.await;
		header
			.element("name")
			.text(&graph_node.name, |t| t.font_size(12.5).style([solid(TEXT)]))
			.await;
		header
			.element("kind")
			.text(&graph_node.kind_name, |t| t.font_size(11.0).style([solid(MUTED)]))
			.await;
		// A line under the header, in the node's tint when it has one: the surface, or an unknown kind.
		let rule = graph_node.tint().unwrap_or(hex(0x000000, 0.4));
		node.element("rule")
			.container(|c| {
				c.absolute_position(0.0, HEADER_HEIGHT - 1.0)
					.width(NODE_WIDTH.into())
					.height(1.into())
					.style([solid(rule)])
			})
			.await;
		if graph_node.kind.is_none_or(|kind| kind.produces) {
			let grip = PendingWire {
				from: index,
				detach: None,
				end: graph_node.output(),
			};
			socket(
				&mut node,
				"output",
				[NODE_WIDTH, HEADER_HEIGHT * 0.5],
				Half::Left,
				zoom,
				*feeds,
				Some(grip),
			)
			.await;
		}
		node.element("remove")
			.container(|c| {
				c.absolute_position(NODE_WIDTH - 26.0, (HEADER_HEIGHT - 20.0) * 0.5)
					.width(20.into())
					.height(20.into())
			})
			.await
			.element("button")
			.component(async move |ctx| remove_button(ctx, index).await)
			.await;

		for (row_index, row) in graph_node.rows.iter().enumerate() {
			let row_top = HEADER_HEIGHT + row_index as f32 * ROW_HEIGHT;
			// Inputs have sockets, filled while a wire drives them; every value but a wired input can be edited.
			let connected = match row {
				Row::Input { source, .. } => Some(*source == Source::Node),
				Row::Setting { .. } => None,
			};
			let mut line = node
				.element(("row", row_index))
				.container(|c| {
					c.absolute_position(0.0, row_top)
						.width(NODE_WIDTH.into())
						.height(ROW_HEIGHT.into())
						.padding(14.0, 0.0)
						.flow(flow::centered_row_with_gap(6))
				})
				.await;
			line.element("label")
				.text(row.field(), |t| t.font_size(11.0).style([solid(MUTED)]))
				.await;
			if connected != Some(true) {
				// Values line up in a column halfway across the node.
				let mut column = line
					.element("value")
					.container(|c| {
						c.absolute_position(NODE_WIDTH * 0.44, ROW_HEIGHT * 0.1)
							.width((NODE_WIDTH * 0.52).into())
							.height((ROW_HEIGHT * 0.8).into())
					})
					.await;
				column
					.element("field")
					.component(async move |ctx| value_field(ctx, index, row_index).await)
					.await;
			}
			if let Some(connected) = connected {
				// A wired input's socket picks its wire up; the wire leaves its source's output.
				let grip = connected
					.then(|| links.iter().find(|wire| wire.to == index && wire.row == row_index))
					.flatten()
					.map(|wire| PendingWire {
						from: wire.from,
						detach: Some((index, row_index)),
						end: graph_node.input(row_index),
					});
				socket(
					&mut node,
					&format!("input {row_index}"),
					[0.0, row_top + ROW_HEIGHT * 0.5],
					Half::Right,
					zoom,
					connected,
					grip,
				)
				.await;
			}
		}
	}
	world
}

/// The value of row `row` of node `node`, as a field: dragging across a number changes it by [`SCRUB_STEP`] a pixel,
/// and a click without dragging takes typed text. It shows the value again whenever the graph's values change.
async fn value_field(ctx: &mut EvaluationContext<Shell>, node: usize, row: usize) {
	let mut well = ctx
		.element("well")
		.container(|c| {
			c.width((NODE_WIDTH * 0.52).into())
				.height((ROW_HEIGHT * 0.8).into())
				.corner_radius(4.0)
				.corner_exponent(SQUIRCLE)
				.clip(true)
				.padding(8.0, 0.0)
				.flow(flow::centered_row_with_gap(4))
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	let mut value = well
		.element("value")
		.text("", |t| t.font_size(11.0).font(VALUE_FONT).style([solid(TEXT)]))
		.await;

	// The number when the pointer pressed, which a drag measures from; `None` for a value that isn't one number.
	let mut start: Option<Option<f64>> = None;
	let mut dragged = false;
	let mut hovered = false;
	let mut shown_hover = false;
	let mut shown = None;
	let mut revision = ctx.proactive(|shell: &Shell| shell.layers.graph().map(|graph| graph.revision));
	loop {
		select! {
			_ = well.on(Events::Grabbed) => {
				start = Some(ctx.with(move |shell: &mut Shell| {
					let graph = shell.layers.graph()?;
					graph.number(node, graph.nodes.get(node)?.rows.get(row)?.field())
				}).await);
				dragged = false;
			},
			event = well.on(Events::Dragged) => {
				dragged = true;
				if let (Some(Some(start)), Some(delta)) = (start, event.delta) {
					// Two decimals, as the canvas shows numbers.
					let number = ((start + f64::from(delta.x) * SCRUB_STEP) * 100.0).round() / 100.0;
					ctx.with(move |shell: &mut Shell| edit(shell, node, row, |_, _| Ok(Some(Json::from(number))))).await;
				}
			},
			_ = well.on(Events::DragEnded) => {
				if !dragged && start.is_some() {
					value.update_text(|t| t.content("")).await;
					let styles = [typing_style(false), row_style(false, false)];
					if let Some(typed) = read_typed(ctx, &mut well, styles, 11.0, "").await {
						ctx.with(move |shell: &mut Shell| {
							edit(shell, node, row, |graph, field| graph.parse(node, field, &typed));
						})
						.await;
					}
					shown = None;
				}
				start = None;
			},
			_ = well.on(Events::PointerEntered) => hovered = true,
			_ = well.on(Events::PointerExited) => hovered = false,
			_ = revision.next() => {},
		}
		// The value is shown again only when the graph's values changed.
		let revision = revision.latest().copied().flatten();
		if revision.is_some() && shown != revision {
			shown = revision;
			let (text, color) = ctx
				.with(move |shell: &mut Shell| {
					let row = shell.layers.graph()?.nodes.get(node)?.rows.get(row)?;
					Some(shown_value(row))
				})
				.await
				.unwrap_or_default();
			value.update_text(|t| t.content(&text).style([solid(color)])).await;
		}
		if hovered != shown_hover {
			shown_hover = hovered;
			well.update_container(|c| c.style(row_style(false, hovered))).await;
		}
	}
}

/// Edits the field row `row` of node `node` shows to the value `value` makes of it.
fn edit(shell: &mut Shell, node: usize, row: usize, value: impl FnOnce(&Graph, &str) -> Result<Option<Json>, String>) {
	change(shell, |graph| {
		let Some(field) = graph
			.nodes
			.get(node)
			.and_then(|node| node.rows.get(row))
			.map(|row| row.field().to_string())
		else {
			return Ok(());
		};
		let value = value(graph, &field)?;
		graph.set(node, &field, value);
		Ok(())
	});
}

/// Changes the open graph with `change`, or shows why it refused.
fn change(shell: &mut Shell, change: impl FnOnce(&mut Graph) -> Result<(), String>) {
	let Some(graph) = shell.layers.graph_mut() else {
		return;
	};
	if let Err(error) = change(graph) {
		log::warn!("The editor couldn't change the material graph. {error}");
		graph.fail(error);
	}
}

/// The text a row's value shows, and its color: faded for a value its kind gives, and empty for none.
fn shown_value(row: &Row) -> (String, RGBA) {
	match row {
		Row::Input {
			source: Source::Constant(value),
			..
		}
		| Row::Setting { value: Some(value), .. } => (value.clone(), TEXT),
		Row::Input {
			source: Source::Default(value),
			..
		} => (value.clone(), DEFAULT_VALUE),
		_ => (String::new(), DEFAULT_VALUE),
	}
}

/// The first sentence of `text`, which the title bar has room for; the log has the rest.
fn first_sentence(text: &str) -> &str {
	text.find(". ").map_or(text, |end| &text[..=end])
}

/// Which half of a plug's circle an element draws: the half left or right of the node's edge it straddles.
#[derive(Clone, Copy)]
enum Half {
	Left,
	Right,
}

impl Half {
	/// Declares this half of a plug under `square`, a [`SOCKET`]-wide square centered on the plug, with `style`: the
	/// plug's circle shrunk by `inset` on every side, clipped to this half, so both halves and the [`ring`] around them
	/// share one outline.
	async fn draw(self, square: &mut EvaluationContext<Shell>, inset: f32, style: impl IntoIterator<Item = ConcreteLayer>) {
		let radius = SOCKET * 0.5;
		// The clip's left edge in the square, and the circle's left edge in the clip.
		let (left, circle) = match self {
			Half::Left => (0.0, inset),
			Half::Right => (radius, inset - radius),
		};
		square
			.element("half")
			.container(|c| {
				c.absolute_position(left, 0.0)
					.width(radius.into())
					.height(SOCKET.into())
					.clip(true)
			})
			.await
			.element("circle")
			.container(|c| {
				c.absolute_position(circle, inset)
					.width((SOCKET - inset * 2.0).into())
					.height((SOCKET - inset * 2.0).into())
					.corner_radius(radius - inset)
					.style(style)
			})
			.await;
	}
}

/// Declares a [`SOCKET`]-wide square centered on `center`, which a plug's halves draw in.
async fn plug_square(
	parent: &mut EvaluationContext<Shell>,
	key: impl Into<ElementKey>,
	center: [f32; 2],
) -> EvaluationContext<Shell> {
	parent
		.element(key)
		.container(|c| {
			c.absolute_position(center[0] - SOCKET * 0.5, center[1] - SOCKET * 0.5)
				.width(SOCKET.into())
				.height(SOCKET.into())
				.clip(false)
		})
		.await
}

/// Declares the half of a plug a wire's end holds, at `center`, on the `half` of it outside the node: a knob lit
/// from above. It has no edge of its own: the socket draws one ring around both halves, so they read as one circle.
async fn plug(world: &mut EvaluationContext<Shell>, key: impl Into<ElementKey>, center: [f32; 2], half: Half) {
	let mut square = plug_square(world, key, center).await;
	half.draw(
		&mut square,
		RING,
		[
			ConcreteLayer::default().color(vertical(hex(0xD4D6DC, 1.0), hex(0x9A9EA7, 1.0))),
			top_highlight(0.5),
		],
	)
	.await;
}

/// Declares the ring around a whole plug, over both its halves, in `square`: the one edge they share.
async fn ring(square: &mut EvaluationContext<Shell>) {
	square
		.element("ring")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(SOCKET.into())
				.height(SOCKET.into())
				.corner_radius(SOCKET * 0.5)
				.style([ConcreteLayer::default().color(hex(0x000000, 0.6).into()).stroke(RING)])
		})
		.await;
}

/// Declares a socket centered on `center`: the `half` of a plug inside the node, a recess cut into its edge that
/// glows in the accent while a wire's plug sits in it. A socket with a `grip` drags a wire out of it, starting as
/// `grip` describes.
async fn socket(
	node: &mut EvaluationContext<Shell>,
	key: &str,
	center: [f32; 2],
	half: Half,
	zoom: &Rc<Cell<f32>>,
	connected: bool,
	grip: Option<PendingWire>,
) {
	let mut place = plug_square(node, key, center).await;
	match grip {
		Some(grip) => {
			place
				.element("handle")
				.component({
					let zoom = zoom.clone();
					async move |ctx| wire_handle(ctx, grip, half, zoom, connected).await
				})
				.await;
		}
		None => {
			half.draw(&mut place, inset(connected), receptacle(connected)).await;
			if connected {
				ring(&mut place).await;
			}
		}
	}
}

/// A socket that drags a wire: pressing it starts `grip` as the graph's pending wire, dragging moves the wire's loose
/// end, and releasing drops it with [`Graph::drop_wire`]. The whole plug's square takes the press, so the pointer
/// needn't find the half the socket draws.
async fn wire_handle(ctx: &mut EvaluationContext<Shell>, grip: PendingWire, half: Half, zoom: Rc<Cell<f32>>, connected: bool) {
	let mut handle = ctx
		.element("grip")
		.container(|c| {
			// A transparent fill keeps the square a surface the pointer can reach while it paints nothing.
			c.width(SOCKET.into())
				.height(SOCKET.into())
				.clip(false)
				.hit_testable(true)
				.style([solid(hex(0x000000, 0.0))])
		})
		.await;
	half.draw(&mut handle, inset(connected), receptacle(connected)).await;
	// The node draws over the wires, so the ring around both halves goes here, over the plug's half too.
	if connected {
		ring(&mut handle).await;
	}
	loop {
		select! {
			_ = handle.on(Events::Grabbed) => {
				ctx.with(move |shell: &mut Shell| {
					if let Some(graph) = shell.layers.graph_mut() {
						graph.pending = Some(grip);
					}
				})
				.await;
			},
			event = handle.on(Events::Dragged) => {
				if let Some(delta) = event.delta {
					// The drag's delta is in pixels, and the graph's units are pixels at a zoom of 1.
					let z = zoom.get();
					let end = [grip.end[0] + delta.x / z, grip.end[1] + delta.y / z];
					ctx.with(move |shell: &mut Shell| {
						if let Some(wire) = shell.layers.graph_mut().and_then(|graph| graph.pending.as_mut()) {
							wire.end = end;
						}
					})
					.await;
				}
			},
			_ = handle.on(Events::DragEnded) => {
				ctx.with(|shell: &mut Shell| {
					change(shell, |graph| graph.pending.take().map_or(Ok(()), |wire| graph.drop_wire(wire)));
				})
				.await;
			},
		}
	}
}

/// How far a socket's half sits inside the plug's circle: under the [`ring`] while a wire's plug fills the socket, and
/// not at all while it's empty, when it draws its own edge.
fn inset(connected: bool) -> f32 {
	if connected { RING } else { 0.0 }
}

/// A socket's half of a plug: an empty recess cut into the node, or, while a wire's plug sits in it, the accent lit
/// from above like the knob, with no edge of its own, since the ring around the plug is its edge.
fn receptacle(connected: bool) -> [ConcreteLayer; 3] {
	if connected {
		[
			ConcreteLayer::default().color(vertical(hex(0x6BE287, 1.0), ACCENT)),
			top_highlight(0.5),
			hairline(0.0),
		]
	} else {
		[
			solid(hex(0x0E0F11, 1.0)),
			ConcreteLayer::default()
				.color(hex(0x000000, 0.75).into())
				.inset_shadow([0.0, 2.0], 2.5),
			hairline(0.6),
		]
	}
}

/// The wire being dragged, from `start`, its source's output, to its loose `end`, both on the canvas, drawn under
/// `world` over the nodes; returns its curve, which the canvas removes when the wire moves or drops.
async fn pending_wire(world: &mut EvaluationContext<Shell>, start: [f32; 2], end: [f32; 2]) -> EvaluationContext<Shell> {
	// The curve's own box spans both ends and the bend, so it never reaches past the world's edges.
	let bend = ((end[0] - start[0]).abs() * 0.5).max(40.0);
	// It also leaves room for the plugs it holds at both ends.
	let margin = SOCKET * 0.5 + 2.0;
	let left = start[0].min(end[0] - bend) - margin;
	let top = start[1].min(end[1]) - margin;
	let right = end[0].max(start[0] + bend) + margin;
	let bottom = start[1].max(end[1]) + margin;
	let local = |[x, y]: [f32; 2]| (x - left, y - top);
	// Curves place in their parent's flow, so a container places this one.
	let mut place = world
		.element("pending")
		.container(|c| {
			c.absolute_position(left, top)
				.width((right - left).into())
				.height((bottom - top).into())
		})
		.await;
	place
		.element("wire")
		.curve(|curve| {
			curve
				.width((right - left).into())
				.height((bottom - top).into())
				.style(ConcreteLayer::default().color(ACCENT.into()).stroke(2.0))
				.cubic(
					local(start),
					local([start[0] + bend, start[1]]),
					local([end[0] - bend, end[1]]),
					local(end),
				)
		})
		.await;
	// The plug that leaves the source's output, and the one its loose end carries toward an input.
	plug(&mut place, "plug out", local(start).into(), Half::Right).await;
	plug(&mut place, "plug in", local(end).into(), Half::Left).await;
	place
}

/// A node's × in its header's right corner, which removes the node from the graph.
async fn remove_button(ctx: &mut EvaluationContext<Shell>, node: usize) {
	let mut button = ctx
		.element("well")
		.container(|c| {
			c.width(20.into())
				.height(20.into())
				.corner_radius(5.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	button
		.element("cross")
		.text("×", |t| t.font_size(14.0).style([solid(MUTED)]))
		.await;
	loop {
		select! {
			_ = button.on(Events::Actuated) => {
				ctx.with(move |shell: &mut Shell| {
					change(shell, |graph| {
						graph.remove(node);
						Ok(())
					});
				})
				.await;
			},
			_ = button.on(Events::PointerEntered) => {
				button.update_container(|c| c.style(row_style(false, true))).await;
			},
			_ = button.on(Events::PointerExited) => {
				button.update_container(|c| c.style(row_style(false, false))).await;
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_fitted_view_centers_the_graph_without_zooming_in() {
		let view = View::fitting([0.0, 0.0, 400.0, 200.0], [800.0, 600.0]);
		assert_eq!(view.zoom, 1.0);
		assert_eq!(view.pan, [200.0, 200.0]);
		let tight = View::fitting([0.0, 0.0, 2000.0, 200.0], [800.0, 600.0]);
		assert!((tight.zoom - 0.36).abs() < 1e-5, "{}", tight.zoom);
	}

	#[test]
	fn the_status_shows_the_first_sentence_of_a_notice() {
		assert_eq!(first_sentence("It failed. Try again."), "It failed.");
		assert_eq!(first_sentence("It failed"), "It failed");
	}

	#[test]
	fn zooming_keeps_the_point_under_the_anchor() {
		let view = View {
			pan: [100.0, 50.0],
			zoom: 1.0,
		};
		let anchor = [300.0, 250.0];
		let graph_point = |view: View| [(anchor[0] - view.pan[0]) / view.zoom, (anchor[1] - view.pan[1]) / view.zoom];
		let zoomed = view.zoomed(ZOOM_STEP, anchor);
		let [x, y] = graph_point(zoomed);
		assert!((x - 200.0).abs() < 1e-3 && (y - 200.0).abs() < 1e-3, "{:?}", [x, y]);
		assert_eq!(view.zoomed(100.0, anchor).zoom, ZOOM_RANGE.1);
	}
}
