//! The editor's modes as a stack of layers, from the scene at the bottom to whatever was opened last on top.
//!
//! The top layer takes the keyboard and the pointer, and Escape pops it: a setting's value back to the palette that
//! found it, the palette back to what it opened over, and the scene, the last layer, closes the editor. A layer reads
//! the layers under it to decide how it behaves, as the palette offers node kinds when it opens over a material graph.
//!
//! This module holds the stack and what each layer is. The editor pushes and pops layers as it answers input, and the
//! shell draws each one from the stack.

use crate::editor::carry::Carry;
use crate::editor::graph::Graph;
use crate::editor::palette;
use crate::editor::variant::Variant;

/// The `Layer` enum is one mode the editor stacks over the game: what it shows, and what takes input while it's on
/// top.
pub(crate) enum Layer {
	/// The panels, the gizmo, and the selection: the bottom layer, which the editor opens on.
	Scene,
	/// A material graph on its canvas, at the bottom of the layers of its own window.
	Graph(Graph),
	/// The command palette, searching what its scope finds.
	Palette(palette::State),
	/// A runtime setting's value being typed, over the palette that found the setting.
	Setting(palette::Setting),
	/// A texture shown in a preview panel, at the bottom of the layers of its own window.
	Texture(Texture),
	/// A material variant's parameters, at the bottom of the layers of its own window.
	Variant(Variant),
	/// A mesh, prefab, or light following the pointer until a click places it.
	Carry(Carry),
	/// Text being typed into a property field, which answers Enter and Escape itself.
	Typing,
}

/// The `Texture` struct is a texture the editor shows in its preview panel, decoded from its source image.
pub(crate) struct Texture {
	/// The source image's asset ID, such as `textures/brick.png`.
	pub(crate) id: String,
	/// The picture the panel shows, scaled down to fit it.
	pub(crate) preview: resource_management::processors::Preview,
	/// What the texture bakes with, which its properties panel shows and changes; `None` when it couldn't be read.
	pub(crate) settings: Option<resource_management::asset::ResolvedImageSettings>,
}

/// The `Layers` struct is the editor's stack of layers; empty while the editor is closed.
#[derive(Default)]
pub(crate) struct Layers {
	stack: Vec<Layer>,
	/// Advanced on every push and pop, so the shell's views know to show what changed.
	pub(crate) revision: u64,
}

impl Layers {
	/// Whether the editor is open, which it is while any layer is.
	pub(crate) fn open(&self) -> bool {
		!self.stack.is_empty()
	}

	/// The layer that takes input.
	pub(crate) fn top(&self) -> Option<&Layer> {
		self.stack.last()
	}

	pub(crate) fn top_mut(&mut self) -> Option<&mut Layer> {
		self.stack.last_mut()
	}

	pub(crate) fn push(&mut self, layer: Layer) {
		self.stack.push(layer);
		self.revision += 1;
	}

	/// Removes the top layer and returns it, so the caller can undo what it left in the game.
	pub(crate) fn pop(&mut self) -> Option<Layer> {
		self.revision += 1;
		self.stack.pop()
	}

	/// Removes the topmost layer that `is` matches and every layer over it, as closing a graph's canvas closes the
	/// palette open over it.
	pub(crate) fn close(&mut self, is: impl Fn(&Layer) -> bool) {
		if let Some(index) = self.stack.iter().rposition(is) {
			self.stack.truncate(index);
			self.revision += 1;
		}
	}

	/// The topmost palette, which shows while it's open, even under a setting being typed.
	pub(crate) fn palette(&self) -> Option<&palette::State> {
		self.stack.iter().rev().find_map(|layer| match layer {
			Layer::Palette(palette) => Some(palette),
			_ => None,
		})
	}

	pub(crate) fn palette_mut(&mut self) -> Option<&mut palette::State> {
		self.stack.iter_mut().rev().find_map(|layer| match layer {
			Layer::Palette(palette) => Some(palette),
			_ => None,
		})
	}

	/// The setting being typed, while it's the top layer.
	pub(crate) fn setting(&self) -> Option<&palette::Setting> {
		match self.stack.last()? {
			Layer::Setting(setting) => Some(setting),
			_ => None,
		}
	}

	pub(crate) fn setting_mut(&mut self) -> Option<&mut palette::Setting> {
		match self.stack.last_mut()? {
			Layer::Setting(setting) => Some(setting),
			_ => None,
		}
	}

	/// The topmost material graph, which its canvas shows while palettes open over it.
	pub(crate) fn graph(&self) -> Option<&Graph> {
		self.stack.iter().rev().find_map(|layer| match layer {
			Layer::Graph(graph) => Some(graph),
			_ => None,
		})
	}

	pub(crate) fn graph_mut(&mut self) -> Option<&mut Graph> {
		self.stack.iter_mut().rev().find_map(|layer| match layer {
			Layer::Graph(graph) => Some(graph),
			_ => None,
		})
	}

	/// The topmost texture being previewed, which its panel shows while palettes open over it.
	pub(crate) fn texture(&self) -> Option<&Texture> {
		self.stack.iter().rev().find_map(|layer| match layer {
			Layer::Texture(texture) => Some(texture),
			_ => None,
		})
	}

	pub(crate) fn texture_mut(&mut self) -> Option<&mut Texture> {
		self.stack.iter_mut().rev().find_map(|layer| match layer {
			Layer::Texture(texture) => Some(texture),
			_ => None,
		})
	}

	/// The variant whose parameters show, which its window shows while palettes open over it.
	pub(crate) fn variant(&self) -> Option<&Variant> {
		self.stack.iter().rev().find_map(|layer| match layer {
			Layer::Variant(variant) => Some(variant),
			_ => None,
		})
	}

	pub(crate) fn variant_mut(&mut self) -> Option<&mut Variant> {
		self.stack.iter_mut().rev().find_map(|layer| match layer {
			Layer::Variant(variant) => Some(variant),
			_ => None,
		})
	}

	/// The carried load, while it's the top layer.
	pub(crate) fn carry_mut(&mut self) -> Option<&mut Carry> {
		match self.stack.last_mut()? {
			Layer::Carry(carry) => Some(carry),
			_ => None,
		}
	}

	/// The scope a palette asked for with `asked` opens on, by what it opens over: the node kinds over a material
	/// graph, and what was asked for otherwise.
	pub(crate) fn palette_scope(&self, asked: palette::Scope) -> palette::Scope {
		// A palette opens over the layers under any palette already open, which it replaces.
		let under = self
			.stack
			.iter()
			.rev()
			.find(|layer| !matches!(layer, Layer::Palette(_) | Layer::Setting(_)));
		match under {
			Some(Layer::Graph(_)) => palette::Scope::Nodes,
			_ => asked,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A material graph with one surface node, for a canvas under other layers.
	fn graph() -> Graph {
		Graph::open("a.bemx", r#"{ nodes: [{ name: "surface", node: "surface" }] }"#).unwrap()
	}

	#[test]
	fn escape_pops_back_down_to_a_closed_editor() {
		let mut layers = Layers::default();
		layers.push(Layer::Scene);
		layers.push(Layer::Palette(palette::State::default()));
		assert!(matches!(layers.top(), Some(Layer::Palette(_))));

		layers.pop();
		assert!(matches!(layers.top(), Some(Layer::Scene)));
		layers.pop();
		assert!(!layers.open());
	}

	#[test]
	fn a_palette_offers_node_kinds_over_a_graph_and_what_was_asked_over_the_scene() {
		let mut layers = Layers::default();
		layers.push(Layer::Scene);
		assert_eq!(layers.palette_scope(palette::Scope::Place), palette::Scope::Place);

		layers.push(Layer::Graph(graph()));
		layers.push(Layer::Palette(palette::State::default()));
		assert_eq!(layers.palette_scope(palette::Scope::Place), palette::Scope::Nodes);
	}

	#[test]
	fn closing_a_layer_closes_the_layers_over_it() {
		let mut layers = Layers::default();
		layers.push(Layer::Scene);
		layers.push(Layer::Graph(graph()));
		layers.push(Layer::Palette(palette::State::default()));
		layers.close(|layer| matches!(layer, Layer::Graph(_)));
		assert!(matches!(layers.top(), Some(Layer::Scene)));
	}
}
