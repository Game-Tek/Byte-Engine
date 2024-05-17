use std::{cell::RefCell, rc::Rc};

use resource_management::asset::material_asset_handler::ProgramGenerator;

use crate::besl::lexer;

pub struct CommonShaderGenerator {
	mesh_struct: besl::parser::Node,
	camera_struct: besl::parser::Node,
	meshlet_struct: besl::parser::Node,
	light_struct: besl::parser::Node,
	material_struct: besl::parser::Node,
	meshes: besl::parser::Node,
	positions: besl::parser::Node,
	normals: besl::parser::Node,
	tangents: besl::parser::Node,
	uvs: besl::parser::Node,
	vertex_indices: besl::parser::Node,
	primitive_indices: besl::parser::Node,
	meshlets: besl::parser::Node,
	textures: besl::parser::Node,
	material_count: besl::parser::Node,
	material_offset: besl::parser::Node,
	pixel_mapping: besl::parser::Node,
	triangle_index: besl::parser::Node,
}

impl ProgramGenerator for CommonShaderGenerator {
	fn transform(&self, mut root: besl::parser::Node, _: &json::JsonValue) -> besl::parser::Node {
		let code = "vec4 get_debug_color(uint i) {
vec4 colors[16] = vec4[16](
	vec4(0.16863, 0.40392, 0.77647, 1),
	vec4(0.32941, 0.76863, 0.21961, 1),
	vec4(0.81961, 0.16078, 0.67451, 1),
	vec4(0.96863, 0.98824, 0.45490, 1),
	vec4(0.75294, 0.09020, 0.75686, 1),
	vec4(0.30588, 0.95686, 0.54510, 1),
	vec4(0.66667, 0.06667, 0.75686, 1),
	vec4(0.78824, 0.91765, 0.27451, 1),
	vec4(0.40980, 0.12745, 0.48627, 1),
	vec4(0.89804, 0.28235, 0.20784, 1),
	vec4(0.93725, 0.67843, 0.33725, 1),
	vec4(0.95294, 0.96863, 0.00392, 1),
	vec4(1.00000, 0.27843, 0.67843, 1),
	vec4(0.29020, 0.90980, 0.56863, 1),
	vec4(0.30980, 0.70980, 0.27059, 1),
	vec4(0.69804, 0.16078, 0.39216, 1)
);

return colors[i % 16];
}";
		let mesh_struct = self.mesh_struct.clone();
		let camera_struct = self.camera_struct.clone();
		let meshlet_struct = self.meshlet_struct.clone();
		let light_struct = self.light_struct.clone();
		let material_struct = self.material_struct.clone();

		let material_offset = self.material_offset.clone();
		let meshes = self.meshes.clone();
		let material_count = self.material_count.clone();
		let uvs = self.uvs.clone();
		let textures = self.textures.clone();
		let pixel_mapping = self.pixel_mapping.clone();
		let triangle_index = self.triangle_index.clone();
		let meshlets = self.meshlets.clone();
		let primitive_indices = self.primitive_indices.clone();
		let vertex_indices = self.vertex_indices.clone();
		let positions = self.positions.clone();
		let normals = self.normals.clone();

		root.add(vec![mesh_struct, camera_struct, meshlet_struct, light_struct, material_offset, meshes, material_count, uvs, textures, pixel_mapping, triangle_index, meshlets, primitive_indices, vertex_indices, positions, normals, material_struct]);
		root.add(vec![besl::parser::Node::glsl(code, Vec::new(), Vec::new()).into()]);

		root
	}
}

impl CommonShaderGenerator {
	pub const SCOPE: &'static str = "Common";

	pub fn new() -> Self {
		use besl::parser::Node;

		let mesh_struct = Node::r#struct("Mesh", vec![Node::member("model", "mat4f"), Node::member("material_index", "u32"), Node::member("base_vertex_index", "u32")]);
		let camera_struct = Node::r#struct("Camera", vec![Node::member("view", "mat4f"), Node::member("projection_matrix", "mat4f"), Node::member("view_projection", "mat4f"), Node::member("inverse_view_matrix", "mat4f"), Node::member("inverse_projection_matrix", "mat4f"), Node::member("inverse_view_projection_matrix", "mat4f")]);
		let meshlet_struct = Node::r#struct("Meshlet", vec![Node::member("instance_index", "u32"), Node::member("vertex_offset", "u16"), Node::member("triangle_offset", "u16"), Node::member("vertex_count", "u8"), Node::member("triangle_count", "u8")]);
		let light_struct = Node::r#struct("Light", vec![Node::member("view_matrix", "mat4f"), Node::member("projection_matrix", "mat4f"), Node::member("vp_matrix", "mat4f"), Node::member("position", "vec3f"), Node::member("color", "vec3f"), Node::member("light_type", "u8")]);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", "u32[16]")]);

		let meshes = Node::binding("meshes", Node::buffer("MeshBuffer", vec![Node::member("meshes", "Mesh[64]")]), 0, 1, true, false);
		let position = Node::binding("positions", Node::buffer("Positions", vec![Node::member("positions", "vec3f[8192]")]), 0, 2, true, false);
		let normals = Node::binding("normals", Node::buffer("Normals", vec![Node::member("normals", "vec3f[8192]")]), 0, 3, true, false);
		let tangents = Node::binding("tangents", Node::buffer("Tangents", vec![Node::member("tangents", "vec3f[8192]")]), 0, 4, true, false);
		let uvs = Node::binding("uvs", Node::buffer("UVs", vec![Node::member("uvs", "vec2f[8192]")]), 0, 5, true, false);
		let set0_binding4 = Node::binding("vertex_indices", Node::buffer("VertexIndices", vec![Node::member("vertex_indices", "u16[8192]")]), 0, 6, true, false);
		let set0_binding5 = Node::binding("primitive_indices", Node::buffer("PrimitiveIndices", vec![Node::member("primitive_indices", "u8[8192]")]), 0, 7, true, false);
		let meshlets = Node::binding("meshlets", Node::buffer("MeshletsBuffer", vec![Node::member("meshlets", "Meshlet[8192]")]), 0, 8, true, false);
		let textures = Node::binding_array("textures", Node::combined_image_sampler(), 0, 9, true, false, 16);

		let set1_binding0 = Node::binding("material_count", Node::buffer("MaterialCount", vec![Node::member("material_count", "u32[2073600]")]), 1, 0, true, false);
		let set1_binding1 = Node::binding("material_offset", Node::buffer("MaterialOffset", vec![Node::member("material_offset", "u32[2073600")]), 1, 1, true, false);
		let set1_binding4 = Node::binding("pixel_mapping", Node::buffer("PixelMapping", vec![Node::member("pixel_mapping", "vec2u16[2073600]")]), 1, 4, true, false);
		let set1_binding6 = Node::binding("triangle_index", Node::image("r32ui"), 1, 6, true, false);

		Self {
			mesh_struct,
			camera_struct,
			meshlet_struct,
			light_struct,
			material_struct,
			meshes,
			positions: position,
			normals,
			tangents,
			uvs,
			vertex_indices: set0_binding4,
			primitive_indices: set0_binding5,
			meshlets,
			textures,
			material_count: set1_binding0,
			material_offset: set1_binding1,
			pixel_mapping: set1_binding4,
			triangle_index: set1_binding6,
		}
	}
}