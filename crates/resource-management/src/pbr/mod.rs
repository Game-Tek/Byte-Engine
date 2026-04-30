use crate::types::AlphaMode;

pub mod gltf;
pub mod shader;

pub use gltf::brdf_material_from_gltf;
pub use shader::{generate_solid_brdf_program, BrdfShaderGenerationError};

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

/// The `BrdfMaterialValidationError` enum describes invalid material graph references.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrdfMaterialValidationError {
	MissingNode { id: BrdfNodeId },
	MissingChildNode { node: BrdfNodeId, child: BrdfNodeId },
	SurfaceNodeMustBeBrdf,
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

/// The `BrdfNode` enum describes each operation in a backend-neutral BRDF material graph.
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

/// The `BrdfTexture` struct describes a texture sample independent of engine resource storage.
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

/// The `BrdfMetallicRoughness` struct represents a metallic-roughness surface BRDF root.
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

/// The `BrdfAlphaMode` enum describes how alpha participates in surface visibility.
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
	fn builder_assigns_stable_dense_node_ids() {
		let mut builder = BrdfMaterialBuilder::new();

		let first = builder.constant(BrdfValue::Scalar(1.0));
		let second = builder.constant(BrdfValue::Scalar(2.0));
		let third = builder.multiply(first, second);

		assert_eq!(first, BrdfNodeId::new(0));
		assert_eq!(second, BrdfNodeId::new(1));
		assert_eq!(third, BrdfNodeId::new(2));
	}

	#[test]
	fn validates_complete_material_graph() {
		let material = test_material_graph();

		assert_eq!(material.validate(), Ok(()));
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
	fn validation_rejects_missing_multiply_children() {
		let material = material_with_surface(BrdfNode::Multiply {
			left: BrdfNodeId::new(10),
			right: BrdfNodeId::new(0),
		});

		assert_eq!(
			material.validate(),
			Err(BrdfMaterialValidationError::MissingChildNode {
				node: BrdfNodeId::new(0),
				child: BrdfNodeId::new(10),
			})
		);
	}

	#[test]
	fn validation_rejects_missing_extract_channel_source() {
		let material = material_with_surface(BrdfNode::ExtractChannel {
			source: BrdfNodeId::new(4),
			channel: BrdfChannel::Green,
		});

		assert_eq!(
			material.validate(),
			Err(BrdfMaterialValidationError::MissingChildNode {
				node: BrdfNodeId::new(0),
				child: BrdfNodeId::new(4),
			})
		);
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

	#[test]
	fn validation_rejects_missing_normal_occlusion_and_emission_children() {
		for node in [
			BrdfNode::NormalMap {
				source: BrdfNodeId::new(3),
				scale: 1.0,
			},
			BrdfNode::Occlusion {
				source: BrdfNodeId::new(3),
				strength: 1.0,
			},
			BrdfNode::Emission {
				color: BrdfNodeId::new(3),
			},
		] {
			let material = material_with_surface(node);

			assert_eq!(
				material.validate(),
				Err(BrdfMaterialValidationError::MissingChildNode {
					node: BrdfNodeId::new(0),
					child: BrdfNodeId::new(3),
				})
			);
		}
	}

	fn test_material_graph() -> BrdfMaterialDescription {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.constant(BrdfValue::Vector4([1.0, 1.0, 1.0, 1.0]));
		let metallic = builder.constant(BrdfValue::Scalar(1.0));
		let roughness = builder.constant(BrdfValue::Scalar(1.0));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		builder.finish(None, surface, false, BrdfAlphaMode::Opaque)
	}

	fn material_with_surface(child: BrdfNode) -> BrdfMaterialDescription {
		BrdfMaterialDescription {
			name: None,
			nodes: vec![
				child,
				BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
					base_color: BrdfNodeId::new(0),
					metallic: BrdfNodeId::new(0),
					roughness: BrdfNodeId::new(0),
					normal: None,
					occlusion: None,
					emission: None,
				}),
			],
			surface: BrdfNodeId::new(1),
			double_sided: false,
			alpha_mode: BrdfAlphaMode::Opaque,
		}
	}
}
