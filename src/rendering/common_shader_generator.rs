use std::{cell::RefCell, rc::Rc};

use resource_management::asset::material_asset_handler::ProgramGenerator;

use crate::besl::lexer;

pub struct CommonShaderGenerator {
	camera_binding: besl::parser::Node,
	mesh_struct: besl::parser::Node,
	camera_struct: besl::parser::Node,
	meshlet_struct: besl::parser::Node,
	light_struct: besl::parser::Node,
	material_struct: besl::parser::Node,
	meshes: besl::parser::Node,
	positions: besl::parser::Node,
	normals: besl::parser::Node,
	uvs: besl::parser::Node,
	vertex_indices: besl::parser::Node,
	primitive_indices: besl::parser::Node,
	meshlets: besl::parser::Node,
	textures: besl::parser::Node,
	material_count: besl::parser::Node,
	material_offset: besl::parser::Node,
	material_offset_scratch: besl::parser::Node,
	material_evaluation_dispatches: besl::parser::Node,
	pixel_mapping: besl::parser::Node,
	triangle_index: besl::parser::Node,
	instance_index: besl::parser::Node,
	process_meshlet: besl::parser::Node,
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

		let camera_binding = self.camera_binding.clone();
		let material_offset = self.material_offset.clone();
		let material_offset_scratch = self.material_offset_scratch.clone();
		let material_evaluation_dispatches = self.material_evaluation_dispatches.clone();
		let meshes = self.meshes.clone();
		let material_count = self.material_count.clone();
		let uvs = self.uvs.clone();
		let textures = self.textures.clone();
		let pixel_mapping = self.pixel_mapping.clone();
		let triangle_index = self.triangle_index.clone();
		let instance_index = self.instance_index.clone();
		let meshlets = self.meshlets.clone();
		let primitive_indices = self.primitive_indices.clone();
		let vertex_indices = self.vertex_indices.clone();
		let positions = self.positions.clone();
		let normals = self.normals.clone();

		let process_meshlet = self.process_meshlet.clone();

		root.add(vec![mesh_struct, camera_struct, meshlet_struct, light_struct, camera_binding, material_offset, material_offset_scratch, material_evaluation_dispatches, meshes, material_count, uvs, textures, pixel_mapping, triangle_index, meshlets, primitive_indices, vertex_indices, positions, normals, material_struct, instance_index, process_meshlet]);
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
		let light_struct = Node::r#struct("Light", vec![Node::member("view_matrix", "mat4f"), Node::member("projection_matrix", "mat4f"), Node::member("view_projection", "mat4f"), Node::member("position", "vec3f"), Node::member("color", "vec3f"), Node::member("light_type", "u8")]);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", "u32[16]")]);

		let camera_binding = Node::binding("camera", Node::buffer("CameraBuffer", vec![Node::member("camera", "Camera")]), 0, 0, true, false);
		let meshes = Node::binding("meshes", Node::buffer("MeshBuffer", vec![Node::member("meshes", "Mesh[64]")]), 0, 1, true, false);
		let positions = Node::binding("vertex_positions", Node::buffer("Positions", vec![Node::member("positions", "vec3f[8192]")]), 0, 2, true, false);
		let normals = Node::binding("vertex_normals", Node::buffer("Normals", vec![Node::member("normals", "vec3f[8192]")]), 0, 3, true, false);
		let uvs = Node::binding("vertex_uvs", Node::buffer("UVs", vec![Node::member("uvs", "vec2f[8192]")]), 0, 5, true, false);
		let vertex_indices = Node::binding("vertex_indices", Node::buffer("VertexIndices", vec![Node::member("vertex_indices", "u16[8192]")]), 0, 6, true, false);
		let primitive_indices = Node::binding("primitive_indices", Node::buffer("PrimitiveIndices", vec![Node::member("primitive_indices", "u8[8192]")]), 0, 7, true, false);
		let meshlets = Node::binding("meshlets", Node::buffer("MeshletsBuffer", vec![Node::member("meshlets", "Meshlet[8192]")]), 0, 8, true, false);
		let textures = Node::binding_array("textures", Node::combined_image_sampler(), 0, 9, true, false, 16);

		let material_count = Node::binding("material_count", Node::buffer("MaterialCount", vec![Node::member("material_count", "u32[2073600]")]), 1, 0, true, true); // TODO: somehow set read/write properties per shader
		let material_offset = Node::binding("material_offset", Node::buffer("MaterialOffset", vec![Node::member("material_offset", "u32[2073600")]), 1, 1, true, true);
		let material_offset_scratch = Node::binding("material_offset_scratch", Node::buffer("MaterialOffsetScratch", vec![Node::member("material_offset_scratch", "u32[2073600]")]), 1, 2, true, true);
		let material_evaluation_dispatches = Node::binding("material_evaluation_dispatches", Node::buffer("MaterialEvaluationDispatches", vec![Node::member("material_evaluation_dispatches", "vec3u[2073600]")]), 1, 3, true, true);
		let pixel_mapping = Node::binding("pixel_mapping", Node::buffer("PixelMapping", vec![Node::member("pixel_mapping", "vec2u16[2073600]")]), 1, 4, true, true);
		let triangle_index = Node::binding("triangle_index", Node::image("r32ui"), 1, 6, true, false);
		let instance_index = Node::binding("instance_index", Node::image("r32ui"), 1, 7, true, false);

		let process_meshlet = Node::function("process_meshlet", vec![Node::parameter("matrix", "mat4f")], "void", vec![Node::glsl("uint meshlet_index = gl_WorkGroupID.x;
		Meshlet meshlet = meshlets.meshlets[meshlet_index];
		Mesh mesh = meshes.meshes[meshlet.instance_index];
	
		uint instance_index = meshlet.instance_index;
	
		SetMeshOutputsEXT(meshlet.vertex_count, meshlet.triangle_count);
	
		if (gl_LocalInvocationID.x < uint(meshlet.vertex_count)) {
			uint vertex_index = mesh.base_vertex_index + uint32_t(vertex_indices.vertex_indices[uint(meshlet.vertex_offset) + gl_LocalInvocationID.x]);
			gl_MeshVerticesEXT[gl_LocalInvocationID.x].gl_Position = matrix * mesh.model * vec4(vertex_positions.positions[vertex_index], 1.0);
		}
		
		if (gl_LocalInvocationID.x < uint(meshlet.triangle_count)) {
			uint triangle_index = uint(meshlet.triangle_offset) + gl_LocalInvocationID.x;
			uint triangle_indices[3] = uint[](primitive_indices.primitive_indices[triangle_index * 3 + 0], primitive_indices.primitive_indices[triangle_index * 3 + 1], primitive_indices.primitive_indices[triangle_index * 3 + 2]);
			gl_PrimitiveTriangleIndicesEXT[gl_LocalInvocationID.x] = uvec3(triangle_indices[0], triangle_indices[1], triangle_indices[2]);
			out_instance_index[gl_LocalInvocationID.x] = instance_index;
			out_primitive_index[gl_LocalInvocationID.x] = (meshlet_index << 8) | (gl_LocalInvocationID.x & 0xFF);
		}", vec!["meshes".to_string(), "vertex_positions".to_string(), "vertex_indices".to_string(), "primitive_indices".to_string(), "meshlets".to_string()], vec![])]);

		Self {
			mesh_struct,
			camera_struct,
			meshlet_struct,
			light_struct,
			material_struct,

			camera_binding,
			meshes,
			positions,
			normals,
			uvs,
			vertex_indices,
			primitive_indices,
			meshlets,
			textures,
			material_count,
			material_offset,
			material_offset_scratch,
			material_evaluation_dispatches,
			pixel_mapping,
			triangle_index,
			instance_index,

			process_meshlet,
		}
	}
}