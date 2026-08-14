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
