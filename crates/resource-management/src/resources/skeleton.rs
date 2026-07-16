use crate::{resource, solver::SolveErrors, Model, Reference, ReferenceModel, Resource, Solver};

/// Stores a four-by-four matrix as four column vectors.
pub type Matrix4Columns = [[f32; 4]; 4];

/// Returns the identity matrix in the resource matrix column layout.
pub const fn identity_matrix4_columns() -> Matrix4Columns {
	[
		[1.0, 0.0, 0.0, 0.0],
		[0.0, 1.0, 0.0, 0.0],
		[0.0, 0.0, 1.0, 0.0],
		[0.0, 0.0, 0.0, 1.0],
	]
}

/// The `LocalTransform` struct preserves the blendable local pose used by skeleton nodes and animation tracks.
#[derive(
	Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct LocalTransform {
	pub translation: [f32; 3],
	pub rotation: [f32; 4],
	pub scale: [f32; 3],
}

impl LocalTransform {
	/// Creates the neutral local pose used for nodes without an authored transform.
	pub const fn identity() -> Self {
		Self {
			translation: [0.0, 0.0, 0.0],
			rotation: [0.0, 0.0, 0.0, 1.0],
			scale: [1.0, 1.0, 1.0],
		}
	}
}

impl Default for LocalTransform {
	fn default() -> Self {
		Self::identity()
	}
}

/// The `SkeletonNode` struct preserves one hierarchy entry and its fallback pose for CPU animation evaluation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SkeletonNode {
	pub name: Option<String>,
	pub parent: Option<u32>,
	pub rest_local: LocalTransform,
}

/// The `SkinJoint` enum maps a palette entry either to a skeleton node or to an identity fallback.
#[derive(
	Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum SkinJoint {
	Node(u32),
	Identity,
}

/// The `SkinPaletteEntry` struct keeps one GPU palette joint paired with the matrix needed to skin flattened vertices.
#[derive(
	Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct SkinPaletteEntry {
	pub joint: SkinJoint,
	pub adjusted_inverse_bind_matrix: Matrix4Columns,
}

/// The `SkinBinding` struct supplies palette-local vertex joint mappings to CPU pose and GPU upload workflows.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SkinBinding {
	pub entries: Vec<SkinPaletteEntry>,
}

impl SkinBinding {
	/// Returns the number of matrices callers must reserve for this binding's GPU palette.
	pub fn len(&self) -> usize {
		self.entries.len()
	}

	/// Reports whether this binding has no addressable GPU palette entries.
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	/// Writes the final skin matrices into caller-owned storage without allocating intermediate palette data.
	pub fn write_matrix_palette(
		&self,
		global_pose: &[Matrix4Columns],
		output: &mut [Matrix4Columns],
	) -> Result<(), SkinPaletteError> {
		if output.len() != self.entries.len() {
			return Err(SkinPaletteError::OutputLength {
				expected: self.entries.len(),
				actual: output.len(),
			});
		}
		// Check every pose index first so a bad binding cannot leave a partially updated GPU upload buffer.
		for (palette_index, entry) in self.entries.iter().enumerate() {
			if let SkinJoint::Node(node) = entry.joint {
				if node as usize >= global_pose.len() {
					return Err(SkinPaletteError::NodeOutOfRange {
						palette_index,
						node,
						pose_len: global_pose.len(),
					});
				}
			}
		}

		for (entry, destination) in self.entries.iter().zip(output) {
			*destination = match entry.joint {
				SkinJoint::Node(node) => {
					multiply_matrix4_columns(&global_pose[node as usize], &entry.adjusted_inverse_bind_matrix)
				}
				SkinJoint::Identity => identity_matrix4_columns(),
			};
		}

		Ok(())
	}
}

/// Reports why a skin binding could not write a complete matrix palette.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkinPaletteError {
	OutputLength {
		expected: usize,
		actual: usize,
	},
	NodeOutOfRange {
		palette_index: usize,
		node: u32,
		pose_len: usize,
	},
}

impl std::fmt::Display for SkinPaletteError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::OutputLength { expected, actual } => write!(
				formatter,
				"Skin palette output has the wrong length. The most likely cause is caller storage for {actual} matrices when {expected} are required."
			),
			Self::NodeOutOfRange {
				palette_index,
				node,
				pose_len,
			} => write!(
				formatter,
				"Skin joint is outside the global pose. The most likely cause is palette entry {palette_index} referencing node {node} in a pose with {pose_len} nodes."
			),
		}
	}
}

impl std::error::Error for SkinPaletteError {}

/// Multiplies column-major matrices using the same convention as imported inverse-bind data.
fn multiply_matrix4_columns(left: &Matrix4Columns, right: &Matrix4Columns) -> Matrix4Columns {
	let mut product = [[0.0; 4]; 4];
	for column in 0..4 {
		for row in 0..4 {
			product[column][row] = (0..4).map(|index| left[index][row] * right[column][index]).sum();
		}
	}
	product
}

/// The `Skeleton` struct supplies the ordered rest hierarchy consumed by CPU animation evaluation.
#[derive(Debug, serde::Serialize)]
pub struct Skeleton {
	pub nodes: Vec<SkeletonNode>,
}

#[derive(Clone, Debug)]
/// The `SkeletonPoseMap` struct preserves a reusable source-to-target node mapping for compatible animation-pack rigs.
pub struct SkeletonPoseMap {
	target_by_source: Vec<Option<usize>>,
}

impl SkeletonPoseMap {
	/// Builds a mapping from stable authored node names while leaving target-only helpers on their rest pose.
	pub fn by_name(source: &Skeleton, target: &Skeleton) -> Self {
		let mut target_by_name = std::collections::HashMap::with_capacity(target.nodes.len());
		for (index, node) in target.nodes.iter().enumerate() {
			let Some(name) = node.name.as_deref() else {
				continue;
			};
			target_by_name
				.entry(name)
				.and_modify(|target| *target = None)
				.or_insert(Some(index));
		}
		Self {
			target_by_source: source
				.nodes
				.iter()
				.map(|node| {
					node.name
						.as_deref()
						.and_then(|name| target_by_name.get(name).copied().flatten())
				})
				.collect(),
		}
	}

	pub fn target_node(&self, source_node: usize) -> Option<usize> {
		self.target_by_source.get(source_node).copied().flatten()
	}

	/// Writes a complete target-local pose without allocating after caller storage reaches the target skeleton size.
	pub fn write_target_local_pose(
		&self,
		source_pose: &[LocalTransform],
		target: &Skeleton,
		output: &mut Vec<LocalTransform>,
	) -> Result<(), SkeletonPoseMapError> {
		if source_pose.len() != self.target_by_source.len() {
			return Err(SkeletonPoseMapError::SourcePoseLength {
				expected: self.target_by_source.len(),
				actual: source_pose.len(),
			});
		}

		output.clear();
		output.extend(target.nodes.iter().map(|node| node.rest_local));
		for (source, target) in source_pose.iter().zip(&self.target_by_source) {
			if let Some(target) = target {
				output[*target] = *source;
			}
		}
		Ok(())
	}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The `SkeletonPoseMapError` enum reports incompatible pose storage supplied to a retained skeleton mapping.
pub enum SkeletonPoseMapError {
	SourcePoseLength { expected: usize, actual: usize },
}

impl std::fmt::Display for SkeletonPoseMapError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::SourcePoseLength { expected, actual } => write!(
				formatter,
				"Source pose has the wrong node count. The most likely cause is that the pose map was built for {expected} nodes but received {actual}."
			),
		}
	}
}

impl std::error::Error for SkeletonPoseMapError {}

/// The `SkeletonModel` struct preserves a serializable skeleton hierarchy for resource storage and clip references.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SkeletonModel {
	pub nodes: Vec<SkeletonNode>,
}

impl Resource for Skeleton {
	type Model = SkeletonModel;

	fn get_class(&self) -> &'static str {
		"Skeleton"
	}
}

impl Model for SkeletonModel {
	fn get_class() -> &'static str {
		"Skeleton"
	}
}

impl<'de> Solver<'de, Skeleton> for SkeletonModel {
	fn solve(self, _storage_backend: &dyn resource::ReadStorageBackend) -> Result<Skeleton, SolveErrors> {
		validate_nodes(&self.nodes)?;
		Ok(Skeleton { nodes: self.nodes })
	}
}

impl<'de> Solver<'de, Reference<Skeleton>> for ReferenceModel<SkeletonModel> {
	/// Resolves a stored hierarchy for animation graphs after validating its serialized node model.
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Skeleton>, SolveErrors> {
		let (stored, reader) = storage_backend.read(self.id()).ok_or(SolveErrors::StorageError)?;
		let model: SkeletonModel = crate::from_slice(stored.resource()).map_err(|error| {
			SolveErrors::DeserializationFailed(format!(
				"Skeleton resource could not be deserialized. The most likely cause is incompatible or corrupted skeleton data: {error}."
			))
		})?;
		let skeleton = model.solve(storage_backend)?;
		Ok(Reference::from_model(self, skeleton, reader))
	}
}

/// Validates the parent-before-child ordering needed for allocation-free hierarchy evaluation.
pub(crate) fn validate_nodes(nodes: &[SkeletonNode]) -> Result<(), SolveErrors> {
	for (index, node) in nodes.iter().enumerate() {
		validate_node(
			index,
			node.parent,
			&node.rest_local.translation,
			&node.rest_local.rotation,
			&node.rest_local.scale,
		)?;
	}

	Ok(())
}

/// Validates a skeleton directly in its archived representation without allocating an owned node tree.
pub(crate) fn validate_archived_nodes(nodes: &[ArchivedSkeletonNode]) -> Result<(), SolveErrors> {
	for (index, node) in nodes.iter().enumerate() {
		let translation = node.rest_local.translation.map(|value| value.to_native());
		let rotation = node.rest_local.rotation.map(|value| value.to_native());
		let scale = node.rest_local.scale.map(|value| value.to_native());
		validate_node(
			index,
			node.parent.as_ref().map(|parent| parent.to_native()),
			&translation,
			&rotation,
			&scale,
		)?;
	}

	Ok(())
}

/// Validates one hierarchy and rest-pose entry shared by owned and archived skeleton resources.
fn validate_node(
	index: usize,
	parent: Option<u32>,
	translation: &[f32; 3],
	rotation: &[f32; 4],
	scale: &[f32; 3],
) -> Result<(), SolveErrors> {
	if parent.is_some_and(|parent| parent as usize >= index) {
		return Err(SolveErrors::DeserializationFailed(format!(
			"Skeleton hierarchy is invalid. The most likely cause is that node {index} references a parent that does not precede it."
		)));
	}
	if !translation.iter().chain(rotation).chain(scale).all(|value| value.is_finite()) {
		return Err(SolveErrors::DeserializationFailed(format!(
			"Skeleton rest pose is invalid. The most likely cause is that node {index} contains a non-finite local transform."
		)));
	}

	let rotation_length_squared = rotation.iter().map(|value| value * value).sum::<f32>();
	if (rotation_length_squared - 1.0).abs() > 1.0e-3 {
		return Err(SolveErrors::DeserializationFailed(format!(
			"Skeleton rest rotation is invalid. The most likely cause is that node {index} contains a zero-length or non-unit quaternion."
		)));
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{
		identity_matrix4_columns, LocalTransform, Skeleton, SkeletonModel, SkeletonNode, SkeletonPoseMap, SkinBinding,
		SkinJoint, SkinPaletteEntry, SkinPaletteError,
	};
	use crate::{resource::storage_backend::tests::TestStorageBackend, Solver};

	#[test]
	fn identity_transform_preserves_the_neutral_pose() {
		let transform = LocalTransform::identity();

		assert_eq!(transform.translation, [0.0, 0.0, 0.0]);
		assert_eq!(transform.rotation, [0.0, 0.0, 0.0, 1.0]);
		assert_eq!(transform.scale, [1.0, 1.0, 1.0]);
		assert_eq!(LocalTransform::default(), transform);
	}

	#[test]
	fn solving_preserves_parent_before_child_rest_pose() {
		let child_pose = LocalTransform {
			translation: [0.0, 2.0, 0.0],
			..LocalTransform::identity()
		};
		let model = SkeletonModel {
			nodes: vec![
				SkeletonNode {
					name: Some("root".into()),
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("child".into()),
					parent: Some(0),
					rest_local: child_pose,
				},
			],
		};

		let skeleton: Skeleton = model
			.solve(&TestStorageBackend::new())
			.expect("A parent-before-child skeleton should solve");
		assert_eq!(skeleton.nodes[1].parent, Some(0));
		assert_eq!(skeleton.nodes[1].rest_local, child_pose);
	}

	#[test]
	fn solving_rejects_forward_and_self_parent_references() {
		for parent in [0, 1] {
			let model = SkeletonModel {
				nodes: vec![SkeletonNode {
					name: None,
					parent: Some(parent),
					rest_local: LocalTransform::identity(),
				}],
			};

			assert!(model.solve(&TestStorageBackend::new()).is_err());
		}
	}

	#[test]
	fn solving_rejects_non_finite_and_non_unit_rest_transforms() {
		for rest_local in [
			LocalTransform {
				translation: [f32::NAN, 0.0, 0.0],
				..LocalTransform::identity()
			},
			LocalTransform {
				rotation: [0.0; 4],
				..LocalTransform::identity()
			},
			LocalTransform {
				rotation: [0.0, 0.0, 0.0, 2.0],
				..LocalTransform::identity()
			},
		] {
			let model = SkeletonModel {
				nodes: vec![SkeletonNode {
					name: None,
					parent: None,
					rest_local,
				}],
			};

			assert!(model.solve(&TestStorageBackend::new()).is_err());
		}
	}

	#[test]
	fn pose_map_matches_named_nodes_and_preserves_target_only_helpers() {
		let source = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: Some("Hips".into()),
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Spine".into()),
					parent: Some(0),
					rest_local: LocalTransform::identity(),
				},
			],
		};
		let helper_rest = LocalTransform {
			translation: [3.0, 0.0, 0.0],
			..LocalTransform::identity()
		};
		let target = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: Some("IKRoot".into()),
					parent: None,
					rest_local: helper_rest,
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Spine".into()),
					parent: Some(1),
					rest_local: LocalTransform::identity(),
				},
			],
		};
		let animated_hips = LocalTransform {
			translation: [0.0, 4.0, 0.0],
			..LocalTransform::identity()
		};
		let map = SkeletonPoseMap::by_name(&source, &target);
		let mut output = Vec::new();

		map.write_target_local_pose(&[animated_hips, LocalTransform::identity()], &target, &mut output)
			.unwrap();

		assert_eq!(map.target_node(0), Some(1));
		assert_eq!(map.target_node(1), Some(2));
		assert_eq!(output[0], helper_rest);
		assert_eq!(output[1], animated_hips);
	}

	#[test]
	fn skin_binding_stores_joint_and_inverse_bind_in_one_palette_entry() {
		let binding = SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint: SkinJoint::Node(0),
				adjusted_inverse_bind_matrix: identity_matrix4_columns(),
			}],
		};

		assert_eq!(binding.len(), 1);
		assert!(!binding.is_empty());
		assert_eq!(binding.entries[0].joint, SkinJoint::Node(0));
		assert_eq!(binding.entries[0].adjusted_inverse_bind_matrix, identity_matrix4_columns());
	}

	#[test]
	fn matrix_palette_multiplies_pose_and_inverse_bind_without_allocating_output() {
		let mut translated = identity_matrix4_columns();
		translated[3] = [5.0, 6.0, 7.0, 1.0];
		let mut inverse_bind = identity_matrix4_columns();
		inverse_bind[0][0] = 2.0;
		inverse_bind[1][1] = 3.0;
		inverse_bind[2][2] = 4.0;
		let binding = SkinBinding {
			entries: vec![
				SkinPaletteEntry {
					joint: SkinJoint::Node(0),
					adjusted_inverse_bind_matrix: inverse_bind,
				},
				SkinPaletteEntry {
					joint: SkinJoint::Identity,
					adjusted_inverse_bind_matrix: [[9.0; 4]; 4],
				},
			],
		};
		let mut output = [[[0.0; 4]; 4]; 2];

		binding
			.write_matrix_palette(&[translated], &mut output)
			.expect("A complete skin binding should write its palette");

		assert_eq!(output[0][0][0], 2.0);
		assert_eq!(output[0][1][1], 3.0);
		assert_eq!(output[0][2][2], 4.0);
		assert_eq!(output[0][3], [5.0, 6.0, 7.0, 1.0]);
		assert_eq!(output[1], identity_matrix4_columns());
	}

	#[test]
	fn matrix_palette_checks_output_and_pose_ranges() {
		let binding = SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint: SkinJoint::Node(1),
				adjusted_inverse_bind_matrix: identity_matrix4_columns(),
			}],
		};
		let mut no_output = [];
		assert_eq!(
			binding.write_matrix_palette(&[], &mut no_output),
			Err(SkinPaletteError::OutputLength { expected: 1, actual: 0 })
		);

		let mut output = [identity_matrix4_columns()];
		assert_eq!(
			binding.write_matrix_palette(&[identity_matrix4_columns()], &mut output),
			Err(SkinPaletteError::NodeOutOfRange {
				palette_index: 0,
				node: 1,
				pose_len: 1,
			})
		);

		let binding = SkinBinding {
			entries: vec![
				SkinPaletteEntry {
					joint: SkinJoint::Node(0),
					adjusted_inverse_bind_matrix: identity_matrix4_columns(),
				},
				SkinPaletteEntry {
					joint: SkinJoint::Node(2),
					adjusted_inverse_bind_matrix: identity_matrix4_columns(),
				},
			],
		};
		let sentinel = [[[-1.0; 4]; 4]; 2];
		let mut output = sentinel;
		assert!(binding
			.write_matrix_palette(&[identity_matrix4_columns()], &mut output)
			.is_err());
		assert_eq!(output, sentinel);
	}
}
