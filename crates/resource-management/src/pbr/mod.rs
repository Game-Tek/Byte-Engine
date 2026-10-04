use crate::{processors::processor::implementations::image::ChannelPacking, types::AlphaMode};

pub mod gltf;
pub mod shader;

pub use gltf::brdf_material_from_gltf;
pub use shader::{BrdfShaderGenerationError, generate_solid_brdf_program, generate_textured_brdf_program};

/// Names the generated shader variable that holds one material texture slot.
pub(crate) fn material_texture_variable_name(slot: u32) -> String {
	format!("material_texture_{slot}")
}

/// The `BrdfMaterialDescription` struct stores a backend-neutral material graph for surface BRDFs.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct BrdfMaterialDescription {
	pub name: Option<String>,
	pub nodes: Vec<BrdfNode>,
	pub surface: BrdfNodeId,
	pub double_sided: bool,
	pub alpha_mode: BrdfAlphaMode,
}

impl BrdfMaterialDescription {
	/// Rewrites every texture node's image index into the material slot that binds it.
	///
	/// Importers call this before shader generation so materials that bind different images through the same graph
	/// produce the same program. `slot_for` receives the imported image index and returns its slot.
	pub(crate) fn assign_texture_slots(&mut self, mut slot_for: impl FnMut(u32) -> u32) {
		for node in &mut self.nodes {
			if let BrdfNode::Texture(texture) = node {
				texture.image_index = slot_for(texture.image_index);
			}
		}
	}

	/// Moves channel reads of packed textures to where the image processor stored them.
	///
	/// Importers call this after building a graph and before [`Self::assign_texture_slots`]. `packing_for` receives an
	/// imported image index and returns the [`ChannelPacking`] that image bakes with, or `None` when it keeps all of
	/// its channels. A read of a channel the packing drops is an error, because the shader would sample data that is
	/// not there.
	pub(crate) fn pack_texture_channels(
		&mut self,
		mut packing_for: impl FnMut(u32) -> Option<ChannelPacking>,
	) -> Result<(), BrdfMaterialValidationError> {
		for index in 0..self.nodes.len() {
			if let BrdfNode::ExtractChannel { source, channel } = self.nodes[index]
				&& let BrdfNode::Texture(texture) = self.node(source)?
				&& let Some(packing) = packing_for(texture.image_index)
			{
				let stored = packing
					.stored_channel(channel.index())
					.and_then(BrdfChannel::from_index)
					.ok_or(BrdfMaterialValidationError::ChannelNotStored {
						node: BrdfNodeId::new(index as u32),
						channel,
					})?;
				self.nodes[index] = BrdfNode::ExtractChannel { source, channel: stored };
			}
		}
		Ok(())
	}

	/// Validates that all node references point to existing nodes and that the graph root is a surface node.
	pub fn validate(&self) -> Result<(), BrdfMaterialValidationError> {
		self.ensure_node_exists(self.surface)?;

		match self.node(self.surface)? {
			BrdfNode::MetallicRoughness(_) => {}
			_ => return Err(BrdfMaterialValidationError::SurfaceNodeMustBeBrdf),
		}

		for (index, node) in self.nodes.iter().enumerate() {
			let node_id = BrdfNodeId::new(index as u32);
			match node {
				BrdfNode::Constant(_) | BrdfNode::Texture(_) => {}
				BrdfNode::Multiply { left, right } => {
					self.ensure_child_node_exists(node_id, *left)?;
					self.ensure_child_node_exists(node_id, *right)?;
				}
				BrdfNode::ExtractChannel { source, .. } => {
					self.ensure_child_node_exists(node_id, *source)?;
				}
				BrdfNode::MetallicRoughness(brdf) => {
					self.ensure_child_node_exists(node_id, brdf.base_color)?;
					self.ensure_child_node_exists(node_id, brdf.metallic)?;
					self.ensure_child_node_exists(node_id, brdf.roughness)?;
					self.ensure_optional_child_node_exists(node_id, brdf.normal)?;
					self.ensure_optional_child_node_exists(node_id, brdf.occlusion)?;
					self.ensure_optional_child_node_exists(node_id, brdf.emission)?;
				}
				BrdfNode::NormalMap { source, .. } => {
					self.ensure_child_node_exists(node_id, *source)?;
				}
				BrdfNode::Occlusion { source, .. } => {
					self.ensure_child_node_exists(node_id, *source)?;
				}
				BrdfNode::Emission { color } => {
					self.ensure_child_node_exists(node_id, *color)?;
				}
			}
		}

		Ok(())
	}

	pub fn node(&self, id: BrdfNodeId) -> Result<&BrdfNode, BrdfMaterialValidationError> {
		self.nodes
			.get(id.index())
			.ok_or(BrdfMaterialValidationError::MissingNode { id })
	}

	fn ensure_node_exists(&self, id: BrdfNodeId) -> Result<(), BrdfMaterialValidationError> {
		if id.index() < self.nodes.len() {
			Ok(())
		} else {
			Err(BrdfMaterialValidationError::MissingNode { id })
		}
	}

	fn ensure_child_node_exists(&self, node: BrdfNodeId, child: BrdfNodeId) -> Result<(), BrdfMaterialValidationError> {
		if child.index() < self.nodes.len() {
			Ok(())
		} else {
			Err(BrdfMaterialValidationError::MissingChildNode { node, child })
		}
	}

	fn ensure_optional_child_node_exists(
		&self,
		node: BrdfNodeId,
		child: Option<BrdfNodeId>,
	) -> Result<(), BrdfMaterialValidationError> {
		if let Some(child) = child {
			self.ensure_child_node_exists(node, child)
		} else {
			Ok(())
		}
	}
}

/// The `BrdfMaterialValidationError` enum identifies invalid references in a material graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrdfMaterialValidationError {
	MissingNode {
		id: BrdfNodeId,
	},
	MissingChildNode {
		node: BrdfNodeId,
		child: BrdfNodeId,
	},
	SurfaceNodeMustBeBrdf,
	/// A channel read of a packed texture names a channel the packing does not store.
	ChannelNotStored {
		node: BrdfNodeId,
		channel: BrdfChannel,
	},
}

/// The `BrdfNodeId` struct identifies a node inside a material graph arena.
#[derive(
	Clone,
	Copy,
	Debug,
	Eq,
	PartialEq,
	Hash,
	serde::Serialize,
	serde::Deserialize,
	rkyv::Archive,
	rkyv::Serialize,
	rkyv::Deserialize,
)]
pub struct BrdfNodeId(u32);

impl BrdfNodeId {
	pub fn new(index: u32) -> Self {
		Self(index)
	}

	pub fn index(self) -> usize {
		self.0 as usize
	}
}

/// The `BrdfNode` enum identifies the operations available in a backend-neutral BRDF material graph.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum BrdfNode {
	Constant(BrdfValue),
	Texture(BrdfTexture),
	Multiply { left: BrdfNodeId, right: BrdfNodeId },
	ExtractChannel { source: BrdfNodeId, channel: BrdfChannel },
	MetallicRoughness(BrdfMetallicRoughness),
	NormalMap { source: BrdfNodeId, scale: f32 },
	Occlusion { source: BrdfNodeId, strength: f32 },
	Emission { color: BrdfNodeId },
}

/// The `BrdfValue` enum stores typed constants used by BRDF graph nodes.
#[derive(
	Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum BrdfValue {
	Scalar(f32),
	Vector3([f32; 3]),
	Vector4([f32; 4]),
}

/// The `BrdfTexture` struct provides a storage-independent texture sample for a BRDF graph.
#[derive(
	Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct BrdfTexture {
	pub image_index: u32,
	pub texcoord_channel: u32,
}

/// The `BrdfChannel` enum identifies one channel from a vector-producing graph node.
#[derive(
	Clone,
	Copy,
	Debug,
	Eq,
	PartialEq,
	Hash,
	serde::Serialize,
	serde::Deserialize,
	rkyv::Archive,
	rkyv::Serialize,
	rkyv::Deserialize,
)]
pub enum BrdfChannel {
	Red,
	Green,
	Blue,
	Alpha,
}

impl BrdfChannel {
	/// Returns the channel's position in an RGBA texel.
	pub fn index(self) -> usize {
		self as usize
	}

	/// Returns the channel at `index` in an RGBA texel.
	pub fn from_index(index: usize) -> Option<Self> {
		[Self::Red, Self::Green, Self::Blue, Self::Alpha].get(index).copied()
	}
}

/// The `BrdfMetallicRoughness` struct provides the metallic-roughness root for a surface BRDF graph.
#[derive(
	Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct BrdfMetallicRoughness {
	pub base_color: BrdfNodeId,
	pub metallic: BrdfNodeId,
	pub roughness: BrdfNodeId,
	pub normal: Option<BrdfNodeId>,
	pub occlusion: Option<BrdfNodeId>,
	pub emission: Option<BrdfNodeId>,
}

/// The `BrdfAlphaMode` enum identifies how alpha affects surface visibility.
#[derive(
	Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum BrdfAlphaMode {
	Opaque,
	Mask(f32),
	Blend,
}

impl From<AlphaMode> for BrdfAlphaMode {
	fn from(value: AlphaMode) -> Self {
		match value {
			AlphaMode::Opaque => BrdfAlphaMode::Opaque,
			AlphaMode::Mask(cutoff) => BrdfAlphaMode::Mask(cutoff),
			AlphaMode::Blend => BrdfAlphaMode::Blend,
		}
	}
}

impl From<BrdfAlphaMode> for AlphaMode {
	fn from(value: BrdfAlphaMode) -> Self {
		match value {
			BrdfAlphaMode::Opaque => AlphaMode::Opaque,
			BrdfAlphaMode::Mask(cutoff) => AlphaMode::Mask(cutoff),
			BrdfAlphaMode::Blend => AlphaMode::Blend,
		}
	}
}

/// The `BrdfMaterialBuilder` struct builds flat material graphs while assigning stable node ids.
#[derive(Debug, Default)]
pub struct BrdfMaterialBuilder {
	nodes: Vec<BrdfNode>,
}

impl BrdfMaterialBuilder {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn add(&mut self, node: BrdfNode) -> BrdfNodeId {
		let index = self.nodes.len();

		assert!(
			index <= u32::MAX as usize,
			"BRDF material node count exceeded u32::MAX. The most likely cause is an invalid importer producing an unbounded graph."
		);
		self.nodes.push(node);
		BrdfNodeId::new(index as u32)
	}

	pub fn constant(&mut self, value: BrdfValue) -> BrdfNodeId {
		self.add(BrdfNode::Constant(value))
	}

	pub fn texture(&mut self, texture: BrdfTexture) -> BrdfNodeId {
		self.add(BrdfNode::Texture(texture))
	}

	pub fn multiply(&mut self, left: BrdfNodeId, right: BrdfNodeId) -> BrdfNodeId {
		self.add(BrdfNode::Multiply { left, right })
	}

	pub fn extract_channel(&mut self, source: BrdfNodeId, channel: BrdfChannel) -> BrdfNodeId {
		self.add(BrdfNode::ExtractChannel { source, channel })
	}

	pub fn finish(
		self,
		name: Option<String>,
		surface: BrdfNodeId,
		double_sided: bool,
		alpha_mode: BrdfAlphaMode,
	) -> BrdfMaterialDescription {
		BrdfMaterialDescription {
			name,
			nodes: self.nodes,
			surface,
			double_sided,
			alpha_mode,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn packing_moves_kept_channel_reads_and_rejects_dropped_ones() {
		let packing = ChannelPacking { source_channels: [1, 2] };
		let mut builder = BrdfMaterialBuilder::new();
		let packed = builder.texture(BrdfTexture {
			image_index: 0,
			texcoord_channel: 0,
		});
		let unpacked = builder.texture(BrdfTexture {
			image_index: 1,
			texcoord_channel: 0,
		});
		let metallic = builder.extract_channel(packed, BrdfChannel::Blue);
		let roughness = builder.extract_channel(unpacked, BrdfChannel::Red);
		let base_color = builder.constant(BrdfValue::Vector4([1.0; 4]));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let mut material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		material
			.pack_texture_channels(|image| (image == 0).then_some(packing))
			.expect("reads of kept channels should remap");

		assert_eq!(
			material.nodes[metallic.index()],
			BrdfNode::ExtractChannel {
				source: packed,
				channel: BrdfChannel::Green
			},
			"blue moves to the second stored channel"
		);
		assert_eq!(
			material.nodes[roughness.index()],
			BrdfNode::ExtractChannel {
				source: unpacked,
				channel: BrdfChannel::Red
			},
			"reads of unpacked images stay"
		);

		assert_eq!(
			material.pack_texture_channels(|_| Some(packing)),
			Err(BrdfMaterialValidationError::ChannelNotStored {
				node: roughness,
				channel: BrdfChannel::Red
			}),
			"reads of a packed image's dropped channels are rejected"
		);
	}

	#[test]
	fn validation_rejects_missing_surface_node() {
		let material = BrdfMaterialDescription {
			name: None,
			nodes: Vec::new(),
			surface: BrdfNodeId::new(0),
			double_sided: false,
			alpha_mode: BrdfAlphaMode::Opaque,
		};

		assert_eq!(
			material.validate(),
			Err(BrdfMaterialValidationError::MissingNode { id: BrdfNodeId::new(0) })
		);
	}

	#[test]
	fn validation_rejects_non_brdf_surface_node() {
		let mut builder = BrdfMaterialBuilder::new();
		let surface = builder.constant(BrdfValue::Scalar(1.0));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		assert_eq!(material.validate(), Err(BrdfMaterialValidationError::SurfaceNodeMustBeBrdf));
	}

	#[test]
	fn validation_rejects_missing_brdf_children() {
		let material = BrdfMaterialDescription {
			name: None,
			nodes: vec![BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
				base_color: BrdfNodeId::new(1),
				metallic: BrdfNodeId::new(0),
				roughness: BrdfNodeId::new(0),
				normal: None,
				occlusion: None,
				emission: None,
			})],
			surface: BrdfNodeId::new(0),
			double_sided: false,
			alpha_mode: BrdfAlphaMode::Opaque,
		};

		assert_eq!(
			material.validate(),
			Err(BrdfMaterialValidationError::MissingChildNode {
				node: BrdfNodeId::new(0),
				child: BrdfNodeId::new(1),
			})
		);
	}
}
