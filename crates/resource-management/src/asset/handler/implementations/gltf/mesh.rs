use super::*;

/// Maps a glTF attribute to the shared vertex stream it fills, or `None` for attributes the engine does not import.
pub(crate) fn gltf_vertex_component(semantic: gltf::Semantic) -> Option<VertexComponent> {
	let semantic = match semantic {
		gltf::Semantic::Positions => VertexSemantics::Position,
		gltf::Semantic::Normals => VertexSemantics::Normal,
		gltf::Semantic::Tangents => VertexSemantics::Tangent,
		gltf::Semantic::Colors(0) => VertexSemantics::Color,
		gltf::Semantic::TexCoords(0) => VertexSemantics::UV,
		gltf::Semantic::Joints(0) => VertexSemantics::Joints,
		gltf::Semantic::Weights(0) => VertexSemantics::Weights,
		_ => return None,
	};
	Some(VertexComponent::canonical(semantic))
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
	pub(crate) primitive: &'a gltf::Primitive<'a>,
	pub(crate) buffers: &'a [Cow<'a, [u8]>],
	pub(crate) material_slot: usize,
	pub(crate) transform: math::Matrix,
	pub(crate) transform_node: Option<u32>,
	pub(crate) skin: Option<u32>,
	pub(crate) skin_joint_count: Option<usize>,
	pub(crate) attributes: GltfPrimitiveAttributes,
}

impl<'a> GltfPrimitiveSource<'a> {
	fn reader(&self) -> gltf::mesh::Reader<'a, 'a, impl Clone + Fn(gltf::Buffer<'a>) -> Option<&'a [u8]>> {
		let buffers = self.buffers;
		self.primitive.reader(move |buffer| Some(&buffers[buffer.index()]))
	}
}

impl MeshPrimitiveSource for GltfPrimitiveSource<'_> {
	type Error = GltfImportError;

	fn material_slot(&self) -> usize {
		self.material_slot
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
			.ok_or(GltfImportError::MissingIndices)?
			.into_u32()
			.map(Ok))
	}

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<Point<ModelSpace>, Self::Error>> + '_, Self::Error> {
		let transform = self.transform;
		Ok(self
			.reader()
			.read_positions()
			.ok_or(GltfImportError::MissingPositions)?
			.map(move |position| {
				Ok(Point::from_maths(
					transform * Point::<ModelSpace>::from_array(position).into_maths(),
				))
			}))
	}

	fn normals(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
		if !self.attributes.normals {
			return Ok(None);
		}
		let normal_transform = gltf_normal_transform(self.transform)?;
		let normals = self
			.reader()
			.read_normals()
			.ok_or(GltfImportError::MissingAttribute(VertexSemantics::Normal))?;
		Ok(Some(
			normals.map(move |normal| transform_gltf_unit_direction(&normal_transform, normal)),
		))
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
			.ok_or(GltfImportError::MissingAttribute(VertexSemantics::Tangent))?;
		Ok(Some(tangents.map(move |tangent| {
			transform_gltf_tangent(&transform, orientation, tangent)
		})))
	}

	fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Self::Error>> + '_>, Self::Error> {
		if !self.attributes.uvs {
			return Ok(None);
		}
		let uvs = self
			.reader()
			.read_tex_coords(0)
			.ok_or(GltfImportError::MissingAttribute(VertexSemantics::UV))?;
		Ok(Some(uvs.into_f32().map(Ok)))
	}

	fn colors(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.attributes.colors {
			return Ok(None);
		}
		let colors = self
			.reader()
			.read_colors(0)
			.ok_or(GltfImportError::MissingAttribute(VertexSemantics::Color))?;
		Ok(Some(colors.into_rgba_f32().map(Ok)))
	}

	fn vertex_skin(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<VertexSkin, Self::Error>> + '_>, Self::Error> {
		let Some(joint_count) = self.skin_joint_count else {
			return Ok(None);
		};
		let reader = self.reader();
		let vertex_count = reader.read_positions().ok_or(GltfImportError::MissingPositions)?.len();
		Ok(Some(gltf_vertex_skin(&reader, vertex_count, joint_count)?))
	}
}
