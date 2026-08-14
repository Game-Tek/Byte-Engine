use super::source::{MeshAttributeData, MeshPrimitiveSource, MeshSource};
use crate::{
	resources::skeleton::{SkeletonModel, SkinBinding, SkinJoint},
	types::{VertexComponent, VertexSemantics},
};

#[derive(Debug, PartialEq, Eq)]
pub enum MeshProcessingError {
	MissingPositionAttribute,
	MissingTriangleIndices,
	MissingAttribute(VertexSemantics, u32),
	DuplicateVertexSemantic(VertexSemantics),
	InvalidAttributeFormat(VertexSemantics),
	AttributeLengthMismatch(VertexSemantics, u32),
	InconsistentVertexCount,
	InvalidTriangleIndexCount,
	FailedToBuildMeshlets,
	InvalidSkeletonModel,
	SkinWithoutSkeleton,
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
	SkinAttributeLengthMismatch {
		primitive: usize,
		joints: usize,
		weights: usize,
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
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			MeshProcessingError::MissingPositionAttribute => {
				write!(
					f,
					"Mesh is missing the position attribute. The most likely cause is that the mesh source did not expose `Position` on channel 0."
				)
			}
			MeshProcessingError::MissingTriangleIndices => {
				write!(
					f,
					"Mesh is missing triangle indices. The most likely cause is that the mesh source did not expose a triangle index stream."
				)
			}
			MeshProcessingError::MissingAttribute(semantic, channel) => {
				write!(
					f,
					"Mesh is missing a required vertex attribute. The most likely cause is that {:?} on channel {} is present in the vertex layout but not in the primitive data.",
					semantic, channel
				)
			}
			MeshProcessingError::DuplicateVertexSemantic(semantic) => {
				write!(
					f,
					"Mesh uses the same vertex semantic more than once. The most likely cause is that the current stream metadata cannot represent multiple channels for {:?}.",
					semantic
				)
			}
			MeshProcessingError::InvalidAttributeFormat(semantic) => {
				write!(
					f,
					"Mesh uses an unsupported vertex attribute format. The most likely cause is that {:?} was exposed with a shape that does not match the engine stream layout.",
					semantic
				)
			}
			MeshProcessingError::AttributeLengthMismatch(semantic, channel) => {
				write!(
					f,
					"Mesh attribute length does not match the position stream. The most likely cause is that {:?} on channel {} does not contain one value per vertex.",
					semantic, channel
				)
			}
			MeshProcessingError::InconsistentVertexCount => {
				write!(
					f,
					"Mesh primitive reported an inconsistent vertex count. The most likely cause is that the primitive metadata does not match its position attribute length."
				)
			}
			MeshProcessingError::InvalidTriangleIndexCount => {
				write!(
					f,
					"Triangle index count is invalid. The most likely cause is that the index stream length is not divisible by three."
				)
			}
			MeshProcessingError::FailedToBuildMeshlets => {
				write!(
					f,
					"Meshlet generation failed. The most likely cause is that the packed position stream could not be adapted for meshopt."
				)
			}
			MeshProcessingError::InvalidSkeletonModel => write!(
				f,
				"Skeleton metadata is invalid. The most likely cause is that the mesh source contains an incompatible serialized skeleton model."
			),
			MeshProcessingError::SkinWithoutSkeleton => write!(
				f,
				"Mesh skin bindings have no skeleton. The most likely cause is that the importer omitted the skeleton reference while retaining skin palettes."
			),
			MeshProcessingError::TransformNodeWithoutSkeleton { primitive, node } => write!(
				f,
				"Primitive transform node has no skeleton. The most likely cause is that primitive {primitive} targets node {node} without retaining its hierarchy."
			),
			MeshProcessingError::TransformNodeOutOfRange { primitive, node, nodes } => write!(
				f,
				"Primitive transform node is outside the skeleton. The most likely cause is that primitive {primitive} targets node {node} in a {nodes}-node hierarchy."
			),
			MeshProcessingError::SkinPaletteTooLarge { skin, joints } => write!(
				f,
				"Skin palette is too large. The most likely cause is that skin {skin} contains {joints} entries, which cannot be addressed by u16 vertex joints."
			),
			MeshProcessingError::SkinJointOutOfRange {
				skin,
				joint,
				node,
				nodes,
			} => write!(
				f,
				"Skin joint is outside the skeleton. The most likely cause is that skin {skin} palette entry {joint} targets node {node} in a {nodes}-node skeleton."
			),
			MeshProcessingError::NonFiniteInverseBind { skin } => write!(
				f,
				"Skin inverse bind is not finite. The most likely cause is malformed transform data in skin {skin}."
			),
			MeshProcessingError::SkinIndexOutOfRange { primitive, skin, skins } => write!(
				f,
				"Primitive skin index is invalid. The most likely cause is that primitive {primitive} targets skin {skin} in a mesh with {skins} skins."
			),
			MeshProcessingError::IncompleteSkinAttributes { primitive } => write!(
				f,
				"Skinned primitive attributes are incomplete. The most likely cause is that primitive {primitive} does not provide both joint and weight values."
			),
			MeshProcessingError::UnboundSkinAttributes { primitive } => write!(
				f,
				"Primitive skin attributes have no binding. The most likely cause is that primitive {primitive} provides joint or weight values without selecting a skin."
			),
			MeshProcessingError::MissingSkinVertexComponent(semantic) => write!(
				f,
				"Skin vertex layout is incomplete. The most likely cause is that {semantic:?} channel 0 was omitted from the declared mesh layout."
			),
			MeshProcessingError::InvalidSkinVertexComponentFormat {
				semantic,
				expected,
				actual,
			} => write!(
				f,
				"Skin vertex layout has an invalid format. The most likely cause is that {semantic:?} was declared as '{actual}' instead of '{expected}'."
			),
			MeshProcessingError::SkinAttributeLengthMismatch {
				primitive,
				joints,
				weights,
			} => write!(
				f,
				"Skin attribute lengths do not match. The most likely cause is that primitive {primitive} contains {joints} joint values but {weights} weight values."
			),
			MeshProcessingError::VertexJointOutOfRange {
				primitive,
				vertex,
				lane,
				joint,
				palette_len,
			} => write!(
				f,
				"Vertex joint index is outside the skin palette. The most likely cause is that primitive {primitive} vertex {vertex} lane {lane} targets joint {joint} in a {palette_len}-entry palette."
			),
			MeshProcessingError::NonFiniteSkinWeight { primitive, vertex, lane } => write!(
				f,
				"Vertex skin weight is not finite. The most likely cause is malformed weight data in primitive {primitive} vertex {vertex} lane {lane}."
			),
			MeshProcessingError::NegativeSkinWeight { primitive, vertex, lane } => write!(
				f,
				"Vertex skin weight is negative. The most likely cause is malformed weight data in primitive {primitive} vertex {vertex} lane {lane}."
			),
			MeshProcessingError::NonPositiveSkinWeightTotal { primitive, vertex } => write!(
				f,
				"Vertex skin weight total is not positive. The most likely cause is that primitive {primitive} vertex {vertex} has no usable joint influence."
			),
			MeshProcessingError::NonNormalizedSkinWeights { primitive, vertex } => write!(
				f,
				"Vertex skin weights are not normalized. The most likely cause is that primitive {primitive} vertex {vertex} was not normalized after influence reduction."
			),
		}
	}
}

impl std::error::Error for MeshProcessingError {}

pub(super) fn validate_vertex_layout(vertex_layout: &[VertexComponent]) -> Result<(), MeshProcessingError> {
	let mut seen = Vec::with_capacity(vertex_layout.len());

	for component in vertex_layout {
		if seen.contains(&component.semantic) {
			return Err(MeshProcessingError::DuplicateVertexSemantic(component.semantic));
		}

		seen.push(component.semantic);
	}

	Ok(())
}

/// Validates skin references before packing so invalid vertex palettes never enter stored mesh resources.
pub(super) fn validate_skin_source<T: MeshSource>(source: &T) -> Result<(), MeshProcessingError> {
	let skeleton_nodes = source
		.skeleton()
		.map(|skeleton| {
			crate::archived_from_slice::<SkeletonModel>(&skeleton.resource)
				.map_err(|_| MeshProcessingError::InvalidSkeletonModel)
				.and_then(|skeleton| {
					crate::resources::skeleton::validate_archived_nodes(skeleton.nodes.as_slice())
						.map_err(|_| MeshProcessingError::InvalidSkeletonModel)?;
					Ok(skeleton.nodes.len())
				})
		})
		.transpose()?;

	if !source.skins().is_empty() && skeleton_nodes.is_none() {
		return Err(MeshProcessingError::SkinWithoutSkeleton);
	}

	for (skin_index, skin) in source.skins().iter().enumerate() {
		if skin.len() > u16::MAX as usize + 1 {
			return Err(MeshProcessingError::SkinPaletteTooLarge {
				skin: skin_index,
				joints: skin.len(),
			});
		}

		let node_count = skeleton_nodes.unwrap_or(0);
		for (joint_index, entry) in skin.entries.iter().enumerate() {
			if let SkinJoint::Node(node) = entry.joint {
				if node as usize >= node_count {
					return Err(MeshProcessingError::SkinJointOutOfRange {
						skin: skin_index,
						joint: joint_index,
						node,
						nodes: node_count,
					});
				}
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
	}

	let mut validated_skin_layout = false;
	for (primitive_index, primitive) in source.primitives().enumerate() {
		if let Some(node) = primitive.transform_node() {
			let Some(node_count) = skeleton_nodes else {
				return Err(MeshProcessingError::TransformNodeWithoutSkeleton {
					primitive: primitive_index,
					node,
				});
			};
			if node as usize >= node_count {
				return Err(MeshProcessingError::TransformNodeOutOfRange {
					primitive: primitive_index,
					node,
					nodes: node_count,
				});
			}
		}
		let joints = primitive.attribute(VertexSemantics::Joints, 0);
		let weights = primitive.attribute(VertexSemantics::Weights, 0);

		match primitive.skin() {
			Some(skin) => {
				if skin as usize >= source.skins().len() {
					return Err(MeshProcessingError::SkinIndexOutOfRange {
						primitive: primitive_index,
						skin,
						skins: source.skins().len(),
					});
				}
				if !validated_skin_layout {
					validate_skin_vertex_layout(source.vertex_layout())?;
					validated_skin_layout = true;
				}
				let (Some(joints), Some(weights)) = (joints, weights) else {
					return Err(MeshProcessingError::IncompleteSkinAttributes {
						primitive: primitive_index,
					});
				};
				validate_skin_vertex_data(primitive_index, joints, weights, &source.skins()[skin as usize])?;
			}
			None if joints.is_some() || weights.is_some() => {
				return Err(MeshProcessingError::UnboundSkinAttributes {
					primitive: primitive_index,
				});
			}
			None => {}
		}
	}

	Ok(())
}

/// Validates the declared shader types required to pack fixed-width skin attributes.
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

/// Validates palette-local joint indices and normalized weights before they are copied into GPU-facing streams.
fn validate_skin_vertex_data(
	primitive: usize,
	joints: MeshAttributeData<'_>,
	weights: MeshAttributeData<'_>,
	skin: &SkinBinding,
) -> Result<(), MeshProcessingError> {
	let MeshAttributeData::U16x4(joints) = joints else {
		return Err(MeshProcessingError::InvalidAttributeFormat(VertexSemantics::Joints));
	};
	let MeshAttributeData::F32x4(weights) = weights else {
		return Err(MeshProcessingError::InvalidAttributeFormat(VertexSemantics::Weights));
	};
	if joints.len() != weights.len() {
		return Err(MeshProcessingError::SkinAttributeLengthMismatch {
			primitive,
			joints: joints.len(),
			weights: weights.len(),
		});
	}

	for (vertex, (vertex_joints, vertex_weights)) in joints.iter().zip(weights).enumerate() {
		let mut total = 0.0;
		for lane in 0..4 {
			let joint = vertex_joints[lane];
			if joint as usize >= skin.len() {
				return Err(MeshProcessingError::VertexJointOutOfRange {
					primitive,
					vertex,
					lane,
					joint,
					palette_len: skin.len(),
				});
			}

			let weight = vertex_weights[lane];
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
	}

	Ok(())
}
