use super::*;
#[derive(Debug, PartialEq, Eq)]
/// The `FbxImportError` enum identifies malformed or unsupported FBX content at the importer boundary.
pub(crate) enum FbxImportError {
	Parse(String),
	AnimationBake(String),
	AnimationNotFound(String),
	UnsupportedFragment(String),
	NoMesh,
	MissingMaterial,
	InvalidFaceIndex,
	InvalidCornerIndex,
	InvalidTriangleCount,
	TriangulationOverflow,
	EmptyPrimitive,
	InvalidSkinVertex,
	InvalidSkinCluster,
	MissingSkinBone,
	MissingFallbackJoint,
	TooManyJoints,
	TooManySkinBindings,
	MultipleSkinDeformers,
	UnsupportedBlendedDualQuaternionSkinning,
	NonInvertibleSkinTransform,
	NonInvertibleAnimatedMeshTransform,
	InvalidSkeletonNode,
	DuplicateSkeletonNode,
	IncompleteSkeleton,
	TooManySkeletonNodes,
	ZeroDirection,
	NonFinite(&'static str),
}

impl fmt::Display for FbxImportError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Parse(description) => write!(
				formatter,
				"FBX parsing failed. The most likely cause is malformed or unsupported FBX data: {description}"
			),
			Self::AnimationBake(description) => write!(
				formatter,
				"FBX animation baking failed. The most likely cause is malformed animation curves or unsupported layer data: {description}"
			),
			Self::AnimationNotFound(description) => write!(
				formatter,
				"FBX animation was not found. The most likely cause is an incorrect animation fragment: {description}"
			),
			Self::UnsupportedFragment(fragment) => write!(
				formatter,
				"FBX fragment is unsupported. The most likely cause is that '{fragment}' does not use `skeleton`, `animation`, or `animations/<index-or-name>`."
			),
			Self::NoMesh => write!(
				formatter,
				"FBX mesh is empty. The most likely cause is that the file contains no polygon mesh instances."
			),
			Self::MissingMaterial => write!(
				formatter,
				"FBX material resolution failed. The most likely cause is inconsistent material metadata in the imported scene."
			),
			Self::InvalidFaceIndex => write!(
				formatter,
				"FBX face index is invalid. The most likely cause is a malformed material part referencing a missing face."
			),
			Self::InvalidCornerIndex => write!(
				formatter,
				"FBX vertex-corner index is invalid. The most likely cause is malformed polygon topology."
			),
			Self::InvalidTriangleCount => write!(
				formatter,
				"FBX triangle index count is invalid. The most likely cause is incomplete triangulation output."
			),
			Self::TriangulationOverflow => write!(
				formatter,
				"FBX triangulation exceeded its scratch buffer. The most likely cause is inconsistent maximum-face metadata."
			),
			Self::EmptyPrimitive => write!(
				formatter,
				"FBX primitive has no vertices. The most likely cause is an empty or degenerate material part."
			),
			Self::InvalidSkinVertex => write!(
				formatter,
				"FBX skin vertex is invalid. The most likely cause is skin weights that do not match the mesh control vertices."
			),
			Self::InvalidSkinCluster => write!(
				formatter,
				"FBX skin cluster is invalid. The most likely cause is a weight referencing a missing joint palette entry."
			),
			Self::MissingSkinBone => write!(
				formatter,
				"FBX skin cluster has no bone. The most likely cause is a broken cluster-to-node connection."
			),
			Self::MissingFallbackJoint => write!(
				formatter,
				"FBX fallback joint is missing. The most likely cause is an unweighted vertex without its required mesh-node palette entry."
			),
			Self::TooManyJoints => write!(
				formatter,
				"FBX skin has too many joints. The most likely cause is a joint palette larger than the engine's u16 joint stream."
			),
			Self::TooManySkinBindings => write!(
				formatter,
				"FBX has too many skin bindings. The most likely cause is more skinned mesh instances than the resource format can index."
			),
			Self::MultipleSkinDeformers => write!(
				formatter,
				"FBX mesh has multiple skin deformers. The most likely cause is layered skinning that cannot be represented by one matrix palette."
			),
			Self::UnsupportedBlendedDualQuaternionSkinning => write!(
				formatter,
				"FBX blended dual-quaternion skinning is unsupported. The most likely cause is a blended DQ/linear deformer or per-vertex DQ blend weights authored on the mesh."
			),
			Self::NonInvertibleSkinTransform => write!(
				formatter,
				"FBX skin transform is not invertible. The most likely cause is a zero-scale skinned mesh instance."
			),
			Self::NonInvertibleAnimatedMeshTransform => write!(
				formatter,
				"FBX animated mesh transform is not invertible. The most likely cause is a zero bind scale that cannot be recovered after flattening geometry."
			),
			Self::InvalidSkeletonNode => write!(
				formatter,
				"FBX skeleton node is invalid. The most likely cause is a node ID outside the imported scene hierarchy."
			),
			Self::DuplicateSkeletonNode => write!(
				formatter,
				"FBX skeleton node is duplicated. The most likely cause is an inconsistent node hierarchy containing the same child twice."
			),
			Self::IncompleteSkeleton => write!(
				formatter,
				"FBX skeleton hierarchy is incomplete. The most likely cause is a scene node disconnected from the imported root."
			),
			Self::TooManySkeletonNodes => write!(
				formatter,
				"FBX skeleton has too many nodes. The most likely cause is a hierarchy larger than the resource's u32 node indices."
			),
			Self::ZeroDirection => write!(
				formatter,
				"FBX direction vector is zero. The most likely cause is malformed normal or tangent data."
			),
			Self::NonFinite(context) => write!(
				formatter,
				"FBX numeric value is invalid. The most likely cause is a non-finite or out-of-range {context}."
			),
		}
	}
}

impl std::error::Error for FbxImportError {}
