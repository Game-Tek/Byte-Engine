//! A material graph, a `.bemx` file, as the graph canvas shows it: its nodes, the wires between them, and where each
//! node sits.
//!
//! This module reads the file's document into nodes and wires, lays the nodes out, and edits the document: its
//! values, its wires, and which nodes it has;
//! drawing them is the shell's job, and reading and writing the file the editor's, through the inspector. Positions
//! aren't stored in the file: every time a graph opens or a node's rows change, its nodes are laid out in columns by
//! how far they are from the surface, which sits on the right.

use resource_management::bemx::Type;
use resource_management::bemx::kinds::{self, InputDefault, NodeKind, SettingKind};
use serde_json::Value as Json;

use crate::utils::RGBA;

/// The side of one cell of the canvas's grid, in canvas units, which are pixels at a zoom of 1. Every node's size and
/// place is a whole number of cells, so nodes sit on the grid's lines. The grid material's `spacing`, in
/// `assets/editor/graph-grid.bema`, must match it.
pub(crate) const GRID: f32 = 24.0;
/// A node's width on the canvas: eight cells.
pub(crate) const NODE_WIDTH: f32 = 8.0 * GRID;
/// The height of a node's header, which holds its name and kind. With [`FOOTER_HEIGHT`] it makes two cells.
pub(crate) const HEADER_HEIGHT: f32 = 32.0;
/// The height of one row of a node, an input or a setting: one cell.
pub(crate) const ROW_HEIGHT: f32 = GRID;
/// The space under a node's last row; with the header it fills two cells, so every node is a whole number of cells
/// tall.
const FOOTER_HEIGHT: f32 = 2.0 * GRID - HEADER_HEIGHT;
/// The gaps between columns and between the nodes of a column, in whole cells.
const COLUMN_GAP: f32 = 4.0 * GRID;
const NODE_GAP: f32 = GRID;

/// The `Graph` struct is one open material graph: the file's document and how the canvas shows it.
#[derive(Debug)]
pub(crate) struct Graph {
	/// The source asset's ID, such as `materials/floor.bemx`.
	pub(crate) id: String,
	/// The file's document, which edits change and saving writes; the nodes and wires are read from it.
	document: Json,
	pub(crate) nodes: Vec<GraphNode>,
	pub(crate) wires: Vec<Wire>,
	/// The type each node produces, by node, which colors the wires it drives; `None` for the surface and for nodes
	/// that don't read yet.
	pub(crate) types: Vec<Option<Type>>,
	/// The input sockets the wire being dragged fits, as each one's node and row and its place on the canvas, which
	/// [`Graph::grab`] lists once so a drag doesn't check types every frame.
	offered: Vec<((usize, usize), [f32; 2])>,
	/// Whether the document changed since it was opened or last saved.
	pub(crate) unsaved: bool,
	/// Advanced whenever an edit changes the nodes, their rows, or the wires, so the canvas declares them again.
	pub(crate) shape_revision: u64,
	/// The wire being dragged on the canvas, until it's dropped.
	pub(crate) pending: Option<PendingWire>,
	/// Why the last edit or save failed, until the next one succeeds.
	pub(crate) notice: Option<String>,
	/// Advanced whenever a value, `unsaved`, or `notice` changes, so the canvas knows to show them again.
	pub(crate) revision: u64,
}

/// The `GraphNode` struct is one node of an open graph as the canvas shows it.
#[derive(Clone, Debug)]
pub(crate) struct GraphNode {
	pub(crate) name: String,
	/// The kind's name, as the file spells it, even when BEMX has no such kind.
	pub(crate) kind_name: String,
	pub(crate) kind: Option<&'static NodeKind>,
	pub(crate) rows: Vec<Row>,
	/// The node's top-left corner on the canvas.
	pub(crate) position: [f32; 2],
}

/// The `Row` enum is one line of a node under its header: an input, with what drives it, or a setting, with its value
/// when the file writes one. Its name is the node's field it edits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Row {
	Input { name: String, source: Source },
	Setting { name: String, value: Option<String> },
}

impl Row {
	/// The node's field the row shows, such as `in2` or `texture`.
	pub(crate) fn field(&self) -> &str {
		match self {
			Row::Input { name, .. } | Row::Setting { name, .. } => name,
		}
	}
}

/// The `Source` enum is what an input reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Source {
	/// Another node's value; a [`Wire`] draws it.
	Node,
	/// A constant written in the file.
	Constant(String),
	/// The default its kind gives it, when the file leaves it out.
	Default(String),
	/// Nothing: an optional input the file leaves out, or a required one it's missing.
	Unset,
}

/// How far from an input socket a dropped wire's end still plugs into it: three quarters of a row, so a drop anywhere
/// in a row's band beside its node plugs into that row, and the next row's socket is a short move away.
pub(crate) const PLUG_REACH: f32 = ROW_HEIGHT * 0.75;

/// The distance between two points on the canvas.
pub(crate) fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
	(a[0] - b[0]).hypot(a[1] - b[1])
}

/// The socket among `sockets` nearest `point`, as its node and row and its place on the canvas.
fn nearest(sockets: &[((usize, usize), [f32; 2])], point: [f32; 2]) -> Option<((usize, usize), [f32; 2])> {
	sockets
		.iter()
		.copied()
		.min_by(|(_, a), (_, b)| distance(point, *a).total_cmp(&distance(point, *b)))
}

/// The `PendingWire` struct is a wire the canvas is dragging out of a node's output, which [`Graph::drop_wire`]
/// connects to the input it's dropped on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PendingWire {
	/// The node whose output the wire leaves.
	pub(crate) from: usize,
	/// The node and row the wire was picked up from, when it was dragged off an input it already drove; dropping it
	/// anywhere else disconnects that input.
	pub(crate) detach: Option<(usize, usize)>,
	/// Where the wire's loose end is, on the canvas.
	pub(crate) end: [f32; 2],
}

/// The `Wire` struct is one connection, from the node whose value is read to the input that reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wire {
	pub(crate) from: usize,
	pub(crate) to: usize,
	/// The row of the reading node the wire ends at.
	pub(crate) row: usize,
}

impl Graph {
	/// Opens the graph whose `.bemx` source text is `text`, laid out for the canvas, or returns why it can't be shown.
	///
	/// A graph that breaks the format's rules still opens when it has a node list, so it can be fixed on the canvas;
	/// saving checks it.
	pub(crate) fn open(id: &str, text: &str) -> Result<Self, String> {
		let document = resource_management::asset::parse_json(text).map_err(|error| {
			format!(
				"Material graph '{id}' couldn't be opened. The most likely cause is a syntax error in the file: {error}. Fix the file's JSON5."
			)
		})?;
		if document.get("nodes").and_then(Json::as_array).is_none() {
			return Err(format!(
				"Material graph '{id}' couldn't be opened. The most likely cause is that it has no `nodes` array. Give the file a `nodes` array."
			));
		}
		let (nodes, wires) = read_nodes(&document);
		let types = resource_management::bemx::json::types(&document);
		let mut graph = Graph {
			id: id.to_string(),
			document,
			types,
			nodes,
			wires,
			offered: Vec::new(),
			unsaved: false,
			shape_revision: 0,
			pending: None,
			notice: None,
			revision: 0,
		};
		graph.lay_out();
		Ok(graph)
	}

	/// The number node `node`'s field `field` holds, written in the file or given by its kind, when it's one number;
	/// dragging across the field starts from it.
	pub(crate) fn number(&self, node: usize, field: &str) -> Option<f64> {
		match self.document["nodes"].get(node)?.get(field) {
			Some(value) => value.as_f64(),
			None => match self.nodes[node]
				.kind?
				.inputs
				.iter()
				.find(|input| input.name == field)?
				.default
			{
				InputDefault::Value([number]) => Some(f64::from(*number)),
				_ => None,
			},
		}
	}

	/// Reads `text`, typed into node `node`'s field `field`, as the value the file writes for it: `None`, which leaves
	/// the field out, for empty text; numbers for an input or a constant setting; and the text itself for a name.
	pub(crate) fn parse(&self, node: usize, field: &str, text: &str) -> Result<Option<Json>, String> {
		let text = text.trim();
		if text.is_empty() {
			return Ok(None);
		}
		let node = &self.nodes[node];
		let setting = node
			.kind
			.and_then(|kind| kind.settings.iter().find(|setting| setting.name == field));
		match setting.map(|setting| setting.kind) {
			Some(SettingKind::Text | SettingKind::Type | SettingKind::Channels) => Ok(Some(Json::from(text))),
			Some(SettingKind::Index) => text.parse::<u64>().map(|index| Some(Json::from(index))).map_err(|_| {
				format!(
					"'{text}' isn't a valid {field} for node '{}'. The most likely cause is a value that isn't a whole number. Type a whole number from 0.",
					node.name
				)
			}),
			Some(SettingKind::Value) | None => numbers(text).map(Some).ok_or_else(|| {
				format!(
					"'{text}' isn't a valid {field} for node '{}'. The most likely cause is a value that isn't one to four numbers. Type one number, or two to four separated by spaces.",
					node.name
				)
			}),
		}
	}

	/// Sets node `node`'s field `field` to `value`, or leaves it out of the file for `None`, then reads the nodes and
	/// wires again and lays them out.
	pub(crate) fn set(&mut self, node: usize, field: &str, value: Option<Json>) {
		let Some(entry) = self.document["nodes"].get_mut(node).and_then(Json::as_object_mut) else {
			return;
		};
		match value {
			Some(value) => entry.insert(field.to_string(), value),
			None => entry.remove(field),
		};
		self.reread();
	}

	/// Whether node `from`'s output can drive input `row` of node `node`: `from` produces a value, the connection makes
	/// no loop, and BEMX doesn't find their types mismatched. A connection whose types BEMX can't judge yet, such as one
	/// into a node that misses another required input, fits, so typing never hides a socket it can't rule out.
	fn fits(&self, from: usize, node: usize, row: usize) -> bool {
		let (source, target) = (&self.nodes[from], &self.nodes[node]);
		source.kind.is_none_or(|kind| kind.produces)
			&& from != node
			&& !self.feeds(node, from)
			&& resource_management::bemx::json::fits(&self.document, &source.name, &target.name, target.rows[row].field())
				!= Some(false)
	}

	/// The input sockets a wire out of node `from` fits, as each one's node and row and its place on the canvas.
	fn sockets(&self, from: usize) -> Vec<((usize, usize), [f32; 2])> {
		self.nodes
			.iter()
			.enumerate()
			.flat_map(|(index, node)| {
				node.rows
					.iter()
					.enumerate()
					.filter(|(_, row)| matches!(row, Row::Input { .. }))
					.map(move |(row, _)| ((index, row), node.input(row)))
			})
			.filter(|((node, row), _)| self.fits(from, *node, *row))
			.collect()
	}

	/// Starts dragging `wire`, and lists the input sockets it fits, which its knob heads for and a drop plugs into.
	pub(crate) fn grab(&mut self, wire: PendingWire) {
		self.offered = self.sockets(wire.from);
		self.pending = Some(wire);
	}

	/// Where the socket the dragged wire fits that lies nearest `point` is, or `None` when it fits none. Its knob heads
	/// for it, and a drop within [`PLUG_REACH`] of it plugs into it, so the knob always shows where a drop lands.
	pub(crate) fn nearest_offered(&self, point: [f32; 2]) -> Option<[f32; 2]> {
		nearest(&self.offered, point).map(|(_, socket)| socket)
	}

	/// Drops `wire`: connects it to the input it fits nearest its end, when that input lies within [`PLUG_REACH`], and
	/// disconnects the input it was picked up from when it lands anywhere else. Only inputs that [`Self::fits`] admits
	/// are offered, so a drop never makes a loop or reads the surface.
	pub(crate) fn drop_wire(&mut self, wire: PendingWire) {
		// Fields are looked up by name, since disconnecting can change a node's rows.
		let field = |graph: &Self, (node, row): (usize, usize)| (node, graph.nodes[node].rows[row].field().to_string());
		self.offered.clear();
		let target = nearest(&self.sockets(wire.from), wire.end)
			.filter(|(_, socket)| distance(wire.end, *socket) <= PLUG_REACH)
			.map(|(target, _)| field(self, target));
		if let Some(detach) = wire.detach.map(|detach| field(self, detach)) {
			if target.as_ref() == Some(&detach) {
				return;
			}
			self.set(detach.0, &detach.1, None);
		}
		if let Some((node, input)) = target {
			let source = Json::from(self.nodes[wire.from].name.as_str());
			self.set(node, &input, Some(source));
		}
	}

	/// Adds a node of `kind`, named after it and numbered when the name is taken, such as `multiply2`, and returns it.
	pub(crate) fn add(&mut self, kind: &NodeKind) -> usize {
		let taken = |name: &str| self.nodes.iter().any(|node| node.name == name);
		let name = (1..)
			.map(|number| match number {
				1 => kind.name.to_string(),
				number => format!("{}{number}", kind.name),
			})
			.find(|name| !taken(name))
			.expect("Some numbered name is free, since a graph has finitely many nodes.");
		let entry = serde_json::json!({ "name": name, "node": kind.name });
		if let Some(entries) = self.document["nodes"].as_array_mut() {
			entries.push(entry);
		}
		self.reread();
		self.nodes.len() - 1
	}

	/// Removes node `node`, and every reference to it: the inputs it drove return to their defaults.
	pub(crate) fn remove(&mut self, node: usize) {
		let name = Json::from(self.nodes[node].name.as_str());
		// Only inputs read nodes, so a setting that happens to hold the same text stays.
		for (index, graph_node) in self.nodes.iter().enumerate() {
			let Some(entry) = self.document["nodes"].get_mut(index).and_then(Json::as_object_mut) else {
				continue;
			};
			for row in &graph_node.rows {
				if matches!(row, Row::Input { .. }) && entry.get(row.field()) == Some(&name) {
					entry.remove(row.field());
				}
			}
		}
		if self.document.get("surface") == Some(&name)
			&& let Some(root) = self.document.as_object_mut()
		{
			root.remove("surface");
		}
		if let Some(entries) = self.document["nodes"].as_array_mut() {
			entries.remove(node);
		}
		self.reread();
	}

	/// Reads the nodes and wires again after the document changed, lays them out, and marks the graph unsaved;
	/// advances the shape revision when the canvas must declare them again.
	fn reread(&mut self) {
		let (nodes, wires) = read_nodes(&self.document);
		let types = resource_management::bemx::json::types(&self.document);
		// A node's type colors its wires, so a change of type redraws them as a change of shape does.
		let reshaped = wires != self.wires
			|| types != self.types
			|| nodes.len() != self.nodes.len()
			|| nodes
				.iter()
				.zip(&self.nodes)
				.any(|(new, old)| new.rows.len() != old.rows.len());
		(self.nodes, self.wires, self.types) = (nodes, wires, types);
		self.lay_out();
		if reshaped {
			self.shape_revision += 1;
		}
		self.unsaved = true;
		self.notice = None;
		self.revision += 1;
	}

	/// Whether node `upstream`'s value reaches node `downstream` through wires.
	fn feeds(&self, upstream: usize, downstream: usize) -> bool {
		// A depth-first walk along the wires, visiting each node once.
		let mut seen = vec![false; self.nodes.len()];
		let mut stack = vec![upstream];
		while let Some(node) = stack.pop() {
			if node == downstream {
				return true;
			}
			if !std::mem::replace(&mut seen[node], true) {
				stack.extend(self.wires.iter().filter(|wire| wire.from == node).map(|wire| wire.to));
			}
		}
		false
	}

	/// Shows `error`, why an edit or a save failed, until the next edit or save succeeds.
	pub(crate) fn fail(&mut self, error: String) {
		self.notice = Some(error);
		self.revision += 1;
	}

	/// Marks the document as saved.
	pub(crate) fn saved(&mut self) {
		self.unsaved = false;
		self.notice = None;
		self.revision += 1;
	}

	/// The document as `.bemx` source text to save, or why it breaks the format's rules, as the engine's reader
	/// finds them.
	pub(crate) fn save_text(&self) -> Result<String, String> {
		resource_management::bemx::json::read(&self.document).map_err(|error| error.to_string())?;
		Ok(resource_management::bemx::json::write(&self.document))
	}

	/// Places the nodes in columns by their distance from the nodes nothing reads, such as the surface, which sit in
	/// the rightmost column; each column stacks its nodes in file order.
	fn lay_out(&mut self) {
		// A node's depth is one more than the deepest node that reads it. The file may hold a cycle, which the reader
		// rejects, so the depth stops growing once it passes the node count.
		let count = self.nodes.len();
		let mut depth = vec![0usize; count];
		for _ in 0..count {
			let mut changed = false;
			for wire in &self.wires {
				let deeper = depth[wire.to] + 1;
				if deeper > depth[wire.from] && deeper < count {
					depth[wire.from] = deeper;
					changed = true;
				}
			}
			if !changed {
				break;
			}
		}
		let deepest = depth.iter().copied().max().unwrap_or(0);
		let mut column_heights = vec![0.0f32; deepest + 1];
		for (node, depth) in self.nodes.iter_mut().zip(depth) {
			let column = deepest - depth;
			node.position = [column as f32 * (NODE_WIDTH + COLUMN_GAP), column_heights[column]];
			column_heights[column] += node.height() + NODE_GAP;
		}
	}

	/// Where the wire `wire` starts, at its source's output, and ends, at its row's input, on the canvas.
	pub(crate) fn wire_ends(&self, wire: &Wire) -> ([f32; 2], [f32; 2]) {
		(self.nodes[wire.from].output(), self.nodes[wire.to].input(wire.row))
	}

	/// The rectangle every node lies in: left, top, right, bottom.
	pub(crate) fn bounds(&self) -> [f32; 4] {
		self.nodes
			.iter()
			.fold([0.0, 0.0, 0.0, 0.0], |[left, top, right, bottom], node| {
				let [x, y] = node.position;
				[
					left.min(x),
					top.min(y),
					right.max(x + NODE_WIDTH),
					bottom.max(y + node.height()),
				]
			})
	}

	/// The source asset's file name, such as `floor.bemx`.
	pub(crate) fn file_name(&self) -> &str {
		self.id.rsplit('/').next().unwrap_or(&self.id)
	}
}

impl GraphNode {
	/// The node's height on the canvas: its header and its rows.
	pub(crate) fn height(&self) -> f32 {
		HEADER_HEIGHT + self.rows.len() as f32 * ROW_HEIGHT + FOOTER_HEIGHT
	}

	/// Where the node's output socket sits, on its header's right edge; every node but the surface has one.
	pub(crate) fn output(&self) -> [f32; 2] {
		[self.position[0] + NODE_WIDTH, self.position[1] + HEADER_HEIGHT * 0.5]
	}

	/// Where the input socket of row `row` sits, on the node's left edge.
	pub(crate) fn input(&self, row: usize) -> [f32; 2] {
		[
			self.position[0],
			self.position[1] + HEADER_HEIGHT + (row as f32 + 0.5) * ROW_HEIGHT,
		]
	}

	/// The color the node's header is tinted with: the surface's is the accent, and unknown kinds are red.
	pub(crate) fn tint(&self) -> Option<RGBA> {
		match self.kind {
			None => Some(RGBA::new(0.95, 0.33, 0.36, 1.0)),
			Some(kind) if !kind.produces => Some(RGBA::new(0.369, 0.416, 0.824, 1.0)),
			Some(_) => None,
		}
	}
}

/// Reads the nodes of `document`, a document with a `nodes` array, and the wires between them, every node at the
/// canvas's origin until the graph is laid out.
fn read_nodes(document: &Json) -> (Vec<GraphNode>, Vec<Wire>) {
	let entries = document["nodes"].as_array().map_or(&[][..], Vec::as_slice);
	let named = |name: &str| {
		entries
			.iter()
			.position(|entry| entry.get("name").and_then(Json::as_str) == Some(name))
	};
	let mut nodes = Vec::with_capacity(entries.len());
	let mut wires = Vec::new();
	for (index, entry) in entries.iter().enumerate() {
		let fields = entry.as_object();
		let field = |key: &str| fields.and_then(|fields| fields.get(key));
		let text = |key: &str| field(key).and_then(Json::as_str).unwrap_or_default().to_string();
		let kind_name = text("node");
		let kind = kinds::kind(&kind_name);
		let mut rows = Vec::new();
		for input in kind.map_or(&[][..], |kind| kind.inputs) {
			let source = match field(input.name) {
				Some(Json::String(reference)) => match named(reference) {
					Some(from) => {
						wires.push(Wire {
							from,
							to: index,
							row: rows.len(),
						});
						Source::Node
					}
					None => Source::Unset,
				},
				Some(value) => Source::Constant(value_text(value)),
				None => match input.default {
					InputDefault::Value(default) => Source::Default(numbers_text(default)),
					InputDefault::Required | InputDefault::Unset => Source::Unset,
				},
			};
			// Numbered inputs read until the first one left out, so only the first unused one shows.
			let numbered = input.name.starts_with("in") && input.name[2..].parse::<u32>().is_ok_and(|number| number > 1);
			let previous_unused = matches!(
				rows.last(),
				Some(Row::Input {
					source: Source::Unset,
					..
				})
			);
			if numbered && source == Source::Unset && previous_unused {
				continue;
			}
			rows.push(Row::Input {
				name: input.name.to_string(),
				source,
			});
		}
		// Every setting shows, so one the file leaves out can be given a value.
		for setting in kind.map_or(&[][..], |kind| kind.settings) {
			rows.push(Row::Setting {
				name: setting.name.to_string(),
				value: field(setting.name).map(value_text),
			});
		}
		nodes.push(GraphNode {
			name: text("name"),
			kind_name,
			kind,
			rows,
			position: [0.0, 0.0],
		});
	}
	(nodes, wires)
}

/// A value as the canvas shows it: a number or an array of numbers with two decimals at most, or a string as is.
fn value_text(value: &Json) -> String {
	match value {
		Json::String(text) => text.clone(),
		Json::Number(number) => number
			.as_f64()
			.map_or_else(|| number.to_string(), |number| number_text(number as f32)),
		Json::Array(items) => {
			let numbers = items
				.iter()
				.filter_map(Json::as_f64)
				.map(|number| number as f32)
				.collect::<Vec<_>>();
			numbers_text(&numbers)
		}
		other => other.to_string(),
	}
}

/// Reads text such as `0.5` or `1 0.9 0.8` as the constant a file writes: one number, or an array of two to four.
fn numbers(text: &str) -> Option<Json> {
	let mut values = Vec::with_capacity(4);
	for part in text
		.split(|c: char| c.is_whitespace() || c == ',')
		.filter(|part| !part.is_empty())
	{
		values.push(Json::from(part.parse::<f64>().ok().filter(|number| number.is_finite())?));
	}
	match values.len() {
		1 => values.pop(),
		2..=4 => Some(Json::Array(values)),
		_ => None,
	}
}

fn numbers_text(numbers: &[f32]) -> String {
	match numbers {
		[number] => number_text(*number),
		numbers => numbers
			.iter()
			.map(|number| number_text(*number))
			.collect::<Vec<_>>()
			.join(" "),
	}
}

/// A number with two decimals at most and no trailing zeros, such as `0.5` or `1`.
fn number_text(number: f32) -> String {
	let text = format!("{number:.2}");
	let text = text.trim_end_matches('0').trim_end_matches('.');
	if text == "-0" { "0".to_string() } else { text.to_string() }
}

#[cfg(test)]
mod tests {
	use super::*;

	const PBR: &str = r#"{
		nodes: [
			{ name: "base_color", node: "image", texture: "color", type: "color3" },
			{ name: "encoded_normal", node: "image", texture: "normal", type: "vector3" },
			{ name: "normal", node: "normalmap", in: "encoded_normal" },
			{ name: "tinted", node: "multiply", in1: "base_color", in2: [1.0, 0.9, 0.8] },
			{ name: "surface", node: "surface", base_color: "tinted", normal: "normal", roughness: 0.5 },
		],
	}"#;

	fn node<'g>(graph: &'g Graph, name: &str) -> &'g GraphNode {
		graph.nodes.iter().find(|node| node.name == name).unwrap()
	}

	#[test]
	fn nodes_lie_in_columns_by_their_distance_from_the_surface() {
		let graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let column = |name: &str| node(&graph, name).position[0] / (NODE_WIDTH + COLUMN_GAP);
		assert_eq!(column("surface"), 2.0);
		assert_eq!(column("tinted"), 1.0);
		assert_eq!(column("normal"), 1.0);
		assert_eq!(column("base_color"), 0.0);
		assert_eq!(column("encoded_normal"), 0.0);
		// A column stacks its nodes in file order without overlapping.
		let (first, second) = (node(&graph, "base_color"), node(&graph, "encoded_normal"));
		assert!(second.position[1] >= first.position[1] + first.height());
	}

	#[test]
	fn a_grabbed_wire_is_offered_only_the_inputs_it_fits() {
		let mut graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let index = |graph: &Graph, name: &str| graph.nodes.iter().position(|node| node.name == name).unwrap();
		let (tinted, surface) = (index(&graph, "tinted"), index(&graph, "surface"));
		let row = |graph: &Graph, field: &str| graph.nodes[surface].rows.iter().position(|row| row.field() == field).unwrap();
		let (normal, roughness) = (row(&graph, "normal"), row(&graph, "roughness"));
		assert_eq!(graph.types[tinted], Some(Type::Vector3));
		graph.grab(PendingWire {
			from: tinted,
			detach: None,
			end: graph.nodes[tinted].output(),
		});
		let roughness_socket = graph.nodes[surface].input(roughness);
		// Right on the roughness socket, a float, a color's knob heads for the normal's below it, which takes a color,
		// past the metalness's above it, another float.
		assert_eq!(
			graph.nearest_offered(roughness_socket),
			Some(graph.nodes[surface].input(normal))
		);
		// Its own inputs, and the inputs of nodes it reads, would make loops.
		let base_color = index(&graph, "base_color");
		assert!(
			graph
				.offered
				.iter()
				.all(|((node, _), _)| *node != tinted && *node != base_color)
		);
		// Dropped right on the roughness socket, out of reach of the normal's, it plugs into nothing.
		let wire = graph.pending.take().unwrap();
		graph.drop_wire(PendingWire {
			end: roughness_socket,
			..wire
		});
		assert!(!graph.wires.iter().any(|wire| wire.to == surface && wire.row == roughness));
	}

	#[test]
	fn nodes_sit_on_whole_grid_cells() {
		let graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let on_grid = |value: f32| (value / GRID).fract() == 0.0;
		for node in &graph.nodes {
			let [x, y] = node.position;
			assert!(on_grid(x) && on_grid(y), "{} sits at {x}, {y}", node.name);
			assert!(on_grid(node.height()), "{} is {} tall", node.name, node.height());
		}
		assert!(on_grid(NODE_WIDTH));
	}

	#[test]
	fn inputs_show_what_drives_them_and_wires_join_outputs_to_their_rows() {
		let graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let tinted = node(&graph, "tinted");
		assert_eq!(
			tinted.rows,
			[
				Row::Input {
					name: "in1".into(),
					source: Source::Node
				},
				Row::Input {
					name: "in2".into(),
					source: Source::Constant("1 0.9 0.8".into())
				},
			]
		);
		let surface = node(&graph, "surface");
		assert_eq!(surface.rows.len(), 11, "the surface shows every input it can read");
		assert!(surface.rows.contains(&Row::Input {
			name: "roughness".into(),
			source: Source::Constant("0.5".into())
		}));
		let image = node(&graph, "base_color");
		assert!(image.rows.contains(&Row::Setting {
			name: "texture".into(),
			value: Some("color".into())
		}));

		let to_tinted = graph.wires.iter().find(|wire| graph.nodes[wire.to].name == "tinted").unwrap();
		assert_eq!(graph.nodes[to_tinted.from].name, "base_color");
		let (start, end) = graph.wire_ends(to_tinted);
		assert_eq!(start, node(&graph, "base_color").output());
		assert_eq!(end, tinted.input(0));
	}

	#[test]
	fn a_numbered_input_list_shows_one_unused_input_and_defaults_show_their_value() {
		let graph = Graph::open(
			"a.bemx",
			r#"{ nodes: [
				{ name: "pick", node: "switch", in1: 0.2, in2: 0.4 },
				{ name: "surface", node: "surface", roughness: "pick" },
			] }"#,
		)
		.unwrap();
		let names = node(&graph, "pick").rows.iter().map(Row::field).collect::<Vec<_>>();
		assert_eq!(names, ["which", "in1", "in2", "in3"]);
		assert_eq!(
			node(&graph, "pick").rows[0],
			Row::Input {
				name: "which".into(),
				source: Source::Default("0".into())
			}
		);
	}

	#[test]
	fn typed_values_change_the_file_and_saving_writes_text_the_reader_accepts() {
		let mut graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let surface = graph.nodes.iter().position(|node| node.name == "surface").unwrap();
		let tinted = graph.nodes.iter().position(|node| node.name == "tinted").unwrap();
		let image = graph.nodes.iter().position(|node| node.name == "base_color").unwrap();

		let roughness = graph.parse(surface, "roughness", "0.25").unwrap();
		graph.set(surface, "roughness", roughness);
		let tint = graph.parse(tinted, "in2", "1, 0.5 0").unwrap();
		graph.set(tinted, "in2", tint);
		let texture = graph.parse(image, "texture", "albedo").unwrap();
		graph.set(image, "texture", texture);
		assert!(graph.unsaved);
		assert_eq!(graph.number(surface, "roughness"), Some(0.25));
		assert!(graph.nodes[tinted].rows.contains(&Row::Input {
			name: "in2".into(),
			source: Source::Constant("1 0.5 0".into())
		}));

		let text = graph.save_text().unwrap();
		let saved = Graph::open("materials/pbr.bemx", &text).unwrap();
		assert_eq!(saved.number(surface, "roughness"), Some(0.25));
		assert_eq!(saved.nodes[tinted].rows, graph.nodes[tinted].rows);
		assert_eq!(saved.nodes[image].rows, graph.nodes[image].rows);
		assert_eq!(saved.wires, graph.wires);
	}

	#[test]
	fn clearing_a_field_returns_it_to_its_default_and_bad_text_is_refused() {
		let mut graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let tinted = graph.nodes.iter().position(|node| node.name == "tinted").unwrap();
		graph.set(tinted, "in2", None);
		assert_eq!(graph.number(tinted, "in2"), Some(1.0), "multiply's in2 defaults to 1");
		assert!(graph.parse(tinted, "in2", "bright").is_err());
		assert!(graph.parse(tinted, "in2", "1 2 3 4 5").is_err());
		assert_eq!(graph.parse(tinted, "in2", "  ").unwrap(), None);
	}

	#[test]
	fn a_graph_that_breaks_the_rules_is_not_saved() {
		let mut graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let surface = graph.nodes.iter().position(|node| node.name == "surface").unwrap();
		// A setting the kind reads, but with text where the reader needs a number.
		graph.set(surface, "roughness", Some(Json::from("missing")));
		assert!(graph.save_text().is_err());
	}

	fn position(graph: &Graph, name: &str) -> usize {
		graph.nodes.iter().position(|node| node.name == name).unwrap()
	}

	#[test]
	fn a_wire_dropped_on_an_input_connects_it_and_one_dropped_elsewhere_disconnects() {
		let mut graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let (base_color, normal, surface) = (
			position(&graph, "base_color"),
			position(&graph, "normal"),
			position(&graph, "surface"),
		);
		let emission = graph.nodes[surface]
			.rows
			.iter()
			.position(|row| row.field() == "emission")
			.unwrap();
		let wire = PendingWire {
			from: base_color,
			detach: None,
			end: graph.nodes[surface].input(emission),
		};
		graph.drop_wire(wire);
		assert!(graph.wires.contains(&Wire {
			from: base_color,
			to: surface,
			row: emission
		}));

		// Picked up off the surface's normal and dropped on empty canvas, the normal's wire is gone.
		let normal_row = graph.nodes[surface]
			.rows
			.iter()
			.position(|row| row.field() == "normal")
			.unwrap();
		let wire = PendingWire {
			from: normal,
			detach: Some((surface, normal_row)),
			end: [-1000.0, -1000.0],
		};
		graph.drop_wire(wire);
		assert!(!graph.wires.iter().any(|wire| wire.from == normal));
		let saved = Graph::open("a.bemx", &graph.save_text().unwrap()).unwrap();
		assert_eq!(saved.wires, graph.wires);
	}

	#[test]
	fn added_nodes_take_free_names_and_removed_ones_leave_no_references() {
		let mut graph = Graph::open("materials/pbr.bemx", PBR).unwrap();
		let multiply = kinds::kind("multiply").unwrap();
		let added = graph.add(multiply);
		assert_eq!(graph.nodes[added].name, "multiply");
		let second = graph.add(multiply);
		assert_eq!(graph.nodes[second].name, "multiply2");

		graph.remove(position(&graph, "tinted"));
		assert!(graph.nodes.iter().all(|node| node.name != "tinted"));
		let surface = position(&graph, "surface");
		assert!(graph.nodes[surface].rows.contains(&Row::Input {
			name: "base_color".into(),
			source: Source::Unset
		}));
		// The new nodes miss their required inputs, so the reader refuses the graph until they're removed.
		assert!(graph.save_text().is_err());
		graph.remove(position(&graph, "multiply"));
		graph.remove(position(&graph, "multiply2"));
		graph.save_text().unwrap();
	}

	#[test]
	fn a_file_without_a_node_list_does_not_open() {
		assert!(Graph::open("a.bemx", "{ nodes: [ }").is_err());
		assert!(Graph::open("a.bemx", "{ surface: \"x\" }").is_err());
	}
}
