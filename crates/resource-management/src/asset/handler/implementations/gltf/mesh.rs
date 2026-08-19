use super::*;

pub(crate) fn gltf_vertex_component(semantic: gltf::Semantic) -> Option<VertexComponent> {
	match semantic {
		gltf::Semantic::Positions => Some(VertexComponent {
			semantic: VertexSemantics::Position,
			format: "vec3f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Normals => Some(VertexComponent {
			semantic: VertexSemantics::Normal,
			format: "vec3f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Tangents => Some(VertexComponent {
			semantic: VertexSemantics::Tangent,
			format: "vec4f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Colors(0) => Some(VertexComponent {
			semantic: VertexSemantics::Color,
			format: "vec4f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::TexCoords(0) => Some(VertexComponent {
			semantic: VertexSemantics::UV,
			format: "vec2f".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Joints(0) => Some(VertexComponent {
			semantic: VertexSemantics::Joints,
			format: "vec4u16".to_string(),
			channel: 0,
		}),
		gltf::Semantic::Weights(0) => Some(VertexComponent {
			semantic: VertexSemantics::Weights,
			format: "vec4f".to_string(),
			channel: 0,
		}),
		_ => None,
	}
}

pub(crate) fn normalize_vertex_layouts(vertex_layouts: &[Vec<VertexComponent>]) -> Vec<VertexComponent> {
	let Some(first_layout) = vertex_layouts.first() else {
		return Vec::new();
	};

	first_layout
		.iter()
		.filter(|component| component.semantic != VertexSemantics::BiTangent)
		.filter(|component| {
			vertex_layouts
				.iter()
				.all(|layout| layout.iter().any(|candidate| candidate == *component))
		})
		.cloned()
		.collect()
}

pub(crate) fn has_vertex_component(vertex_layout: &[VertexComponent], semantic: VertexSemantics, channel: u32) -> bool {
	vertex_layout
		.iter()
		.any(|component| component.semantic == semantic && component.channel == channel)
}

/// The `GltfMeshSourceError` enum identifies glTF data that cannot supply a canonical processor stream.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GltfMeshSourceError {
	MissingIndices,
	MissingPositions,
	MissingAttribute(VertexSemantics),
	Skeletal(GltfSkeletalImportError),
}

impl std::fmt::Display for GltfMeshSourceError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::MissingIndices => write!(formatter, "glTF triangle indices are missing. The most likely cause is an unindexed source primitive."),
			Self::MissingPositions => write!(formatter, "glTF positions are missing. The most likely cause is a missing or malformed POSITION accessor."),
			Self::MissingAttribute(semantic) => write!(formatter, "glTF vertex data is incomplete. The most likely cause is a missing {semantic:?} accessor required by the shared mesh layout."),
			Self::Skeletal(error) => error.fmt(formatter),
		}
	}
}

impl std::error::Error for GltfMeshSourceError {}

impl From<GltfSkeletalImportError> for GltfMeshSourceError {
	fn from(error: GltfSkeletalImportError) -> Self {
		Self::Skeletal(error)
	}
}

/// The `GltfPrimitiveAttributes` struct records which shared vertex streams each glTF primitive should expose.
#[derive(Clone, Copy)]
pub(crate) struct GltfPrimitiveAttributes {
	normals: bool,
	tangents: bool,
	uvs: bool,
	colors: bool,
}

impl GltfPrimitiveAttributes {
	pub(crate) fn from_layout(vertex_layout: &[VertexComponent]) -> Self {
		Self {
			normals: has_vertex_component(vertex_layout, VertexSemantics::Normal, 0),
			tangents: has_vertex_component(vertex_layout, VertexSemantics::Tangent, 0),
			uvs: has_vertex_component(vertex_layout, VertexSemantics::UV, 0),
			colors: has_vertex_component(vertex_layout, VertexSemantics::Color, 0),
		}
	}
}

/// The `GltfPrimitiveSource` struct lends one glTF primitive and its accessor data to the common mesh processor.
pub(crate) struct GltfPrimitiveSource<'a> {
	primitive: &'a gltf::Primitive<'a>,
	buffers: &'a [gltf::buffer::Data],
	material: &'a ReferenceModel<VariantModel>,
	transform: maths_rs::Mat4f,
	transform_node: Option<u32>,
	skin: Option<u32>,
	skin_joint_count: Option<usize>,
	attributes: GltfPrimitiveAttributes,
}

impl<'a> GltfPrimitiveSource<'a> {
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn new(
		primitive: &'a gltf::Primitive<'a>,
		buffers: &'a [gltf::buffer::Data],
		material: &'a ReferenceModel<VariantModel>,
		transform: maths_rs::Mat4f,
		transform_node: Option<u32>,
		skin: Option<u32>,
		skin_joint_count: Option<usize>,
		attributes: GltfPrimitiveAttributes,
	) -> Self {
		Self {
			primitive,
			buffers,
			material,
			transform,
			transform_node,
			skin,
			skin_joint_count,
			attributes,
		}
	}

	fn reader(&self) -> gltf::mesh::Reader<'a, 'a, impl Clone + Fn(gltf::Buffer<'a>) -> Option<&'a [u8]>> {
		let buffers = self.buffers;
		self.primitive.reader(move |buffer| Some(&buffers[buffer.index()]))
	}
}

impl MeshPrimitiveSource for GltfPrimitiveSource<'_> {
	type Error = GltfMeshSourceError;

	fn material(&self) -> &ReferenceModel<VariantModel> {
		self.material
	}

	fn transform_node(&self) -> Option<u32> {
		self.transform_node
	}

	fn skin(&self) -> Option<u32> {
		self.skin
	}

	fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error> {
		Ok(self
			.reader()
			.read_indices()
			.ok_or(GltfMeshSourceError::MissingIndices)?
			.into_u32()
			.map(Ok))
	}

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_, Self::Error> {
		let transform = self.transform;
		Ok(self
			.reader()
			.read_positions()
			.ok_or(GltfMeshSourceError::MissingPositions)?
			.map(move |position| {
				let transformed = transform * maths_rs::Vec3f::new(position[0], position[1], position[2]);
				Ok([transformed[0], transformed[1], transformed[2]])
			}))
	}

	fn normals(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_>, Self::Error> {
		if !self.attributes.normals {
			return Ok(None);
		}
		let normal_transform = gltf_normal_transform(self.transform)?;
		let normals = self
			.reader()
			.read_normals()
			.ok_or(GltfMeshSourceError::MissingAttribute(VertexSemantics::Normal))?;
		Ok(Some(normals.map(move |normal| {
			transform_gltf_unit_direction(&normal_transform, normal).map_err(Into::into)
		})))
	}

	fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.attributes.tangents {
			return Ok(None);
		}
		let transform = self.transform;
		let orientation = gltf_transform_orientation(transform)?;
		let tangents = self
			.reader()
			.read_tangents()
			.ok_or(GltfMeshSourceError::MissingAttribute(VertexSemantics::Tangent))?;
		Ok(Some(tangents.map(move |tangent| {
			transform_gltf_tangent(&transform, orientation, tangent).map_err(Into::into)
		})))
	}

	fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Self::Error>> + '_>, Self::Error> {
		if !self.attributes.uvs {
			return Ok(None);
		}
		let uvs = self
			.reader()
			.read_tex_coords(0)
			.ok_or(GltfMeshSourceError::MissingAttribute(VertexSemantics::UV))?;
		Ok(Some(uvs.into_f32().map(Ok)))
	}

	fn colors(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.attributes.colors {
			return Ok(None);
		}
		let colors = self
			.reader()
			.read_colors(0)
			.ok_or(GltfMeshSourceError::MissingAttribute(VertexSemantics::Color))?;
		Ok(Some(colors.into_rgba_f32().map(Ok)))
	}

	fn vertex_skin(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<VertexSkin, Self::Error>> + '_>, Self::Error> {
		let Some(joint_count) = self.skin_joint_count else {
			return Ok(None);
		};
		let reader = self.reader();
		let vertex_count = reader.read_positions().ok_or(GltfMeshSourceError::MissingPositions)?.len();
		Ok(Some(
			GltfVertexSkinIterator::new(&reader, vertex_count, joint_count)?.map(|value| value.map_err(Into::into)),
		))
	}
}
