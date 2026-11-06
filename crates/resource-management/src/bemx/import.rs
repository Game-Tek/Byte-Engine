//! Holds the materials glTF and FBX importers build, and the texture bookkeeping they share.

use std::fmt::{Display, Formatter};

use super::graph::{BinaryOperator, Channel, Graph, GraphBuilder, GraphError, Node, NodeId, Surface, Type, Value};
use super::{material_texture_parameter_name, material_texture_slot};
use crate::processors::image::{ChannelPacking, METALLIC_ROUGHNESS_PACKING, Semantic};
use crate::types::AlphaMode;

/// The `ImportedMaterial` struct holds one material a container importer read: its graph, the container images its
/// texture parameters bind, and the material properties that are not part of the graph.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImportedMaterial {
	pub name: Option<String>,
	pub graph: Graph,
	/// The container image each texture parameter binds, in slot order. Slot `n` is the parameter
	/// [`material_texture_parameter_name`]`(n)`, which is how the graph's image nodes name it.
	pub images: Vec<u32>,
	pub double_sided: bool,
	pub alpha_mode: AlphaMode,
}

impl ImportedMaterial {
	/// Returns the container image the texture parameter `texture` binds.
	pub fn image(&self, texture: &str) -> Option<u32> {
		image_of(&self.images, texture)
	}

	/// Moves channel reads of packed images to where the image processor stores them.
	///
	/// Importers call this after building a material. `packing_for` receives a container image index and returns the
	/// [`ChannelPacking`] that image bakes with, or `None` when it keeps all of its channels. A read of a channel the
	/// packing drops is an error, because the shader would sample data that is not there.
	pub fn pack_texture_channels(
		&mut self,
		mut packing_for: impl FnMut(u32) -> Option<ChannelPacking>,
	) -> Result<(), PackingError> {
		let images = &self.images;

		self.graph.pack_texture_channels(|texture, channel| {
			let Some(packing) = image_of(images, texture).and_then(&mut packing_for) else {
				return Ok(None);
			};

			packing
				.stored_channel(usize::from(channel))
				.map(|stored| Some(stored as u8))
				.ok_or_else(|| PackingError {
					texture: texture.to_string(),
					channel,
				})
		})
	}

	/// Merges the semantic each container image is sampled with into `semantics`, which is indexed by container image.
	///
	/// The semantic comes from the surface input that reads the image, never from the image's name, so every importer
	/// bakes a texture the way its material uses it. Importers call this for every material of a container, then bake
	/// each image with its merged semantic. Images outside `semantics` are ignored.
	pub fn merge_image_semantics(&self, semantics: &mut [Option<Semantic>]) {
		let surface = self.graph.surface();

		for (node, semantic) in [
			(surface.base_color, Semantic::Albedo),
			// A base color texture's alpha is also read as opacity, and its albedo read outranks that one.
			(surface.opacity, Semantic::Opacity),
			(surface.metalness, Semantic::Metallic),
			(surface.roughness, Semantic::Roughness),
			(surface.normal, Semantic::Normal),
			(surface.occlusion, Semantic::AO),
			(surface.emission, Semantic::Emissive),
		] {
			if let Some(node) = node {
				self.merge_node_image_semantics(node, semantic, semantics);
			}
		}
	}

	/// Merges the semantic every image `node` reaches is sampled with, reading it as `semantic` unless the path to the
	/// image says otherwise.
	fn merge_node_image_semantics(&self, node: NodeId, semantic: Semantic, semantics: &mut [Option<Semantic>]) {
		let graph = &self.graph;

		match graph.node(node) {
			Node::Image { texture, .. } => {
				if let Some(merged) = self.image(texture).and_then(|image| semantics.get_mut(image as usize)) {
					*merged = Some(merged.map_or(semantic, |merged| merge_semantics(merged, semantic)));
				}
			}
			Node::Extract { value, index } => {
				// A metallic or roughness read of a channel the packing keeps lets the image bake as a two-channel map. Any
				// other read of the image outranks it in `merge_semantics`, so only fully packable images pack.
				let packable = matches!(semantic, Semantic::Metallic | Semantic::Roughness)
					&& METALLIC_ROUGHNESS_PACKING.stored_channel(usize::from(*index)).is_some()
					&& matches!(graph.node(*value), Node::Image { .. });
				let semantic = if packable { Semantic::MetallicRoughness } else { semantic };

				self.merge_node_image_semantics(*value, semantic, semantics);
			}
			Node::NormalMap { value, scale } => {
				self.merge_node_image_semantics(*value, Semantic::Normal, semantics);
				self.merge_node_image_semantics(*scale, semantic, semantics);
			}
			other => other.visit_inputs(|input| self.merge_node_image_semantics(input, semantic, semantics)),
		}
	}
}

/// Picks the semantic an image bakes with when materials sample it in more than one way.
///
/// The pick is a fixed priority, so the order in which reads merge does not change the result.
fn merge_semantics(left: Semantic, right: Semantic) -> Semantic {
	// Prefer color semantics when an unusual asset reuses the same image for color and data textures.
	// This avoids accidentally sampling an albedo texture as linear data after processing.
	match (left, right) {
		(Semantic::Albedo, _) | (_, Semantic::Albedo) => Semantic::Albedo,
		(Semantic::Emissive, _) | (_, Semantic::Emissive) => Semantic::Emissive,
		(Semantic::Opacity, _) | (_, Semantic::Opacity) => Semantic::Opacity,
		(Semantic::Normal, _) | (_, Semantic::Normal) => Semantic::Normal,
		(Semantic::AO, _) | (_, Semantic::AO) => Semantic::AO,
		(Semantic::Metallic, _) | (_, Semantic::Metallic) => Semantic::Metallic,
		(Semantic::Roughness, _) | (_, Semantic::Roughness) => Semantic::Roughness,
		// A packed map drops channels, so any other use of the image keeps all of them.
		(Semantic::MetallicRoughness, other) | (other, Semantic::MetallicRoughness) => other,
		_ => left,
	}
}

/// Returns the container image the texture parameter `texture` binds, given the image of each slot.
fn image_of(images: &[u32], texture: &str) -> Option<u32> {
	images.get(material_texture_slot(texture)?).copied()
}

/// The `ImportedGraphBuilder` struct builds an imported material's graph while numbering the images it samples.
///
/// Images are numbered in the order the graph first samples them, so two materials that differ only in their images
/// build identical graphs.
#[derive(Default)]
pub(crate) struct ImportedGraphBuilder {
	pub builder: GraphBuilder,
	images: Vec<u32>,
}

impl ImportedGraphBuilder {
	pub fn new() -> Self {
		Self::default()
	}

	/// Adds a sample of container image `image`, keeping `type`'s worth of its channels.
	pub fn image(&mut self, image: u32, r#type: Type) -> Result<NodeId, GraphError> {
		let slot = self.images.iter().position(|used| *used == image).unwrap_or_else(|| {
			self.images.push(image);
			self.images.len() - 1
		});

		self.builder.add(Node::Image {
			texture: material_texture_parameter_name(slot),
			r#type,
			texcoord: None,
		})
	}

	/// Adds a constant factor, multiplied by a sample of container image `image` when the material has one.
	///
	/// The sample keeps as many channels as the factor has, which is how glTF and FBX scale a colour by its texture.
	pub fn factor_times_image(&mut self, factor: Value, image: Option<u32>) -> Result<NodeId, GraphError> {
		let r#type = factor.r#type();
		let factor = self.builder.constant(factor);

		let Some(image) = image else {
			return Ok(factor);
		};

		let texture = self.image(image, r#type)?;

		self.builder.add(Node::Binary {
			operator: BinaryOperator::Multiply,
			left: factor,
			right: texture,
		})
	}

	/// Adds a scalar factor, multiplied by channel `channel` of `texture` when the material has one.
	///
	/// glTF reads metalness and roughness from two channels of one texture; FBX reads each from the red channel of its own.
	pub fn factor_times_channel(&mut self, factor: f32, texture: Option<NodeId>, channel: u8) -> Result<NodeId, GraphError> {
		let factor = self.builder.constant(Value::Float(factor));

		let Some(texture) = texture else {
			return Ok(factor);
		};

		let channel = self.builder.add(Node::Extract {
			value: texture,
			index: channel,
		})?;

		self.builder.add(Node::Binary {
			operator: BinaryOperator::Multiply,
			left: factor,
			right: channel,
		})
	}

	/// Adds a tangent-space normal map that samples container image `image`, scaled by `scale`.
	pub fn normal_map(&mut self, image: u32, scale: f32) -> Result<NodeId, GraphError> {
		let value = self.image(image, Type::Vector3)?;
		let scale = self.builder.constant(Value::Float(scale));

		self.builder.add(Node::NormalMap { value, scale })
	}

	/// Splits a `vector4` colour into the base colour and the opacity a surface reads.
	///
	/// Every importer writes its colour this way, which is the shape the material baker reads coverage from.
	pub fn color_and_opacity(&mut self, color: NodeId) -> Result<(NodeId, NodeId), GraphError> {
		let rgb = self.builder.add(Node::Swizzle {
			value: color,
			channels: vec![Channel::X, Channel::Y, Channel::Z],
		})?;
		let opacity = self.builder.add(Node::Extract { value: color, index: 3 })?;

		Ok((rgb, opacity))
	}

	/// Finishes the material with `surface` as its surface.
	pub fn finish(
		mut self,
		name: Option<String>,
		surface: Surface,
		double_sided: bool,
		alpha_mode: AlphaMode,
	) -> Result<ImportedMaterial, GraphError> {
		let surface = self.builder.add(Node::Surface(surface))?;

		Ok(ImportedMaterial {
			name,
			graph: self.builder.finish(surface)?,
			images: self.images,
			double_sided,
			alpha_mode,
		})
	}
}

/// The `PackingError` struct reports a channel read of a packed image that the packing does not store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackingError {
	pub texture: String,
	pub channel: u8,
}

impl Display for PackingError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"Material texture '{}' reads channel {}, which its packed image does not store. The most likely cause is an image shared between a packed and an unpacked use.",
			self.texture, self.channel
		)
	}
}

impl std::error::Error for PackingError {}
