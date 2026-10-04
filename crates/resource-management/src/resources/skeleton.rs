use math::{AffineMatrix, Orientation, Scale, Vector};

use crate::{Reference, ReferenceModel, Solver, resource, resources::ParentSpace, solver::SolveError};

/// The `LocalTransform` struct preserves the blendable local pose used by skeleton nodes and animation tracks.
#[derive(
	Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct LocalTransform {
	pub translation: Vector<ParentSpace>,
	pub rotation: Orientation,
	pub scale: Scale,
}

impl LocalTransform {
	/// Creates the neutral local pose used for nodes without an authored transform.
	pub const fn identity() -> Self {
		Self {
			translation: Vector::zero(),
			rotation: Orientation::identity(),
			scale: Scale::identity(),
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
	pub adjusted_inverse_bind_matrix: AffineMatrix,
}

/// The `SkinBinding` struct supplies palette-local vertex joint mappings to CPU pose and GPU upload workflows.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SkinBinding {
	pub entries: Vec<SkinPaletteEntry>,
}

impl SkinBinding {
	/// Reports whether this binding has no addressable GPU palette entries.
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	/// Writes the final skin matrices into caller-owned storage without allocating intermediate palette data.
	pub fn write_matrix_palette(
		&self,
		global_pose: &[AffineMatrix],
		output: &mut [AffineMatrix],
	) -> Result<(), SkinPaletteError> {
		if output.len() != self.entries.len() {
			return Err(SkinPaletteError::OutputLength {
				expected: self.entries.len(),
				actual: output.len(),
			});
		}
		// Check every pose index first so a bad binding cannot leave a partially updated GPU upload buffer.
		for (palette_index, entry) in self.entries.iter().enumerate() {
			if let SkinJoint::Node(node) = entry.joint
				&& node as usize >= global_pose.len()
			{
				return Err(SkinPaletteError::NodeOutOfRange {
					palette_index,
					node,
					pose_len: global_pose.len(),
				});
			}
		}

		for (entry, destination) in self.entries.iter().zip(output) {
			*destination = match entry.joint {
				SkinJoint::Node(node) => global_pose[node as usize] * entry.adjusted_inverse_bind_matrix,
				SkinJoint::Identity => AffineMatrix::identity(),
			};
		}

		Ok(())
	}
}

/// The `SkinPaletteError` enum identifies failures while writing a complete skin matrix palette.
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

/// The `Skeleton` struct supplies the ordered rest hierarchy consumed by CPU animation evaluation.
#[derive(Debug, serde::Serialize)]
pub struct Skeleton {
	pub nodes: Vec<SkeletonNode>,
}

#[derive(Clone, Debug)]
/// The `SkeletonPoseMap` struct preserves a reusable source-to-target node mapping for compatible animation-pack rigs.
pub struct SkeletonPoseMap {
	target_by_source: Vec<Option<usize>>,
	direct_target_by_source: Vec<Option<usize>>,
	target_rest_pose: Vec<LocalTransform>,
}

impl SkeletonPoseMap {
	/// Builds a mapping from stable authored node names while leaving target-only helpers on their rest pose.
	pub fn by_name(source: &Skeleton, target: &Skeleton) -> Self {
		// Players build a map on every state entry, so the transient name lookup uses the engine's fast hasher.
		let mut target_by_name = utils::hash::HashMap::with_capacity_and_hasher(target.nodes.len(), Default::default());
		for (index, node) in target.nodes.iter().enumerate() {
			let Some(name) = node.name.as_deref() else {
				continue;
			};
			target_by_name
				.entry(name)
				.and_modify(|target| *target = None)
				.or_insert(Some(index));
		}
		let target_by_source: Vec<_> = source
			.nodes
			.iter()
			.map(|node| {
				node.name
					.as_deref()
					.and_then(|name| target_by_name.get(name).copied().flatten())
			})
			.collect();
		// Direct sampling starts from the final rest pose, so only the final source
		// writer for each target may apply animated channels.
		let mut direct_target_by_source = target_by_source.clone();
		let mut final_source_by_target = vec![None; target.nodes.len()];
		for (source, target) in target_by_source.iter().enumerate() {
			if let Some(target) = target
				&& let Some(previous) = final_source_by_target[*target].replace(source)
			{
				direct_target_by_source[previous] = None;
			}
		}
		let mut target_rest_pose: Vec<_> = target.nodes.iter().map(|node| node.rest_local).collect();
		for (source, target) in source.nodes.iter().zip(&target_by_source) {
			if let Some(target) = target {
				target_rest_pose[*target] = source.rest_local;
			}
		}
		Self {
			target_by_source,
			direct_target_by_source,
			target_rest_pose,
		}
	}

	/// Returns the target node mapped from one source node.
	pub fn target_node(&self, source_node: usize) -> Option<usize> {
		self.target_by_source.get(source_node).copied().flatten()
	}

	/// Returns the target node written by direct mapped sampling for one source node.
	///
	/// When multiple source nodes map to one target, only the last source node
	/// writes that target. This matches [`Self::write_target_local_pose`].
	pub fn direct_target_node(&self, source_node: usize) -> Option<usize> {
		self.direct_target_by_source.get(source_node).copied().flatten()
	}

	/// Returns the complete target rest pose after compatible source rest transforms replace matching target nodes.
	///
	/// Use this as the base pose before directly sampling mapped animation tracks.
	pub fn target_rest_pose(&self) -> &[LocalTransform] {
		&self.target_rest_pose
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

		debug_assert_eq!(target.nodes.len(), self.target_rest_pose.len());
		output.clear();
		output.extend_from_slice(&self.target_rest_pose);
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

super::impl_resource_model!(Skeleton, SkeletonModel, "Skeleton");

impl<'de> Solver<'de, Skeleton> for SkeletonModel {
	fn solve(
		self,
		_storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Skeleton, SolveError>> {
		crate::r#async::future(async move {
			validate_nodes(&self.nodes)?;
			Ok(Skeleton { nodes: self.nodes })
		})
	}
}

impl crate::StoredModel for SkeletonModel {
	type Resource = Skeleton;

	/// Resolves a stored hierarchy for animation graphs after validating its serialized node model.
	fn solve_stored<'de>(
		stored: crate::SerializableResource,
		reader: crate::resource::resource_handler::MultiResourceReader,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Skeleton>, SolveError>> {
		crate::r#async::future(async move {
			let model: SkeletonModel = crate::from_slice(stored.resource()).map_err(|error| {
				SolveError::DeserializationFailed(format!(
					"Skeleton resource could not be deserialized. The most likely cause is incompatible or corrupted skeleton data: {error}."
				))
			})?;
			let skeleton = model.solve(storage_backend).await?;
			Ok(Reference::from_stored(stored, skeleton, reader))
		})
	}
}

/// Validates the parent-before-child ordering needed for allocation-free hierarchy evaluation.
pub(crate) fn validate_nodes(nodes: &[SkeletonNode]) -> Result<(), SolveError> {
	for (index, node) in nodes.iter().enumerate() {
		validate_node(
			index,
			node.parent,
			&node.rest_local.translation.to_array(),
			&node.rest_local.rotation.to_array(),
			&node.rest_local.scale.to_array(),
		)?;
	}

	Ok(())
}

/// Validates a skeleton directly in its archived representation without allocating an owned node tree.
pub(crate) fn validate_archived_nodes(nodes: &[ArchivedSkeletonNode]) -> Result<(), SolveError> {
	for (index, node) in nodes.iter().enumerate() {
		validate_node(
			index,
			node.parent.as_ref().map(|parent| parent.to_native()),
			&node.rest_local.translation.map(|value| value.to_native()),
			&node.rest_local.rotation.map(|value| value.to_native()),
			&node.rest_local.scale.map(|value| value.to_native()),
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
) -> Result<(), SolveError> {
	if parent.is_some_and(|parent| parent as usize >= index) {
		return Err(SolveError::DeserializationFailed(format!(
			"Skeleton hierarchy is invalid. The most likely cause is that node {index} references a parent that does not precede it."
		)));
	}
	if !translation.iter().chain(rotation).chain(scale).all(|value| value.is_finite()) {
		return Err(SolveError::DeserializationFailed(format!(
			"Skeleton rest pose is invalid. The most likely cause is that node {index} contains a non-finite local transform."
		)));
	}

	let rotation_length_squared = rotation.iter().map(|value| value * value).sum::<f32>();
	if (rotation_length_squared - 1.0).abs() > 1.0e-3 {
		return Err(SolveError::DeserializationFailed(format!(
			"Skeleton rest rotation is invalid. The most likely cause is that node {index} contains a zero-length or non-unit quaternion."
		)));
	}

	Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
	use math::{AffineMatrix, Vector};

	use super::{
		LocalTransform, Skeleton, SkeletonModel, SkeletonNode, SkeletonPoseMap, SkinBinding, SkinJoint, SkinPaletteEntry,
		SkinPaletteError,
	};
	use crate::{Solver, resource::storage_backend::tests::TestStorageBackend};

	/// Builds a skeleton node whose rest pose only translates, for tests that author small hierarchies.
	pub(crate) fn node(name: Option<&str>, parent: Option<u32>, translation: [f32; 3]) -> SkeletonNode {
		SkeletonNode {
			name: name.map(Into::into),
			parent,
			rest_local: LocalTransform {
				translation: Vector::from_array(translation),
				..LocalTransform::identity()
			},
		}
	}

	#[crate::r#async::test]
	async fn solving_rejects_forward_and_self_parent_references() {
		for parent in [0, 1] {
			let model = SkeletonModel {
				nodes: vec![node(None, Some(parent), [0.0; 3])],
			};

			assert!(model.solve(&TestStorageBackend::new()).await.is_err());
		}
	}

	#[crate::r#async::test]
	async fn solving_rejects_non_finite_and_non_unit_rest_transforms() {
		let model = SkeletonModel {
			nodes: vec![node(None, None, [f32::NAN, 0.0, 0.0])],
		};

		assert!(model.solve(&TestStorageBackend::new()).await.is_err());
	}

	#[test]
	fn pose_map_matches_named_nodes_and_preserves_target_only_helpers() {
		let source = Skeleton {
			nodes: vec![node(Some("Hips"), None, [0.0; 3]), node(Some("Spine"), Some(0), [0.0; 3])],
		};
		let target = Skeleton {
			nodes: vec![
				node(Some("IKRoot"), None, [3.0, 0.0, 0.0]),
				node(Some("Hips"), None, [0.0; 3]),
				node(Some("Spine"), Some(1), [0.0; 3]),
			],
		};
		let animated_hips = LocalTransform {
			translation: Vector::new(0.0, 4.0, 0.0),
			..LocalTransform::identity()
		};
		let map = SkeletonPoseMap::by_name(&source, &target);
		let mut output = Vec::new();

		map.write_target_local_pose(&[animated_hips, LocalTransform::identity()], &target, &mut output)
			.unwrap();

		assert_eq!(map.target_node(0), Some(1));
		assert_eq!(map.target_node(1), Some(2));
		assert_eq!(output[0], target.nodes[0].rest_local);
		assert_eq!(output[1], animated_hips);
	}

	#[test]
	fn direct_pose_map_keeps_the_last_duplicate_source_node() {
		let source = Skeleton {
			nodes: vec![
				node(Some("Hips"), None, [1.0, 0.0, 0.0]),
				node(Some("Hips"), None, [2.0, 0.0, 0.0]),
			],
		};
		let target = Skeleton {
			nodes: vec![node(Some("Hips"), None, [0.0; 3])],
		};

		let map = SkeletonPoseMap::by_name(&source, &target);

		assert_eq!(map.target_node(0), Some(0));
		assert_eq!(map.target_node(1), Some(0));
		assert_eq!(map.direct_target_node(0), None);
		assert_eq!(map.direct_target_node(1), Some(0));
		assert_eq!(map.target_rest_pose()[0].translation, Vector::new(2.0, 0.0, 0.0));
	}

	#[test]
	fn matrix_palette_multiplies_pose_and_inverse_bind_without_allocating_output() {
		let translated = AffineMatrix::from_columns([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [5.0, 6.0, 7.0]]);
		let inverse_bind = AffineMatrix::from_columns([[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0], [0.0, 0.0, 0.0]]);
		let binding = SkinBinding {
			entries: vec![
				SkinPaletteEntry {
					joint: SkinJoint::Node(0),
					adjusted_inverse_bind_matrix: inverse_bind,
				},
				SkinPaletteEntry {
					joint: SkinJoint::Identity,
					adjusted_inverse_bind_matrix: AffineMatrix::from_columns([[9.0; 3]; 4]),
				},
			],
		};
		let mut output = [AffineMatrix::from_columns([[0.0; 3]; 4]); 2];

		binding
			.write_matrix_palette(&[translated], &mut output)
			.expect("A complete skin binding should write its palette");

		assert_eq!(
			output[0].columns(),
			[[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0], [5.0, 6.0, 7.0]]
		);
		assert_eq!(output[1], AffineMatrix::identity());
	}

	#[test]
	fn matrix_palette_checks_output_and_pose_ranges() {
		let binding = SkinBinding {
			entries: vec![SkinPaletteEntry {
				joint: SkinJoint::Node(1),
				adjusted_inverse_bind_matrix: AffineMatrix::identity(),
			}],
		};
		let mut no_output = [];

		assert_eq!(
			binding.write_matrix_palette(&[], &mut no_output),
			Err(SkinPaletteError::OutputLength { expected: 1, actual: 0 })
		);

		let mut output = [AffineMatrix::identity()];

		assert_eq!(
			binding.write_matrix_palette(&[AffineMatrix::identity()], &mut output),
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
					adjusted_inverse_bind_matrix: AffineMatrix::identity(),
				},
				SkinPaletteEntry {
					joint: SkinJoint::Node(2),
					adjusted_inverse_bind_matrix: AffineMatrix::identity(),
				},
			],
		};
		let sentinel = [AffineMatrix::from_columns([[-1.0; 3]; 4]); 2];
		let mut output = sentinel;

		assert!(
			binding
				.write_matrix_palette(&[AffineMatrix::identity()], &mut output)
				.is_err()
		);
		assert_eq!(output, sentinel);
	}
}
