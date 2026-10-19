//! Holds the materials glTF and FBX importers build, and the texture bookkeeping they share.

use std::fmt::{Display, Formatter};

use super::graph::{BinaryOperator, Channel, Graph, GraphBuilder, GraphError, Node, NodeId, Surface, Type, Value};
use super::{material_texture_parameter_name, material_texture_slot};
use crate::processors::image::ChannelPacking;
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
