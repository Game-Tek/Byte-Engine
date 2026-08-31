#[derive(Debug, PartialEq, Eq)]
pub enum MeshProcessingError {
	MissingAttribute(VertexSemantics, u32),
	DuplicateVertexSemantic(VertexSemantics),
	AttributeLengthMismatch(VertexSemantics, u32),
	InvalidTriangleIndexCount,
	InvalidPositionData,
	FailedToBuildMeshlets,
	InvalidSkeletonModel,
	SkinWithoutSkeleton,
	TooManySkinBindings {
		skins: usize,
	},
	TransformNodeWithoutSkeleton {
		primitive: usize,
		node: u32,
	},
	TransformNodeOutOfRange {
		primitive: usize,
		node: u32,
		nodes: usize,
	},
	SkinPaletteTooLarge {
		skin: usize,
		joints: usize,
	},
	SkinJointOutOfRange {
		skin: usize,
		joint: usize,
		node: u32,
		nodes: usize,
	},
	NonFiniteInverseBind {
		skin: usize,
	},
	SkinIndexOutOfRange {
		primitive: usize,
		skin: u32,
		skins: usize,
	},
	IncompleteSkinAttributes {
		primitive: usize,
	},
	UnboundSkinAttributes {
		primitive: usize,
	},
	MissingSkinVertexComponent(VertexSemantics),
	InvalidSkinVertexComponentFormat {
		semantic: VertexSemantics,
		expected: &'static str,
		actual: String,
	},
	SkinVertexCountMismatch {
		primitive: usize,
		values: usize,
		positions: usize,
	},
	VertexJointOutOfRange {
		primitive: usize,
		vertex: usize,
		lane: usize,
		joint: u16,
		palette_len: usize,
	},
	NonFiniteSkinWeight {
		primitive: usize,
		vertex: usize,
		lane: usize,
	},
	NegativeSkinWeight {
		primitive: usize,
		vertex: usize,
		lane: usize,
	},
	NonPositiveSkinWeightTotal {
		primitive: usize,
		vertex: usize,
	},
	NonNormalizedSkinWeights {
		primitive: usize,
		vertex: usize,
	},
}

impl std::fmt::Display for MeshProcessingError {
	// Keep every public processing error next to its cause guidance so additions remain exhaustive and reviewable.
	#[allow(clippy::too_many_lines)]
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::MissingAttribute(semantic, channel) => write!(
				f,
				"Mesh is missing a required vertex attribute. The most likely cause is that {semantic:?} on channel {channel} is present in the vertex layout but not in the primitive data."
			),
			Self::DuplicateVertexSemantic(semantic) => write!(
				f,
				"Mesh uses the same vertex semantic more than once. The most likely cause is that the current stream metadata cannot represent multiple channels for {semantic:?}."
			),
			Self::AttributeLengthMismatch(semantic, channel) => write!(
				f,
				"Mesh attribute length does not match the position stream. The most likely cause is that {semantic:?} on channel {channel} does not contain one value per vertex."
			),
			Self::InvalidTriangleIndexCount => write!(
				f,
				"Triangle index count is invalid. The most likely cause is that the index stream length is not divisible by three."
			),
			Self::InvalidPositionData => write!(
				f,
				"Mesh position data is invalid. The most likely cause is an empty position stream or a non-finite component."
			),
			Self::FailedToBuildMeshlets => write!(
				f,
				"Meshlet generation failed. The most likely cause is that the packed position stream could not be adapted for meshopt."
			),
			Self::InvalidSkeletonModel => write!(
				f,
				"Skeleton metadata is invalid. The most likely cause is that the mesh source contains an incompatible serialized skeleton model."
			),
			Self::SkinWithoutSkeleton => write!(
				f,
				"Mesh skin bindings have no skeleton. The most likely cause is that the importer omitted the skeleton reference while retaining skin palettes."
			),
			Self::TooManySkinBindings { skins } => write!(
				f,
				"Mesh has too many skin bindings. The most likely cause is that {skins} palettes cannot be addressed by the resource's u32 skin indices."
			),
			Self::TransformNodeWithoutSkeleton { primitive, node } => write!(
				f,
				"Primitive transform node has no skeleton. The most likely cause is that primitive {primitive} targets node {node} without retaining its hierarchy."
			),
			Self::TransformNodeOutOfRange { primitive, node, nodes } => write!(
				f,
				"Primitive transform node is outside the skeleton. The most likely cause is that primitive {primitive} targets node {node} in a {nodes}-node hierarchy."
			),
			Self::SkinPaletteTooLarge { skin, joints } => write!(
				f,
				"Skin palette is too large. The most likely cause is that skin {skin} contains {joints} entries, which cannot be addressed by u16 vertex joints."
			),
			Self::SkinJointOutOfRange {
				skin,
				joint,
				node,
				nodes,
			} => write!(
				f,
				"Skin joint is outside the skeleton. The most likely cause is that skin {skin} palette entry {joint} targets node {node} in a {nodes}-node skeleton."
			),
			Self::NonFiniteInverseBind { skin } => write!(
				f,
				"Skin inverse bind is not finite. The most likely cause is malformed transform data in skin {skin}."
			),
			Self::SkinIndexOutOfRange { primitive, skin, skins } => write!(
				f,
				"Primitive skin index is invalid. The most likely cause is that primitive {primitive} targets skin {skin} in a mesh with {skins} skins."
			),
			Self::IncompleteSkinAttributes { primitive } => write!(
				f,
				"Skinned primitive attributes are incomplete. The most likely cause is that primitive {primitive} does not provide joint and weight values."
			),
			Self::UnboundSkinAttributes { primitive } => write!(
				f,
				"Primitive skin attributes have no binding. The most likely cause is that primitive {primitive} provides joint or weight values without selecting a skin."
			),
			Self::MissingSkinVertexComponent(semantic) => write!(
				f,
				"Skin vertex layout is incomplete. The most likely cause is that {semantic:?} channel 0 was omitted from the declared mesh layout."
			),
			Self::InvalidSkinVertexComponentFormat {
				semantic,
				expected,
				actual,
			} => write!(
				f,
				"Skin vertex layout has an invalid format. The most likely cause is that {semantic:?} was declared as '{actual}' instead of '{expected}'."
			),
			Self::SkinVertexCountMismatch {
				primitive,
				values,
				positions,
			} => write!(
				f,
				"Vertex skin count does not match the position stream. The most likely cause is that primitive {primitive} contains {values} skin values for {positions} positions."
			),
			Self::VertexJointOutOfRange {
				primitive,
				vertex,
				lane,
				joint,
				palette_len,
			} => write!(
				f,
				"Vertex joint index is outside the skin palette. The most likely cause is that primitive {primitive} vertex {vertex} lane {lane} targets joint {joint} in a {palette_len}-entry palette."
			),
			Self::NonFiniteSkinWeight { primitive, vertex, lane } => write!(
				f,
				"Vertex skin weight is not finite. The most likely cause is malformed weight data in primitive {primitive} vertex {vertex} lane {lane}."
			),
			Self::NegativeSkinWeight { primitive, vertex, lane } => write!(
				f,
				"Vertex skin weight is negative. The most likely cause is malformed weight data in primitive {primitive} vertex {vertex} lane {lane}."
			),
			Self::NonPositiveSkinWeightTotal { primitive, vertex } => write!(
				f,
				"Vertex skin weight total is not positive. The most likely cause is that primitive {primitive} vertex {vertex} has no usable joint influence."
			),
			Self::NonNormalizedSkinWeights { primitive, vertex } => write!(
				f,
				"Vertex skin weights are not normalized. The most likely cause is that primitive {primitive} vertex {vertex} was not normalized after influence reduction."
			),
		}
	}
}

impl std::error::Error for MeshProcessingError {}

pub(super) fn validate_vertex_layout(vertex_layout: &[VertexComponent]) -> Result<(), MeshProcessingError> {
	let mut seen = [false; 8];
	for component in vertex_layout {
		let index = semantic_index(component.semantic);
		if seen[index] {
			return Err(MeshProcessingError::DuplicateVertexSemantic(component.semantic));
		}
		seen[index] = true;
	}
	Ok(())
}

/// Returns the validated node count needed to check primitive and skin references during streaming processing.
pub(super) fn skeleton_node_count(
	skeleton: Option<&ReferenceModel<SkeletonModel>>,
) -> Result<Option<usize>, MeshProcessingError> {
	skeleton
		.map(|skeleton| {
			crate::archived_from_slice::<SkeletonModel>(&skeleton.resource)
				.map_err(|_| MeshProcessingError::InvalidSkeletonModel)
				.and_then(|skeleton| {
					crate::resources::skeleton::validate_archived_nodes(skeleton.nodes.as_slice())
						.map_err(|_| MeshProcessingError::InvalidSkeletonModel)?;
					Ok(skeleton.nodes.len())
				})
		})
		.transpose()
}

pub(super) fn validate_skin_binding(
	skin_index: usize,
	skin: &SkinBinding,
	skeleton_nodes: Option<usize>,
) -> Result<(), MeshProcessingError> {
	let Some(node_count) = skeleton_nodes else {
		return Err(MeshProcessingError::SkinWithoutSkeleton);
	};
	if skin.len() > u16::MAX as usize + 1 {
		return Err(MeshProcessingError::SkinPaletteTooLarge {
			skin: skin_index,
			joints: skin.len(),
		});
	}
	for (joint_index, entry) in skin.entries.iter().enumerate() {
		if let SkinJoint::Node(node) = entry.joint
			&& node as usize >= node_count
		{
			return Err(MeshProcessingError::SkinJointOutOfRange {
				skin: skin_index,
				joint: joint_index,
				node,
				nodes: node_count,
			});
		}
		if !entry
			.adjusted_inverse_bind_matrix
			.iter()
			.flatten()
			.all(|value| value.is_finite())
		{
			return Err(MeshProcessingError::NonFiniteInverseBind { skin: skin_index });
		}
	}
	Ok(())
}

pub(super) fn validate_primitive_metadata(
	primitive: usize,
	transform_node: Option<u32>,
	skin: Option<u32>,
	has_vertex_skin: bool,
	vertex_layout: &[VertexComponent],
	skeleton_nodes: Option<usize>,
	skins: &[SkinBinding],
) -> Result<(), MeshProcessingError> {
	if let Some(node) = transform_node {
		let Some(nodes) = skeleton_nodes else {
			return Err(MeshProcessingError::TransformNodeWithoutSkeleton { primitive, node });
		};
		if node as usize >= nodes {
			return Err(MeshProcessingError::TransformNodeOutOfRange { primitive, node, nodes });
		}
	}
	match skin {
		Some(skin) => {
			if skin as usize >= skins.len() {
				return Err(MeshProcessingError::SkinIndexOutOfRange {
					primitive,
					skin,
					skins: skins.len(),
				});
			}
			if !has_vertex_skin {
				return Err(MeshProcessingError::IncompleteSkinAttributes { primitive });
			}
			validate_skin_vertex_layout(vertex_layout)?;
		}
		None if has_vertex_skin => return Err(MeshProcessingError::UnboundSkinAttributes { primitive }),
		None => {}
	}
	Ok(())
}

fn validate_skin_vertex_layout(vertex_layout: &[VertexComponent]) -> Result<(), MeshProcessingError> {
	for (semantic, expected) in [(VertexSemantics::Joints, "vec4u16"), (VertexSemantics::Weights, "vec4f")] {
		let Some(component) = vertex_layout
			.iter()
			.find(|component| component.semantic == semantic && component.channel == 0)
		else {
			return Err(MeshProcessingError::MissingSkinVertexComponent(semantic));
		};
		if component.format != expected {
			return Err(MeshProcessingError::InvalidSkinVertexComponentFormat {
				semantic,
				expected,
				actual: component.format.clone(),
			});
		}
	}
	Ok(())
}

pub(super) fn validate_vertex_skin(
	primitive: usize,
	vertex: usize,
	vertex_skin: VertexSkin,
	skin: &SkinBinding,
) -> Result<(), MeshProcessingError> {
	let mut total = 0.0;
	for lane in 0..4 {
		let joint = vertex_skin.joints[lane];
		if joint as usize >= skin.len() {
			return Err(MeshProcessingError::VertexJointOutOfRange {
				primitive,
				vertex,
				lane,
				joint,
				palette_len: skin.len(),
			});
		}
		let weight = vertex_skin.weights[lane];
		if !weight.is_finite() {
			return Err(MeshProcessingError::NonFiniteSkinWeight { primitive, vertex, lane });
		}
		if weight < 0.0 {
			return Err(MeshProcessingError::NegativeSkinWeight { primitive, vertex, lane });
		}
		total += weight;
	}
	if total <= 0.0 {
		return Err(MeshProcessingError::NonPositiveSkinWeightTotal { primitive, vertex });
	}
	if (total - 1.0).abs() > 1.0e-4 {
		return Err(MeshProcessingError::NonNormalizedSkinWeights { primitive, vertex });
	}
	Ok(())
}

const fn semantic_index(semantic: VertexSemantics) -> usize {
	match semantic {
		VertexSemantics::Position => 0,
		VertexSemantics::Normal => 1,
		VertexSemantics::Tangent => 2,
		VertexSemantics::BiTangent => 3,
		VertexSemantics::UV => 4,
		VertexSemantics::Color => 5,
		VertexSemantics::Joints => 6,
		VertexSemantics::Weights => 7,
	}
}

use super::source::VertexSkin;
use crate::{
	ReferenceModel,
	resources::skeleton::{SkeletonModel, SkinBinding, SkinJoint},
	types::{VertexComponent, VertexSemantics},
};
