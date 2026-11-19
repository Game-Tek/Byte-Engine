//! The graph canvas: while a material graph is open, a panel over the game view that draws its nodes and wires.
//!
//! Dragging the canvas pans it, and scrolling zooms it about the pointer. A node's values are fields: dragging across
//! a number changes it, and clicking one takes typed text, where empty text returns the value to its default.
//! Dragging out of a node's output draws a wire that connects to the input it's dropped on; dragging a wire off an
//! input moves it, or disconnects it when dropped on empty canvas. A node's × removes it, and the palette adds nodes.
//! The title bar's Save button, or Cmd+S, asks the editor to write the graph. The panel is declared each time a graph
//! opens and removed when it closes; zooming, or an edit that changes the nodes, their rows, or the wires, declares
//! the nodes again.

use serde_json::Value as Json;

use super::{
	ACCENT, AXIS_COLORS, CORNER, MARGIN, MUTED, SQUIRCLE, Shell, TEXT,
	hairline, hex, panel, read_typed, row_style, solid, top_highlight, typing_style, vertical,
};
use crate::editor::layers::Layer;
use crate::editor::graph::{Graph, HEADER_HEIGHT, NODE_WIDTH, PendingWire, ROW_HEIGHT, Row, Source};
use crate::ui::components::container::Container;
use crate::ui::layout::context::{ContainerContext, Context, ElementContext};
use crate::ui::layout::engine::EvaluationContext;
use crate::ui::layout::engine::Properties;
use crate::ui::primitive::Events;
use crate::ui::{ConcreteLayer, flow};
use crate::utils::RGBA;
use crate::utils::r#async::select;

/// The height of the panel's title bar, over the canvas.
const TITLE_HEIGHT: f32 = 36.0;
/// The smallest and largest zoom.
const ZOOM_RANGE: (f32, f32) = (0.3, 2.0);
/// How much one scroll step zooms in or out.
const ZOOM_STEP: f32 = 1.15;
/// The width of the title bar's Save button.
const SAVE_WIDTH: f32 = 48.0;
/// A socket's diameter at a zoom of 1.
const SOCKET: f32 = 10.0;
/// How much dragging one pixel across a number changes it.
const SCRUB_STEP: f64 = 0.01;
/// The color of a value the file leaves out, which its kind gives.
const DEFAULT_VALUE: RGBA = hex(0x8A8F98, 0.6);

/// Shows the open graph's panel while a graph is open, declaring it again for each graph that opens.
pub(super) async fn graph_view(ctx: &mut EvaluationContext<Shell>) {
	loop {
		// Waiting on the layer reads it only when the shell changed, so a closed view asks for no frames.
		let mut open = ctx.proactive(|shell: &Shell| shell.layers.graph().is_some());
		while !*open.next().await {}
		let mut frame = graph_panel(ctx).await;
		frame.remove().await;
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

/// Declares the open graph's panel and keeps it current until the graph closes, then returns it for removal.
async fn graph_panel(ctx: &mut EvaluationContext<Shell>) -> EvaluationContext<Shell> {
	let viewport = ctx.with(|shell: &mut Shell| shell.viewport).await;
	// The panel fills the graph's window, inside its margin.
	let (left, top) = (MARGIN, MARGIN);
	let width = (viewport.x() - MARGIN * 2.0).max(200.0);
	let height = (viewport.y() - MARGIN * 2.0).max(TITLE_HEIGHT + 100.0);
	let canvas_size = [width - 2.0, height - TITLE_HEIGHT - 1.0];

	let mut frame = ctx
		.element("frame")
		.container(|c| {
			c.absolute_position(left, top)
				.width(width.into())
				.height(height.into())
				.corner_radius(CORNER)
				.corner_exponent(SQUIRCLE)
				.clip(false)
				.hit_testable(true)
				.style(panel(height, 1.0))
		})
		.await;
	let (title, id) = ctx
		.with(|shell: &mut Shell| {
			let graph = shell
				.layers
				.graph()
				.expect("The graph panel shows only while a graph is open.");
			(graph.file_name().to_string(), graph.id.clone())
		})
		.await;
	let mut bar = frame
		.element("title")
		.container(|c| {
			c.absolute_position(0.0, 0.0)
				.width(width.into())
				.height(TITLE_HEIGHT.into())
				.padding(14.0, 0.0)
				.flow(flow::centered_row_with_gap(8))
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
	let mut save = frame
		.element("save")
		.container(|c| {
			c.absolute_position(width - 32.0 - 6.0 - SAVE_WIDTH, (TITLE_HEIGHT - 24.0) * 0.5)
				.width(SAVE_WIDTH.into())
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
	let mut close = frame
		.element("close")
		.container(|c| {
			c.absolute_position(width - 32.0, (TITLE_HEIGHT - 24.0) * 0.5)
				.width(24.into())
				.height(24.into())
				.corner_radius(6.0)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	close
		.element("cross")
		.text("×", |t| t.font_size(16.0).style([solid(MUTED)]))
		.await;

	let mut canvas = frame
		.element("canvas")
		.container(|c| {
			c.absolute_position(1.0, TITLE_HEIGHT)
				.width(canvas_size[0].into())
				.height(canvas_size[1].into())
				.corner_radius(CORNER - 1.0)
				.corner_exponent(SQUIRCLE)
				.clip(true)
				.hit_testable(true)
				.style([solid(hex(0x141518, 1.0)), hairline(0.0), top_highlight(0.0)])
		})
		.await;
	let bounds = ctx
		.with(|shell: &mut Shell| shell.layers.graph().map(Graph::bounds))
		.await
		.unwrap_or_default();
	let mut view = View::fitting(bounds, canvas_size);
	let mut world = draw_world(ctx, &mut canvas, view).await;

	let mut pan_start = view.pan;
	let mut close_hovered = false;
	let mut save_hovered = false;
	// The graph's revisions the panel last showed: its rows', and its values' and status's.
	let mut shown_shape = ctx
		.with(|shell: &mut Shell| shell.layers.graph().map_or(0, |graph| graph.shape_revision))
		.await;
	let mut shown_status = None;
	// The wire being dragged, as the panel last drew it, and its curve.
	let mut shown_pending: Option<([f32; 2], [f32; 2])> = None;
	let mut pending_curve: Option<EvaluationContext<Shell>> = None;
	// What the panel shows of the shell: whether the graph is open, its revisions, the window's size, and the wire
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
					world.update_container(|c| c.absolute_position(view.pan[0], view.pan[1])).await;
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
					world.remove().await;
					world = draw_world(ctx, &mut canvas, view).await;
				}
			},
			_ = close.on(Events::Actuated) => {
				ctx.with(|shell: &mut Shell| shell.layers.close(|layer| matches!(layer, Layer::Graph(_)))).await;
			},
			_ = close.on(Events::PointerEntered) => close_hovered = true,
			_ = close.on(Events::PointerExited) => close_hovered = false,
			_ = save.on(Events::Actuated) => {
				ctx.with(|shell: &mut Shell| shell.save_graph = true).await;
			},
			_ = save.on(Events::PointerEntered) => save_hovered = true,
			_ = save.on(Events::PointerExited) => save_hovered = false,
			_ = state.next() => {},
		}
		close.update_container(|c| c.style(row_style(false, close_hovered))).await;
		save.update_container(|c| c.style(row_style(false, save_hovered))).await;
		// Another branch can win before the first read resolves; the next pass shows the state then.
		let Some(&(open, shape, status_revision, now, pending)) = state.latest() else {
			continue;
		};
		// The graph closing removes the panel, and the window resizing declares it again, fitted to the window.
		if !open || now != viewport {
			return frame;
		}
		if shape != shown_shape {
			shown_shape = shape;
			world.remove().await;
			world = draw_world(ctx, &mut canvas, view).await;
			// The curve went with the old nodes.
			(shown_pending, pending_curve) = (None, None);
		}
		if pending != shown_pending {
			shown_pending = pending;
			if let Some(mut curve) = pending_curve.take() {
				curve.remove().await;
			}
			if let Some((start, end)) = pending {
				pending_curve = Some(pending_wire(&mut world, start, end, view.zoom).await);
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

/// Declares the graph's nodes and wires under `canvas`, at the view's zoom, in a layer the view's pan places; returns
/// the layer, which panning moves and zooming declares again.
async fn draw_world(
	ctx: &mut EvaluationContext<Shell>,
	canvas: &mut EvaluationContext<Shell>,
	view: View,
) -> EvaluationContext<Shell> {
	let z = view.zoom;
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
			c.absolute_position(view.pan[0], view.pan[1])
				.width(((right + SOCKET) * z).max(1.0).into())
				.height(((bottom + SOCKET) * z).max(1.0).into())
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
					.width(((right + SOCKET) * z).into())
					.height(((bottom + SOCKET) * z).into())
					.style(ConcreteLayer::default().color(hex(0x8A8F98, 0.85).into()).stroke(2.0));
				for ([x0, y0], [x1, y1]) in &wires {
					let bend = ((x1 - x0).abs() * 0.5).max(40.0);
					curve = curve.cubic(
						(x0 * z, y0 * z),
						((x0 + bend) * z, y0 * z),
						((x1 - bend) * z, y1 * z),
						(x1 * z, y1 * z),
					);
				}
				curve
			})
			.await;
	}

	for (index, (graph_node, feeds)) in nodes.iter().enumerate() {
		let ([x, y], height) = (graph_node.position, graph_node.height());
		let mut node = world
			.element(("node", index))
			.container(|c| {
				c.absolute_position(x * z, y * z)
					.width((NODE_WIDTH * z).into())
					.height((height * z).into())
					.corner_radius(8.0 * z)
					.corner_exponent(SQUIRCLE)
					.clip(false)
					.style([
						ConcreteLayer::default()
							.color(hex(0x000000, 0.35).into())
							.drop_shadow([0.0, 4.0 * z], 10.0 * z),
						ConcreteLayer::default().color(vertical(height * z, hex(0x2C2D33, 1.0), hex(0x232429, 1.0))),
						top_highlight(0.07),
						hairline(0.55),
					])
			})
			.await;
		let mut header = node
			.element("header")
			.container(|c| {
				c.absolute_position(0.0, 0.0)
					.width((NODE_WIDTH * z).into())
					.height((HEADER_HEIGHT * z).into())
					.padding(10.0 * z, 0.0)
					.flow(flow::centered_row_with_gap((6.0 * z) as u32))
			})
			.await;
		header
			.element("name")
			.text(&graph_node.name, |t| t.font_size(12.5 * z).style([solid(TEXT)]))
			.await;
		header
			.element("kind")
			.text(&graph_node.kind_name, |t| t.font_size(11.0 * z).style([solid(MUTED)]))
			.await;
		// A line under the header, in the node's tint when it has one: the surface, or an unknown kind.
		let rule = graph_node.tint().unwrap_or(hex(0x000000, 0.4));
		node.element("rule")
			.container(|c| {
				c.absolute_position(0.0, HEADER_HEIGHT * z - 1.0)
					.width((NODE_WIDTH * z).into())
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
				[NODE_WIDTH * z, HEADER_HEIGHT * 0.5 * z],
				z,
				*feeds,
				Some(grip),
			)
			.await;
		}
		node.element("remove")
			.container(|c| {
				c.absolute_position((NODE_WIDTH - 26.0) * z, (HEADER_HEIGHT - 20.0) * 0.5 * z)
					.width((20.0 * z).into())
					.height((20.0 * z).into())
			})
			.await
			.element("button")
			.component(async move |ctx| remove_button(ctx, index, z).await)
			.await;

		for (row_index, row) in graph_node.rows.iter().enumerate() {
			let row_top = (HEADER_HEIGHT + row_index as f32 * ROW_HEIGHT) * z;
			// Inputs have sockets, filled while a wire drives them; every value but a wired input can be edited.
			let connected = match row {
				Row::Input { source, .. } => Some(*source == Source::Node),
				Row::Setting { .. } => None,
			};
			let mut line = node
				.element(("row", row_index))
				.container(|c| {
					c.absolute_position(0.0, row_top)
						.width((NODE_WIDTH * z).into())
						.height((ROW_HEIGHT * z).into())
						.padding(14.0 * z, 0.0)
						.flow(flow::centered_row_with_gap((6.0 * z) as u32))
				})
				.await;
			line.element("label")
				.text(row.field(), |t| t.font_size(11.0 * z).style([solid(MUTED)]))
				.await;
			if connected != Some(true) {
				// Values line up in a column halfway across the node.
				let mut column = line
					.element("value")
					.container(|c| {
						c.absolute_position(NODE_WIDTH * 0.44 * z, ROW_HEIGHT * 0.1 * z)
							.width((NODE_WIDTH * 0.52 * z).into())
							.height((ROW_HEIGHT * 0.8 * z).into())
					})
					.await;
				column
					.element("field")
					.component(async move |ctx| value_field(ctx, index, row_index, z).await)
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
					[0.0, row_top + ROW_HEIGHT * 0.5 * z],
					z,
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
async fn value_field(ctx: &mut EvaluationContext<Shell>, node: usize, row: usize, z: f32) {
	let mut well = ctx
		.element("well")
		.container(|c| {
			c.width((NODE_WIDTH * 0.52 * z).into())
				.height((ROW_HEIGHT * 0.8 * z).into())
				.corner_radius(4.0 * z)
				.corner_exponent(SQUIRCLE)
				.clip(true)
				.padding(8.0 * z, 0.0)
				.flow(flow::centered_row_with_gap((4.0 * z) as u32))
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	let mut value = well
		.element("value")
		.text("", |t| t.font_size(11.0 * z).style([solid(TEXT)]))
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
					if let Some(typed) = read_typed(ctx, &mut well, styles, 11.0 * z).await {
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

/// Declares a socket centered on `center`: filled with the accent while a wire meets it, hollow otherwise. A socket
/// with a `grip` drags a wire out of it, starting as `grip` describes.
async fn socket(
	node: &mut EvaluationContext<Shell>,
	key: &str,
	center: [f32; 2],
	z: f32,
	connected: bool,
	grip: Option<PendingWire>,
) {
	let size = SOCKET * z;
	let mut place = node
		.element(key)
		.container(|c| {
			c.absolute_position(center[0] - size * 0.5, center[1] - size * 0.5)
				.width(size.into())
				.height(size.into())
		})
		.await;
	match grip {
		Some(grip) => {
			place
				.element("handle")
				.component(async move |ctx| wire_handle(ctx, grip, z, connected).await)
				.await;
		}
		None => {
			place.element("dot").container(|c| socket_container(c, size, connected)).await;
		}
	}
}

/// A socket that drags a wire: pressing it starts `grip` as the graph's pending wire, dragging moves the wire's loose
/// end, and releasing drops it with [`Graph::drop_wire`].
async fn wire_handle(ctx: &mut EvaluationContext<Shell>, grip: PendingWire, z: f32, connected: bool) {
	let size = SOCKET * z;
	let mut handle = ctx
		.element("dot")
		.container(|c| socket_container(c, size, connected).hit_testable(true))
		.await;
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

/// A socket's circle, `size` pixels across, filled with the accent while a wire meets it.
fn socket_container<'s>(c: Properties<'s, Container>, size: f32, connected: bool) -> Properties<'s, Container> {
	let fill = if connected { ACCENT } else { hex(0x16171A, 1.0) };
	c.width(size.into()).height(size.into()).corner_radius(size * 0.5).style([
		solid(fill),
		top_highlight(0.0),
		ConcreteLayer::default().color(hex(0x8A8F98, 0.9).into()).stroke(1.0),
	])
}

/// The wire being dragged, from `start`, its source's output, to its loose `end`, both on the canvas, drawn under
/// `world` at zoom `z` over the nodes; returns its curve, which the panel removes when the wire moves or drops.
async fn pending_wire(
	world: &mut EvaluationContext<Shell>,
	start: [f32; 2],
	end: [f32; 2],
	z: f32,
) -> EvaluationContext<Shell> {
	// The curve's own box spans both ends and the bend, so it never reaches past the world's edges.
	let bend = ((end[0] - start[0]).abs() * 0.5).max(40.0);
	let left = start[0].min(end[0] - bend) - 2.0;
	let top = start[1].min(end[1]) - 2.0;
	let right = end[0].max(start[0] + bend) + 2.0;
	let bottom = start[1].max(end[1]) + 2.0;
	let local = |[x, y]: [f32; 2]| ((x - left) * z, (y - top) * z);
	// Curves place in their parent's flow, so a container places this one.
	let mut place = world
		.element("pending")
		.container(|c| {
			c.absolute_position(left * z, top * z)
				.width(((right - left) * z).into())
				.height(((bottom - top) * z).into())
		})
		.await;
	place
		.element("wire")
		.curve(|curve| {
			curve
				.width(((right - left) * z).into())
				.height(((bottom - top) * z).into())
				.style(ConcreteLayer::default().color(ACCENT.into()).stroke(2.0))
				.cubic(
					local(start),
					local([start[0] + bend, start[1]]),
					local([end[0] - bend, end[1]]),
					local(end),
				)
		})
		.await;
	place
}

/// A node's × in its header's right corner, which removes the node from the graph.
async fn remove_button(ctx: &mut EvaluationContext<Shell>, node: usize, z: f32) {
	let mut button = ctx
		.element("well")
		.container(|c| {
			c.width((20.0 * z).into())
				.height((20.0 * z).into())
				.corner_radius(5.0 * z)
				.corner_exponent(SQUIRCLE)
				.flow(flow::center)
				.hit_testable(true)
				.style(row_style(false, false))
		})
		.await;
	button
		.element("cross")
		.text("×", |t| t.font_size(14.0 * z).style([solid(MUTED)]))
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
