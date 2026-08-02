use std::sync::Arc;
use std::{cell::RefCell, ops::Deref, rc::Rc, sync::OnceLock};

use besl::{parser::Node, NodeReference};
use resource_management::{
	asset::{bema_asset_handler::ProgramGenerator, JsonObject},
	resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT,
};
use utils::json::{self, JsonContainerTrait, JsonValueTrait};

use crate::rendering::common_shader_generator::CommonShaderScope;
use crate::rendering::pipelines::visibility::{
	MAX_BINDLESS_TEXTURES, MAX_LIGHTS, MAX_MATERIALS, MAX_MATERIAL_TEXTURES, MAX_MESHLETS, MAX_PIXEL_MAPPING_ENTRIES,
	MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES,
};

fn light_array_type() -> &'static str {
	static LIGHT_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	LIGHT_ARRAY_TYPE
		.get_or_init(|| format!("Light[{MAX_LIGHTS}]").into_boxed_str())
		.as_ref()
}

fn material_array_type() -> &'static str {
	static MATERIAL_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	MATERIAL_ARRAY_TYPE
		.get_or_init(|| format!("Material[{MAX_MATERIALS}]").into_boxed_str())
		.as_ref()
}

fn material_texture_array_type() -> &'static str {
	static MATERIAL_TEXTURE_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	MATERIAL_TEXTURE_ARRAY_TYPE
		.get_or_init(|| format!("u32[{MAX_MATERIAL_TEXTURES}]").into_boxed_str())
		.as_ref()
}

fn vertex_vec3_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("vec3f[{MAX_VERTICES}]").into_boxed_str())
}

fn vertex_vec2_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("vec2f[{MAX_VERTICES}]").into_boxed_str())
}

fn skinned_vertex_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("SkinnedVertex[{MAX_VERTICES}]").into_boxed_str())
}

fn vertex_index_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("u16[{MAX_PRIMITIVE_TRIANGLES}]").into_boxed_str())
}

fn primitive_index_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("u8[{}]", MAX_TRIANGLES * 3).into_boxed_str())
}

fn meshlet_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("Meshlet[{MAX_MESHLETS}]").into_boxed_str())
}

/// Parses one reusable BESL helper function from an isolated source scope.
fn parse_besl_function(source: &'static str, function_name: &str) -> besl::parser::Node<'static> {
	let mut root = besl::parse(source).unwrap_or_else(|error| {
		panic!(
			"Failed to parse `{function_name}`. The most likely cause is invalid BESL syntax in the visibility shader module: {error:?}"
		)
	});

	match root.node_mut() {
		besl::parser::Nodes::Scope { children, .. } if children.len() == 1 => children.remove(0),
		_ => panic!(
			"Invalid `{function_name}` helper scope. The most likely cause is that its BESL source defines more than one top-level element."
		),
	}
}

/// Extracts reusable statements from one portable BESL helper function.
fn parse_besl_statements(source: &'static str, function_name: &str) -> Vec<besl::parser::Node<'static>> {
	let mut function = parse_besl_function(source, function_name);
	match function.node_mut() {
		besl::parser::Nodes::Function { statements, .. } => std::mem::take(statements),
		_ => panic!(
			"Invalid `{function_name}` helper. The most likely cause is that its BESL source no longer defines a function."
		),
	}
}

fn material_evaluation_prefix_statements() -> Vec<besl::parser::Node<'static>> {
	parse_besl_statements(MATERIAL_EVALUATION_PREFIX_SOURCE, "material_evaluation_prefix")
}

fn material_evaluation_suffix_statements() -> Vec<besl::parser::Node<'static>> {
	parse_besl_statements(MATERIAL_EVALUATION_SUFFIX_SOURCE, "material_evaluation_suffix")
}

/// Makes material texture context explicit before the parser tree is linked.
fn add_material_sample_context(node: &mut besl::parser::Node<'_>, texture_slots: &[(&str, u32)]) {
	match node.node_mut() {
		besl::parser::Nodes::Function { statements, .. } => {
			for statement in statements {
				add_material_sample_context(statement, texture_slots);
			}
		}
		besl::parser::Nodes::Conditional { condition, statements } => {
			add_material_sample_context(condition, texture_slots);
			for statement in statements {
				add_material_sample_context(statement, texture_slots);
			}
		}
		besl::parser::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			add_material_sample_context(initializer, texture_slots);
			add_material_sample_context(condition, texture_slots);
			add_material_sample_context(update, texture_slots);
			for statement in statements {
				add_material_sample_context(statement, texture_slots);
			}
		}
		besl::parser::Nodes::Expression(expression) => add_material_sample_context_to_expression(expression, texture_slots),
		_ => {}
	}
}

/// Recurses through one expression and expands the material sampling shorthand.
fn add_material_sample_context_to_expression(expression: &mut besl::parser::Expressions<'_>, texture_slots: &[(&str, u32)]) {
	match expression {
		besl::parser::Expressions::Call { name, parameters } => {
			for parameter in parameters.iter_mut() {
				add_material_sample_context(parameter, texture_slots);
			}
			let besl::parser::TypeName::Named(name) = name else {
				return;
			};
			if !matches!(*name, "sample_material" | "sample_normal") || parameters.len() != 1 {
				return;
			}

			let slot = parameters.remove(0);
			let slot = match slot.node() {
				besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) => texture_slots
					.iter()
					.find_map(|(texture_name, index)| (*texture_name == *name).then_some(*index))
					.map_or(slot, |index| Node::literal_expression(format!("{index}u"))),
				_ => slot,
			};
			let material_textures = Node::accessor(Node::member_expression("material"), Node::member_expression("textures"));
			// Index access has an expression on its right side. Preserve that shape so
			// the linked backend AST can distinguish `textures[slot]` from `.field`.
			parameters.push(Node::accessor(material_textures, Node::sentence(vec![slot])));
			parameters.push(Node::member_expression("vertex_uv"));
		}
		besl::parser::Expressions::Expression(elements) => {
			for element in elements {
				add_material_sample_context(element, texture_slots);
			}
		}
		besl::parser::Expressions::Accessor { left, right } | besl::parser::Expressions::Operator { left, right, .. } => {
			add_material_sample_context(left, texture_slots);
			add_material_sample_context(right, texture_slots);
		}
		besl::parser::Expressions::Return { value } => {
			if let Some(value) = value {
				add_material_sample_context(value, texture_slots);
			}
		}
		besl::parser::Expressions::Macro { body, .. } => add_material_sample_context(body, texture_slots),
		besl::parser::Expressions::Member { .. }
		| besl::parser::Expressions::Literal { .. }
		| besl::parser::Expressions::VariableDeclaration { .. }
		| besl::parser::Expressions::RawCode { .. }
		| besl::parser::Expressions::Continue => {}
	}
}

// These statements are spliced around the material-authored main body. Keeping
// them in BESL lets each backend derive resource access, packed loads, type names,
// and matrix multiplication from the linked AST.
const MATERIAL_EVALUATION_PREFIX_SOURCE: &str = r#"
material_evaluation_prefix: fn () -> void {
	let invocation: vec2u = thread_id();
	if (invocation.x >= material_evaluation_dispatches.material_evaluation_dispatches[push_constant.material_id].w) {
		return;
	}

	let offset: u32 = material_offset.material_offset[push_constant.material_id];
	let packed_pixel_coordinates: vec2u16 = pixel_mapping.pixel_mapping[offset + invocation.x];
	let raw_pixel_coordinates: vec2u = vec2u(
		u32(packed_pixel_coordinates.x),
		u32(packed_pixel_coordinates.y)
	);
	if (raw_pixel_coordinates.x == 0 || raw_pixel_coordinates.y == 0) {
		return;
	}
	let pixel_coordinates: vec2u = raw_pixel_coordinates - vec2u(1, 1);
	let image_extent: vec2u = image_size(triangle_index);
	if (pixel_coordinates.x >= image_extent.x || pixel_coordinates.y >= image_extent.y) {
		return;
	}

	let triangle_meshlet_indices: u32 = image_load_u32(triangle_index, pixel_coordinates);
	let instance_index: u32 = image_load_u32(instance_index_render_target, pixel_coordinates);
	let meshlet_triangle_index: u32 = triangle_meshlet_indices & 255;
	let meshlet_index: u32 = triangle_meshlet_indices >> 8;
	let meshlet: Meshlet = meshlets.meshlets[meshlet_index];
	let mesh: Mesh = meshes.meshes[instance_index];
	let material: Material = materials.materials[push_constant.material_id];

	let primitive_index_base: u32 = (mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3;
	let primitive_index0: u32 = u32(primitive_indices.primitive_indices[primitive_index_base]);
	let primitive_index1: u32 = u32(primitive_indices.primitive_indices[primitive_index_base + 1]);
	let primitive_index2: u32 = u32(primitive_indices.primitive_indices[primitive_index_base + 2]);
	let vertex_index0: u32 = compute_vertex_index(mesh, meshlet, primitive_index0);
	let vertex_index1: u32 = compute_vertex_index(mesh, meshlet, primitive_index1);
	let vertex_index2: u32 = compute_vertex_index(mesh, meshlet, primitive_index2);

	let position0: vec3f = vertex_positions.positions[vertex_index0];
	let position1: vec3f = vertex_positions.positions[vertex_index1];
	let position2: vec3f = vertex_positions.positions[vertex_index2];
	let normal0: vec3f = vertex_normals.normals[vertex_index0];
	let normal1: vec3f = vertex_normals.normals[vertex_index1];
	let normal2: vec3f = vertex_normals.normals[vertex_index2];
	let model_space_vertex_position0: vec4f = vec4f(position0.x, position0.y, position0.z, 1.0);
	let model_space_vertex_position1: vec4f = vec4f(position1.x, position1.y, position1.z, 1.0);
	let model_space_vertex_position2: vec4f = vec4f(position2.x, position2.y, position2.z, 1.0);
	let vertex_normal0: vec4f = vec4f(normal0.x, normal0.y, normal0.z, 0.0);
	let vertex_normal1: vec4f = vec4f(normal1.x, normal1.y, normal1.z, 0.0);
	let vertex_normal2: vec4f = vec4f(normal2.x, normal2.y, normal2.z, 0.0);

	if (mesh.skinned_base_vertex_index != 4294967295) {
		let skinned_vertex_index0: u32 = mesh.skinned_base_vertex_index + (vertex_index0 - mesh.base_vertex_index);
		let skinned_vertex_index1: u32 = mesh.skinned_base_vertex_index + (vertex_index1 - mesh.base_vertex_index);
		let skinned_vertex_index2: u32 = mesh.skinned_base_vertex_index + (vertex_index2 - mesh.base_vertex_index);
		model_space_vertex_position0 = skinned_vertices.vertices[skinned_vertex_index0].position;
		model_space_vertex_position1 = skinned_vertices.vertices[skinned_vertex_index1].position;
		model_space_vertex_position2 = skinned_vertices.vertices[skinned_vertex_index2].position;
		vertex_normal0 = skinned_vertices.vertices[skinned_vertex_index0].normal;
		vertex_normal1 = skinned_vertices.vertices[skinned_vertex_index1].normal;
		vertex_normal2 = skinned_vertices.vertices[skinned_vertex_index2].normal;
	}

	let vertex_uv0: vec2f = vertex_uvs.uvs[vertex_index0];
	let vertex_uv1: vec2f = vertex_uvs.uvs[vertex_index1];
	let vertex_uv2: vec2f = vertex_uvs.uvs[vertex_index2];
	let nc: vec2f = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);
	let view: View = views.views[0];
	let model: mat4x3f = mesh.model;
	let world_space_vertex_position0: vec3f = model * model_space_vertex_position0;
	let world_space_vertex_position1: vec3f = model * model_space_vertex_position1;
	let world_space_vertex_position2: vec3f = model * model_space_vertex_position2;
	let clip_space_vertex_position0: vec4f = view.view_projection * vec4f(
		world_space_vertex_position0.x,
		world_space_vertex_position0.y,
		world_space_vertex_position0.z,
		1.0
	);
	let clip_space_vertex_position1: vec4f = view.view_projection * vec4f(
		world_space_vertex_position1.x,
		world_space_vertex_position1.y,
		world_space_vertex_position1.z,
		1.0
	);
	let clip_space_vertex_position2: vec4f = view.view_projection * vec4f(
		world_space_vertex_position2.x,
		world_space_vertex_position2.y,
		world_space_vertex_position2.z,
		1.0
	);
	let world_space_vertex_normal0: vec3f = normalize(model * vertex_normal0);
	let world_space_vertex_normal1: vec3f = normalize(model * vertex_normal1);
	let world_space_vertex_normal2: vec3f = normalize(model * vertex_normal2);

	let barycentric_deriv: BarycentricDeriv = calculate_full_bary(
		clip_space_vertex_position0,
		clip_space_vertex_position1,
		clip_space_vertex_position2,
		nc,
		vec2f(f32(image_extent.x), f32(image_extent.y))
	);
	let barycenter: vec3f = barycentric_deriv.lambda;
	let derivative_x: vec3f = barycentric_deriv.ddx;
	let derivative_y: vec3f = barycentric_deriv.ddy;
	let world_space_vertex_position: vec3f = interpolate_vec3f_with_deriv(
		barycenter,
		world_space_vertex_position0,
		world_space_vertex_position1,
		world_space_vertex_position2
	);
	let world_space_vertex_normal: vec3f = normalize(interpolate_vec3f_with_deriv(
		barycenter,
		world_space_vertex_normal0,
		world_space_vertex_normal1,
		world_space_vertex_normal2
	));
	let vertex_uv: vec2f = interpolate_vec2f_with_deriv(barycenter, vertex_uv0, vertex_uv1, vertex_uv2);
	let N: vec3f = world_space_vertex_normal;
	let camera_position: vec3f = view.inverse_view * vec4f(0.0, 0.0, 0.0, 1.0);
	let V: vec3f = normalize(camera_position - world_space_vertex_position);
	let position_derivative_x: vec3f = interpolate_vec3f_with_deriv(
		derivative_x,
		world_space_vertex_position0,
		world_space_vertex_position1,
		world_space_vertex_position2
	);
	let position_derivative_y: vec3f = interpolate_vec3f_with_deriv(
		derivative_y,
		world_space_vertex_position0,
		world_space_vertex_position1,
		world_space_vertex_position2
	);
	let uv_derivative_x: vec2f = interpolate_vec2f_with_deriv(derivative_x, vertex_uv0, vertex_uv1, vertex_uv2);
	let uv_derivative_y: vec2f = interpolate_vec2f_with_deriv(derivative_y, vertex_uv0, vertex_uv1, vertex_uv2);
	let tangent_scale: f32 = 1.0 / (uv_derivative_x.x * uv_derivative_y.y - uv_derivative_y.x * uv_derivative_x.y);
	let T: vec3f = normalize(
		tangent_scale * (uv_derivative_y.y * position_derivative_x - uv_derivative_x.y * position_derivative_y)
	);
	let B: vec3f = normalize(
		tangent_scale * ((0.0 - uv_derivative_y.x) * position_derivative_x + uv_derivative_x.x * position_derivative_y)
	);

	let albedo: vec4f = vec4f(1.0, 0.0, 0.0, 1.0);
	let normal: vec3f = vec3f(0.0, 0.0, 1.0);
	let metalness: f32 = 0.0;
	let roughness: f32 = 0.5;
	let occlusion: f32 = 1.0;
	let emission: vec3f = vec3f(0.0, 0.0, 0.0);
}
"#;

const MATERIAL_EVALUATION_SUFFIX_SOURCE: &str = r#"
material_evaluation_suffix: fn () -> void {
	let diffuse: vec3f = vec3f(0.0, 0.0, 0.0);
	let specular: vec3f = vec3f(0.0, 0.0, 0.0);
	let ao_factor: f32 = 1.0;
	if (push_constant.blend == 0) {
		ao_factor = fetch(ao, pixel_coordinates).x;
	}

	normal = normalize(normal.x * T + normal.y * B + normal.z * N);
	let albedo_rgb: vec3f = vec3f(albedo.x, albedo.y, albedo.z);
	let F0: vec3f = vec3f(0.04, 0.04, 0.04) * (1.0 - metalness) + albedo_rgb * metalness;
	let NdotV: f32 = max(dot(normal, V), 0.0);
	let roughness_alpha: f32 = roughness * roughness;
	let roughness_alpha_squared: f32 = roughness_alpha * roughness_alpha;
	let adjusted_roughness: f32 = roughness + 1.0;
	let geometry_k: f32 = adjusted_roughness * adjusted_roughness / 8.0;
	let view_fresnel_factor: f32 = pow(clamp(1.0 - NdotV, 0.0, 1.0), 5.0);
	let one_minus_fresnel_n_dot_v: vec3f = vec3f(1.0, 1.0, 1.0) - fresnel_schlick_from_factor(view_fresnel_factor, F0);
	let light_count: u32 = lighting_data.light_count;

	for (let light_index: u32 = 0; light_index < light_count; light_index = light_index + 1) {
		let light: Light = lighting_data.lights[light_index];
		let L: vec3f = vec3f(0.0, 0.0, 0.0);
		let attenuation: f32 = 1.0;
		let light_position: vec3f = vec3f(light.position.x, light.position.y, light.position.z);
		if (light.type == 68) {
			L = normalize(vec3f(0.0, 0.0, 0.0) - light_position);
		}
		if (light.type != 68) {
			let surface_to_light: vec3f = light_position - world_space_vertex_position;
			let distance_squared: f32 = dot(surface_to_light, surface_to_light);
			if (distance_squared <= 0.0) {
				continue;
			}
			L = surface_to_light * inversesqrt(distance_squared);
			attenuation = 1.0 / distance_squared;
		}

		let NdotL: f32 = max(dot(normal, L), 0.0);
		if (NdotL <= 0.0) {
			continue;
		}

		let occlusion_factor: f32 = 1.0;
		let view_space_surface_position: vec3f = view.view * vec4f(
			world_space_vertex_position.x,
			world_space_vertex_position.y,
			world_space_vertex_position.z,
			1.0
		);
		if (light.type == 68) {
			occlusion_factor = sample_shadow(
				depth_shadow_map,
				light,
				world_space_vertex_position,
				view_space_surface_position,
				world_space_vertex_normal,
				L
			);
			if (occlusion_factor == 0.0) {
				continue;
			}
			attenuation = 1.0;
		}
		if (light.type != 68) {
			if (light.type == 1) {
				let light_direction: vec3f = vec3f(light.direction.x, light.direction.y, light.direction.z);
				let cone_cosine: f32 = dot(normalize(light_direction), vec3f(0.0, 0.0, 0.0) - L);
				let cone_factor: f32 = cone_attenuation(cone_cosine, light.cone_cosines.x, light.cone_cosines.y);
				if (cone_factor <= 0.0) {
					continue;
				}
				attenuation = attenuation * cone_factor;
				occlusion_factor = sample_shadow(
					cone_shadow_map,
					light,
					world_space_vertex_position,
					view_space_surface_position,
					world_space_vertex_normal,
					L
				);
				if (occlusion_factor == 0.0) {
					continue;
				}
			}
		}

		let H: vec3f = normalize(V + L);
		let light_color: vec3f = vec3f(light.color.x, light.color.y, light.color.z);
		let radiance: vec3f = light_color * attenuation;
		let half_view_fresnel_factor: f32 = pow(clamp(1.0 - max(dot(H, V), 0.0), 0.0, 1.0), 5.0);
		let F: vec3f = fresnel_schlick_from_factor(half_view_fresnel_factor, F0);
		let NDF: f32 = distribution_ggx_from_terms(max(dot(normal, H), 0.0), roughness_alpha_squared);
		let G: f32 = geometry_smith_from_terms(NdotV, NdotL, geometry_k);
		let local_specular: vec3f = (NDF * G * F) / (4.0 * NdotV * NdotL + 0.000001);
		let light_fresnel_factor: f32 = pow(clamp(1.0 - NdotL, 0.0, 1.0), 5.0);
		let kD: vec3f = (vec3f(1.0, 1.0, 1.0) - fresnel_schlick_from_factor(light_fresnel_factor, F0))
			* one_minus_fresnel_n_dot_v
			* (1.0 - metalness);
		let local_diffuse: vec3f = kD * albedo_rgb / 3.14159265359;
		diffuse = diffuse + local_diffuse * radiance * NdotL * occlusion_factor;
		specular = specular + local_specular * radiance * NdotL * occlusion_factor;
	}

	let ambient_irradiance: vec3f = sample_environment_irradiance(normal);
	let incident: vec3f = vec3f(0.0, 0.0, 0.0) - V;
	let reflection_direction: vec3f = incident - 2.0 * dot(incident, normal) * normal;
	let reflection_radiance: vec3f = sample_environment_specular(reflection_direction, roughness);
	let F_ibl: vec3f = fresnel_schlick_roughness(NdotV, F0, roughness);
	let kD_ibl: vec3f = (vec3f(1.0, 1.0, 1.0) - F_ibl) * (1.0 - metalness);
	let ibl_diffuse: vec3f = kD_ibl * albedo_rgb * ambient_irradiance;

	let c0: vec4f = vec4f(0.0 - 1.0, 0.0 - 0.0275, 0.0 - 0.572, 0.022);
	let c1: vec4f = vec4f(1.0, 0.0425, 1.04, 0.0 - 0.04);
	let r: vec4f = roughness * c0 + c1;
	let a004: f32 = min(r.x * r.x, pow(2.0, (0.0 - 9.28) * NdotV)) * r.x + r.y;
	let env_brdf: vec2f = vec2f(0.0 - 1.04, 1.04) * a004 + vec2f(r.z, r.w);
	let ibl_specular: vec3f = (F0 * env_brdf.x + env_brdf.y) * reflection_radiance;
	let ambient: vec3f = ibl_diffuse + ibl_specular;
	ao_factor = ao_factor * occlusion;
	let lit: vec3f = (diffuse + specular) * ao_factor + ambient * ao_factor + emission;
	let output_color: vec4f = vec4f(lit.x, lit.y, lit.z, 1.0);
	if (push_constant.blend != 0) {
		let source_alpha: f32 = clamp(albedo.w, 0.0, 1.0);
		let destination_color: vec4f = image_load(lit_map, pixel_coordinates);
		output_color = source_over(
			vec4f(lit.x * source_alpha, lit.y * source_alpha, lit.z * source_alpha, source_alpha),
			destination_color
		);
	}
	write(lit_map, pixel_coordinates, output_color);
}
"#;

const SHADOW_TAP_SOURCE: &str = r#"
sample_shadow_tap: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	offset: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	let offset_shadow_uv: vec2f = shadow_uv + offset;
	if (offset_shadow_uv.x < 0.0 || offset_shadow_uv.x > 1.0 || offset_shadow_uv.y < 0.0 || offset_shadow_uv.y > 1.0) {
		return 1.0;
	}
	if (surface_depth < 0.0 || surface_depth > 1.0) {
		return 1.0;
	}

	let maximum_texel: vec2u = shadow_map_extent - vec2u(1, 1);
	let shadow_texel: vec2u = vec2u(
		u32(clamp(offset_shadow_uv.x * f32(shadow_map_extent.x), 0.0, f32(maximum_texel.x))),
		u32(clamp(offset_shadow_uv.y * f32(shadow_map_extent.y), 0.0, f32(maximum_texel.y)))
	);
	let closest_depth: f32 = fetch(shadow_map, shadow_texel, shadow_layer).x;
	if (surface_depth < closest_depth) {
		return 0.0;
	}
	return 1.0;
}
"#;

const ROTATED_SHADOW_TAP_SOURCE: &str = r#"
sample_rotated_shadow_tap: fn (
	shadow_map: ArrayTexture2D,
	shadow_uv: vec2f,
	surface_depth: f32,
	poisson_offset: vec2f,
	rotation: vec2f,
	texel_size: vec2f,
	shadow_layer: u32,
	shadow_map_extent: vec2u
) -> f32 {
	let rotated_offset: vec2f = vec2f(
		poisson_offset.x * rotation.x - poisson_offset.y * rotation.y,
		poisson_offset.x * rotation.y + poisson_offset.y * rotation.x
	) * texel_size * 1.5;
	return sample_shadow_tap(
		shadow_map,
		shadow_uv,
		surface_depth,
		rotated_offset,
		shadow_layer,
		shadow_map_extent
	);
}
"#;

const SHADOW_SOURCE: &str = r#"
sample_shadow: fn (
	shadow_map: ArrayTexture2D,
	light: Light,
	world_space_position: vec3f,
	view_space_position: vec3f,
	surface_normal: vec3f,
	surface_to_light_direction: vec3f
) -> f32 {
	if (light.shadow_views[0] == 0) {
		return 1.0;
	}

	let shadow_view_index: u32 = light.shadow_views[0];
	let shadow_layer: u32 = light.shadow_layer;
	let bias_scale: f32 = 1.0;
	if (light.type == 68) {
		let depth_value: f32 = abs(view_space_position.z);
		let cascade_index: u32 = 3;
		if (depth_value < views.views[light.shadow_views[0]].far) {
			cascade_index = 0;
		}
		if (cascade_index == 3 && depth_value < views.views[light.shadow_views[1]].far) {
			cascade_index = 1;
		}
		if (cascade_index == 3 && depth_value < views.views[light.shadow_views[2]].far) {
			cascade_index = 2;
		}
		shadow_view_index = light.shadow_views[cascade_index];
		shadow_layer = cascade_index;
		bias_scale = f32(cascade_index + 1);
	}

	let shadow_view: View = views.views[shadow_view_index];
	let surface_light_clip_position: vec4f = shadow_view.view_projection * vec4f(
		world_space_position.x,
		world_space_position.y,
		world_space_position.z,
		1.0
	);
	let surface_light_ndc_position: vec3f = vec3f(
		surface_light_clip_position.x,
		surface_light_clip_position.y,
		surface_light_clip_position.z
	) / surface_light_clip_position.w;
	let shadow_uv: vec2f = vec2f(
		surface_light_ndc_position.x * 0.5 + 0.5,
		0.5 - surface_light_ndc_position.y * 0.5
	);
	let normal_alignment: f32 = max(dot(normalize(surface_normal), surface_to_light_direction), 0.0);
	let cascade_depth_range: f32 = max(shadow_view.far - shadow_view.near, 0.0001);
	let slope_scaled_bias: f32 = 0.0002 * bias_scale * (1.0 - normal_alignment);
	let constant_bias: f32 = 0.00002 * bias_scale;
	let cascade_range_bias: f32 = cascade_depth_range * 0.0000025;
	let surface_depth_bias: f32 = max(slope_scaled_bias + cascade_range_bias, constant_bias);
	let surface_depth: f32 = surface_light_ndc_position.z + surface_depth_bias;
	if (surface_depth < 0.0 || surface_depth > 1.0) {
		return 1.0;
	}

	let shadow_map_extent: vec2u = texture_size(shadow_map);
	let texel_size: vec2f = vec2f(1.0, 1.0) / vec2f(f32(shadow_map_extent.x), f32(shadow_map_extent.y));
	let rotation_noise: f32 = fract(
		sin(dot(vec2f(world_space_position.x, world_space_position.z) + world_space_position.y, vec2f(12.9898, 78.233))) * 43758.5453
	);
	let rotation_angle: f32 = rotation_noise * 6.2831853;
	let rotation: vec2f = vec2f(cos(rotation_angle), sin(rotation_angle));
	let occlusion: f32 = 0.0;
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0 - 0.613392, 0.617481), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.170019, 0.0 - 0.040254), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0 - 0.299417, 0.791925), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.645680, 0.493210), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0 - 0.651784, 0.717887), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.421003, 0.027070), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0 - 0.817194, 0.0 - 0.271096), rotation, texel_size, shadow_layer, shadow_map_extent);
	occlusion = occlusion + sample_rotated_shadow_tap(shadow_map, shadow_uv, surface_depth, vec2f(0.0 - 0.705374, 0.0 - 0.668203), rotation, texel_size, shadow_layer, shadow_map_extent);
	return occlusion / 8.0;
}
"#;

const ENVIRONMENT_IRRADIANCE_SOURCE: &str = r#"
sample_environment_irradiance: fn (direction: vec3f) -> vec3f {
	let dir: vec3f = normalize(direction);
	let environment_uv: vec2f = vec2f(
		atan2(dir.z, dir.x) * 0.15915494309189535 + 0.5,
		0.5 - asin(clamp(dir.y, 0.0 - 1.0, 1.0)) * 0.3183098861837907
	);
	let environment_extent: vec2u = texture_size(environment_irradiance);
	let environment_half_texel: f32 = 0.5 / f32(environment_extent.y);
	environment_uv.y = clamp(environment_uv.y, environment_half_texel, 1.0 - environment_half_texel);
	let environment_sample: vec4f = texture_lod(environment_irradiance, environment_uv);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

const ENVIRONMENT_SPECULAR_SOURCE: &str = r#"
sample_environment_specular: fn (direction: vec3f, roughness: f32) -> vec3f {
	let dir: vec3f = normalize(direction);
	let environment_uv: vec2f = vec2f(
		atan2(dir.z, dir.x) * 0.15915494309189535 + 0.5,
		0.5 - asin(clamp(dir.y, 0.0 - 1.0, 1.0)) * 0.3183098861837907
	);
	let specular_level: f32 = clamp(roughness, 0.0, 1.0) * 7.0;
	let lower_level: u32 = u32(floor(specular_level));
	let upper_level: u32 = lower_level + 1;
	if (upper_level > 7) {
		upper_level = 7;
	}
	let lower_extent: vec2u = environment_level_size(lower_level);
	let upper_extent: vec2u = environment_level_size(upper_level);
	let lower_half_texel: f32 = 0.5 / f32(lower_extent.y);
	let upper_half_texel: f32 = 0.5 / f32(upper_extent.y);
	let lower_uv: vec2f = vec2f(environment_uv.x, clamp(environment_uv.y, lower_half_texel, 1.0 - lower_half_texel));
	let upper_uv: vec2f = vec2f(environment_uv.x, clamp(environment_uv.y, upper_half_texel, 1.0 - upper_half_texel));
	let lower_sample: vec4f = sample_environment_level(lower_level, lower_uv);
	let upper_sample: vec4f = sample_environment_level(upper_level, upper_uv);
	let level_blend: f32 = fract(specular_level);
	let lower_color: vec3f = vec3f(lower_sample.x, lower_sample.y, lower_sample.z);
	let upper_color: vec3f = vec3f(upper_sample.x, upper_sample.y, upper_sample.z);
	return lower_color * (1.0 - level_blend) + upper_color * level_blend;
}
"#;

/// The `VisibilityShaderScope` struct provides material programs with the shared visibility data and lighting contract.
pub struct VisibilityShaderScope {}

/// The `VisibilityShaderGenerator` struct adapts portable material programs for visibility-buffer evaluation.
pub struct VisibilityShaderGenerator {
	scope: besl::parser::Node<'static>,
}

impl VisibilityShaderGenerator {
	pub fn new(
		material_count_read: bool,
		material_count_write: bool,
		material_offset_read: bool,
		material_offset_write: bool,
		material_offset_scratch_read: bool,
		material_offset_scratch_write: bool,
		pixel_mapping_read: bool,
		pixel_mapping_write: bool,
	) -> Self {
		Self {
			scope: VisibilityShaderScope::new_with_params(
				material_count_read,
				material_count_write,
				material_offset_read,
				material_offset_write,
				material_offset_scratch_read,
				material_offset_scratch_write,
				pixel_mapping_read,
				pixel_mapping_write,
			),
		}
	}
}

impl VisibilityShaderScope {
	pub fn new<'a>() -> besl::parser::Node<'a> {
		Self::new_with_params(true, true, true, true, true, true, true, true)
	}

	pub fn new_with_params<'a>(
		material_count_read: bool,
		material_count_write: bool,
		material_offset_read: bool,
		material_offset_write: bool,
		material_offset_scratch_read: bool,
		material_offset_scratch_write: bool,
		pixel_mapping_read: bool,
		pixel_mapping_write: bool,
	) -> besl::parser::Node<'a> {
		use besl::parser::Node;

		let mesh_struct = Node::r#struct(
			"Mesh",
			vec![
				Node::member("model", "mat4x3f"),
				Node::member("material_index", "u32"),
				Node::member("base_vertex_index", "u32"),
				Node::member("base_primitive_index", "u32"),
				Node::member("base_triangle_index", "u32"),
				Node::member("base_meshlet_index", "u32"),
				Node::member("meshlet_count", "u32"),
				Node::member("skinned_base_vertex_index", "u32"),
				Node::member("padding0", "u32"),
			],
		);
		let skinned_vertex_struct = Node::r#struct(
			"SkinnedVertex",
			vec![Node::member("position", "vec4f"), Node::member("normal", "vec4f")],
		);
		let view_struct = Node::r#struct(
			"View",
			vec![
				Node::member("view", "mat4x3f"),
				Node::member("view_projection", "mat4f"),
				Node::member("inverse_view", "mat4x3f"),
				Node::member("fov", "vec2f"),
				Node::member("near", "f32"),
				Node::member("far", "f32"),
			],
		);
		let meshlet_struct = Node::r#struct(
			"Meshlet",
			vec![
				Node::member("primitive_offset", "u32"),
				Node::member("triangle_offset", "u32"),
				Node::member("primitive_count", "u32"),
				Node::member("triangle_count", "u32"),
				Node::member("center_radius", "vec4f"),
				Node::member("cone_apex_cutoff", "vec4f"),
				Node::member("cone_axis", "vec4f"),
			],
		);
		let light_struct = Node::r#struct(
			"Light",
			vec![
				// Use explicit 16-byte vector fields so every storage-buffer backend shares the CPU layout.
				Node::member("position", "vec4f"),
				Node::member("color", "vec4f"),
				Node::member("direction", "vec4f"),
				Node::member("cone_cosines", "vec2f"),
				Node::member("type", "u32"),
				Node::member("shadow_views", "u32[8]"),
				Node::member("shadow_layer", "u32"),
			],
		);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", material_texture_array_type())]);

		let views_binding = Node::constant_buffer_binding(
			"views",
			Node::buffer("ViewsBuffer", vec![Node::member("views", "View[9]")]),
			0,
			true,
			false,
		);
		let meshes = Node::device_buffer_binding(
			"meshes",
			Node::buffer("MeshBuffer", vec![Node::member("meshes", "Mesh[1024]")]),
			1,
			true,
			false,
		);
		let positions = Node::device_buffer_binding(
			"vertex_positions",
			Node::buffer("Positions", vec![Node::member("positions", vertex_vec3_array_type())]),
			2,
			true,
			false,
		);
		let normals = Node::device_buffer_binding(
			"vertex_normals",
			Node::buffer("Normals", vec![Node::member("normals", vertex_vec3_array_type())]),
			3,
			true,
			false,
		);
		let skinned_vertices = Node::device_buffer_binding(
			"skinned_vertices",
			Node::buffer("SkinnedVertices", vec![Node::member("vertices", skinned_vertex_array_type())]),
			4,
			true,
			false,
		);
		let uvs = Node::device_buffer_binding(
			"vertex_uvs",
			Node::buffer("UVs", vec![Node::member("uvs", vertex_vec2_array_type())]),
			5,
			true,
			false,
		);
		let vertex_indices = Node::device_buffer_binding(
			"vertex_indices",
			Node::buffer(
				"VertexIndices",
				vec![Node::member("vertex_indices", vertex_index_array_type())],
			),
			6,
			true,
			false,
		);
		let primitive_indices = Node::device_buffer_binding(
			"primitive_indices",
			Node::buffer(
				"PrimitiveIndices",
				vec![Node::member("primitive_indices", primitive_index_array_type())],
			),
			7,
			true,
			false,
		);
		let meshlets = Node::device_buffer_binding(
			"meshlets",
			Node::buffer("MeshletsBuffer", vec![Node::member("meshlets", meshlet_array_type())]),
			8,
			true,
			false,
		);
		let textures = Node::binding_array(
			"textures",
			Node::combined_image_sampler(),
			9,
			true,
			false,
			MAX_BINDLESS_TEXTURES as u32,
		);

		let material_count = Node::device_buffer_binding(
			"material_count",
			Node::buffer("MaterialCount", vec![Node::member("material_count", "u32[1024]")]),
			1033,
			material_count_read,
			material_count_write,
		); // TODO: somehow set read/write properties per shader
		let material_offset = Node::device_buffer_binding(
			"material_offset",
			Node::buffer("MaterialOffset", vec![Node::member("material_offset", "u32[1024]")]),
			1034,
			material_offset_read,
			material_offset_write,
		);
		let material_offset_scratch = Node::device_buffer_binding(
			"material_offset_scratch",
			Node::buffer(
				"MaterialOffsetScratch",
				vec![Node::member("material_offset_scratch", "u32[1024]")],
			),
			1035,
			material_offset_scratch_read,
			material_offset_scratch_write,
		);
		let material_evaluation_dispatches = Node::device_buffer_binding(
			"material_evaluation_dispatches",
			Node::buffer(
				"MaterialEvaluationDispatches",
				vec![Node::member("material_evaluation_dispatches", "vec4u[1024]")],
			),
			1036,
			material_offset_read,
			material_offset_write,
		);
		let pixel_mapping = Node::device_buffer_binding(
			"pixel_mapping",
			Node::buffer(
				"PixelMapping",
				vec![Node::member(
					"pixel_mapping",
					&format!("vec2u16[{MAX_PIXEL_MAPPING_ENTRIES}]"),
				)],
			),
			1037,
			pixel_mapping_read,
			pixel_mapping_write,
		);
		let triangle_index = Node::binding("triangle_index", Node::image("r32ui"), 1039, true, false);
		let instance_index = Node::binding("instance_index_render_target", Node::image("r32ui"), 1040, true, false);

		let compute_vertex_index = {
			let mut root = besl::parse(
				r#"
				compute_vertex_index: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> u32 {
					let relative_index: u16 = vertex_indices.vertex_indices[
						mesh.base_primitive_index + meshlet.primitive_offset + primitive_index
					];
					return mesh.base_vertex_index + u16_to_u32(relative_index);
				}
				"#,
			)
			.expect("Expected compute_vertex_index source to parse");

			match root.node_mut() {
				besl::parser::Nodes::Scope { children, .. } => children.remove(0),
				_ => panic!(
					"Expected compute_vertex_index source to parse into a scope. The most likely cause is invalid BESL syntax in the visibility shader module."
				),
			}
		};
		let u16_to_u32 = parse_besl_function("u16_to_u32: fn (value: u16) -> u32 { return u32(value); }", "u16_to_u32");
		let cone_attenuation = parse_besl_function(
			"cone_attenuation: fn (cosine: f32, inner_cosine: f32, outer_cosine: f32) -> f32 { return clamp((cosine - outer_cosine) / (inner_cosine - outer_cosine), 0.0, 1.0); }",
			"cone_attenuation",
		);
		let set2_binding0 = Node::binding("lit_map", Node::image("rgba16"), 1041, true, true);
		let set2_binding4 = Node::constant_buffer_binding(
			"lighting_data",
			Node::buffer(
				"LightingBuffer",
				vec![
					Node::member("light_count", "u32"),
					// Keep the light array at the CPU record's 16-byte boundary on scalar-layout backends.
					Node::member("_light_count_padding", "u32[3]"),
					Node::member("lights", light_array_type()),
				],
			),
			1045,
			true,
			false,
		);
		let set2_binding5 = Node::device_buffer_binding(
			"materials",
			Node::buffer("MaterialBuffer", vec![Node::member("materials", material_array_type())]),
			1046,
			true,
			false,
		);
		let set2_binding10 = Node::binding("ao", Node::combined_image_sampler(), 1051, true, false);
		let set2_binding11 = Node::binding("depth_shadow_map", Node::combined_array_image_sampler(), 1052, true, false);
		let cone_shadow_map = Node::binding("cone_shadow_map", Node::combined_array_image_sampler(), 1064, true, false);
		let environment_irradiance = Node::binding("environment_irradiance", Node::combined_image_sampler(), 1054, true, false);
		let environment_specular = Node::binding_array(
			"environment_specular",
			Node::combined_image_sampler(),
			1055,
			true,
			false,
			IBL_PREFILTERED_SPECULAR_MIP_COUNT,
		);

		let push_constant = Node::push_constant(vec![Node::member("material_id", "u32"), Node::member("blend", "u32")]);

		let sample_function = Node::intrinsic_with_parameters(
			"sample_material",
			vec![Node::parameter("texture_index", "u32"), Node::parameter("uv", "vec2f")],
			Node::sentence(vec![Node::member_expression("textures")]),
			"vec4f",
		);

		let sample_normal_function = Node::intrinsic_with_parameters(
			"sample_normal",
			vec![Node::parameter("texture_index", "u32"), Node::parameter("uv", "vec2f")],
			Node::sentence(vec![
				Node::member_expression("textures"),
				Node::member_expression("unit_vector_from_xy"),
			]),
			"vec3f",
		);
		let sample_environment_level = Node::intrinsic_with_parameters(
			"sample_environment_level",
			vec![Node::parameter("level", "u32"), Node::parameter("uv", "vec2f")],
			Node::sentence(vec![Node::member_expression("environment_specular")]),
			"vec4f",
		);
		let environment_level_size = Node::intrinsic(
			"environment_level_size",
			Node::parameter("level", "u32"),
			Node::sentence(vec![Node::member_expression("environment_specular")]),
			"vec2u",
		);

		// Lighting helpers are authored once. Texture operations that differ by API remain typed intrinsics below.
		let sample_shadow_tap = parse_besl_function(SHADOW_TAP_SOURCE, "sample_shadow_tap");
		let sample_rotated_shadow_tap = parse_besl_function(ROTATED_SHADOW_TAP_SOURCE, "sample_rotated_shadow_tap");
		let sample_shadow = parse_besl_function(SHADOW_SOURCE, "sample_shadow");
		let sample_environment_irradiance = parse_besl_function(ENVIRONMENT_IRRADIANCE_SOURCE, "sample_environment_irradiance");
		let sample_environment_specular = parse_besl_function(ENVIRONMENT_SPECULAR_SOURCE, "sample_environment_specular");

		Node::scope(
			"Visibility",
			vec![
				view_struct,
				views_binding,
				mesh_struct,
				skinned_vertex_struct,
				meshlet_struct,
				light_struct,
				material_struct,
				sample_shadow_tap,
				sample_rotated_shadow_tap,
				sample_shadow,
				meshes,
				positions,
				normals,
				skinned_vertices,
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
				u16_to_u32,
				cone_attenuation,
				compute_vertex_index,
				set2_binding0,
				set2_binding4,
				set2_binding5,
				set2_binding10,
				set2_binding11,
				cone_shadow_map,
				environment_irradiance,
				environment_specular,
				push_constant,
				sample_function,
				sample_normal_function,
				sample_environment_level,
				environment_level_size,
				sample_environment_irradiance,
				sample_environment_specular,
			],
		)
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
		let mut extra: Vec<Node<'a>> = Vec::new();
		let mut texture_slots = Vec::new();

		let mut texture_count = 0;

		for variable in material["variables"].as_array().unwrap().iter() {
			let name = variable["name"].as_str().unwrap();
			let data_type = variable["data_type"].as_str().unwrap();

			match data_type {
				"u32" | "f32" | "vec2f" | "vec3f" | "vec4f" => {
					let x = besl::parser::Node::specialization(name, data_type);
					extra.push(x);
				}
				"Texture2D" => {
					texture_slots.push((name, texture_count));
					let slot = format!("{texture_count}u");
					let slot_node = besl::parser::Node::literal_expression(slot);
					let x = besl::parser::Node::constant(name, "u32", slot_node);
					extra.push(x);
					texture_count += 1;
				}
				_ => {}
			}
		}

		let m = root.get_mut("main").unwrap();
		add_material_sample_context(m, &texture_slots);

		if let besl::parser::Nodes::Function { statements, .. } = m.node_mut() {
			statements.splice(0..0, material_evaluation_prefix_statements());
			statements.extend(material_evaluation_suffix_statements());
		}

		root.add(extra);
		root.add(vec![CommonShaderScope::new(), self.scope.clone()]);

		root
	}
}

#[cfg(test)]
mod tests {
	use resource_management::asset::{bema_asset_handler::ProgramGenerator, JsonObject};
	use resource_management::pbr::{
		generate_textured_brdf_program, BrdfAlphaMode, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfNode, BrdfTexture,
		BrdfValue,
	};
	use resource_management::shader::besl::backends::{
		glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator,
	};
	use resource_management::shader::besl::evaluation::ProgramEvaluation;
	use resource_management::shader::generator::ShaderGenerationSettings;
	use utils::json::{self, JsonContainerTrait, JsonValueTrait};

	fn parser_expression_contains_raw_code(expression: &besl::parser::Expressions<'_>) -> bool {
		match expression {
			besl::parser::Expressions::RawCode { .. } => true,
			besl::parser::Expressions::Expression(elements)
			| besl::parser::Expressions::Call {
				parameters: elements, ..
			} => elements.iter().any(parser_node_contains_raw_code),
			besl::parser::Expressions::Accessor { left, right } | besl::parser::Expressions::Operator { left, right, .. } => {
				parser_node_contains_raw_code(left) || parser_node_contains_raw_code(right)
			}
			besl::parser::Expressions::Macro { body, .. } => parser_node_contains_raw_code(body),
			besl::parser::Expressions::Return { value } => value.as_deref().is_some_and(parser_node_contains_raw_code),
			besl::parser::Expressions::Member { .. }
			| besl::parser::Expressions::Literal { .. }
			| besl::parser::Expressions::VariableDeclaration { .. }
			| besl::parser::Expressions::Continue => false,
		}
	}

	/// Walks generated parser nodes so raw code cannot hide in shared lighting helpers.
	fn parser_node_contains_raw_code(node: &besl::parser::Node<'_>) -> bool {
		match node.node() {
			besl::parser::Nodes::RawCode { .. } => true,
			besl::parser::Nodes::Scope { children, .. } => children.iter().any(parser_node_contains_raw_code),
			besl::parser::Nodes::Function { statements, .. } => statements.iter().any(parser_node_contains_raw_code),
			besl::parser::Nodes::Conditional { condition, statements } => {
				parser_node_contains_raw_code(condition) || statements.iter().any(parser_node_contains_raw_code)
			}
			besl::parser::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				parser_node_contains_raw_code(initializer)
					|| parser_node_contains_raw_code(condition)
					|| parser_node_contains_raw_code(update)
					|| statements.iter().any(parser_node_contains_raw_code)
			}
			besl::parser::Nodes::Intrinsic { elements, .. } => elements.iter().any(parser_node_contains_raw_code),
			besl::parser::Nodes::Expression(expression) => parser_expression_contains_raw_code(expression),
			_ => false,
		}
	}

	use crate::besl;

	macro_rules! material_metadata {
		($($json:tt)*) => {
			serde_json::json!({ $($json)* })
				.as_object()
				.expect("test material metadata should be an object")
				.clone()
		};
	}

	#[test]
	fn write_to_albedo() {
		let material = material_metadata! {
			"variables": []
		};

		let shader_source = "main: fn () -> void { albedo = vec4f(1, 2, 3, 4); }";

		let shader_node = besl::parse(shader_source).expect("expected test value");

		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		let _node = besl::lex(shader).expect("expected test value");
	}

	#[test]
	fn vec4f_variable() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "albedo",
					"data_type": "vec4f",
					"value": "Purple"
				}
			]
		};

		let shader_source = "main: fn () -> void { out_color = albedo; }";

		let shader_node = besl::parse(shader_source).expect("expected test value");

		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		println!("{:#?}", shader);
	}

	/// Verifies material texture variables produce valid BESL.
	#[test]
	fn texture_variable_transform_produces_valid_besl() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "base_color",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_source = "main: fn () -> void { albedo = sample_material(base_color); }";
		let shader_node = besl::parse(shader_source).expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		besl::lex(shader).expect("expected test value");
	}

	#[test]
	fn material_evaluation_texture_variables_produce_valid_besl() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "base_color",
					"data_type": "Texture2D"
				},
				{
					"name": "normal_map",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_source = "main: fn () -> void { albedo = sample_material(base_color); normal = sample_normal(normal_map); }";
		let shader_node = besl::parse(shader_source).expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);
		besl::lex(shader).expect("expected test value");
	}

	/// Verifies HLSL transforms tangent-space normals with the same basis convention as GLSL and MSL.
	#[test]
	fn material_evaluation_hlsl_combines_tangent_basis_vectors() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "normal_map",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_node =
			besl::parse("main: fn () -> void { normal = sample_normal(normal_map); }").expect("test material should parse");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material))
			.expect("material evaluation should produce valid BESL");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let source = HLSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect(
				"Failed to emit the HLSL material pass. The most likely cause is an invalid tangent-basis shader contract.",
			);

		assert!(
			source.contains("normal = normalize(((normal.x * T) + (normal.y * B)) + (normal.z * N));"),
			"HLSL did not combine the tangent basis explicitly. The most likely cause is that the material pass reintroduced a row-versus-column matrix assumption."
		);
		assert!(
			!source.contains("mul(TBN, normal)"),
			"HLSL multiplied a row-constructed tangent basis as a column basis. The most likely cause is that the material pass reintroduced the faceted-normal transform."
		);
	}

	/// Verifies material evaluation keeps per-pixel and per-light terms out of the repeated PCF tap path.
	#[test]
	fn material_evaluation_hoists_shared_terms_and_uses_direct_ao_reads() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("test material should parse");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material))
			.expect("material evaluation should produce valid BESL");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		let glsl = GLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the GLSL material pass. The most likely cause is an invalid visibility shader contract.");
		let hlsl = HLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the HLSL material pass. The most likely cause is an invalid visibility shader contract.");
		let msl = MSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the MSL material pass. The most likely cause is an invalid visibility shader contract.");
		assert!(glsl.contains("texelFetch(ao, ivec2(pixel_coordinates),0).x"));
		assert!(hlsl.contains("ao.Load(int3(pixel_coordinates, 0)).x"));
		assert!(msl.contains("resources.ao.read(pixel_coordinates).x"));
		assert!(glsl.contains("texelFetch(shadow_map, ivec3(ivec2(shadow_texel),int(shadow_layer)),0).x"));
		assert!(hlsl.contains("shadow_map.Load(int4(shadow_texel, int(shadow_layer), 0)).x"));
		assert!(hlsl.contains(
			"environment_specular[NonUniformResourceIndex(lower_level)].GetDimensions(lower_extent.x, lower_extent.y)"
		));
		assert!(msl.contains("float3 world_space_vertex_position0"));
		assert!(!msl.contains("world_space_vertex_positions[3]"));
		assert!(!msl.contains("primitive_indices[3]"));
		assert!(msl.contains("geometry_smith_from_terms(NdotV, NdotL, geometry_k)"));
		assert!(msl.contains("distribution_ggx_from_terms(max(dot(normal, H), 0.0), roughness_alpha_squared)"));
		assert!(msl.contains("View shadow_view = resources.views->views[shadow_view_index];"));
		assert!(msl.contains(
			"float sample_shadow_tap(texture2d_array<float> shadow_map, float2 shadow_uv, float surface_depth, float2 offset, uint shadow_layer, uint2 shadow_map_extent)"
		));
		assert!(msl.contains("float2 offset_shadow_uv"));
		assert!(msl.contains("shadow_map.read(shadow_texel, shadow_layer).x"));
	}

	/// Verifies material evaluation with skinned geometry produces valid BESL.
	#[test]
	fn material_evaluation_with_skinning_produces_valid_besl() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);
		besl::lex(shader).expect("expected test value");
	}

	/// Verifies material evaluation samples the bound environment without a procedural fallback.
	#[test]
	fn material_evaluation_with_environment_ibl_produces_valid_besl() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material)).expect("expected test value");
		let main = shader.get_main().expect("expected material evaluation main");
		let source = MSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("expected valid Metal material evaluation source");
		assert!(!source.contains("sample_analytical_reflection"));
		assert!(!source.contains("environment_sample.a"));
		assert!(!source.contains("lower_sample.a"));
	}

	#[test]
	fn material_evaluation_emits_cone_attenuation_for_every_backend() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material)).expect("expected test value");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));

		let glsl = GLSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the GLSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);
		let hlsl = HLSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the HLSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);
		let msl = MSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the MSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);

		for source in [&glsl, &hlsl, &msl] {
			assert!(source.contains("cone_cosines"));
			assert!(source.contains("cone_attenuation"));
			assert!(source.contains("light.type == 1"));
			assert!(source.contains("_light_count_padding"));
			assert!(source.contains("shadow_layer"));
			assert!(source.contains("cone_shadow_map"));
		}
		assert!(glsl.contains("vec4 position"));
		assert!(glsl.contains("uint32_t type"));
		assert!(hlsl.contains("float4 position"));
		assert!(hlsl.contains("uint32_t type"));
		assert!(msl.contains("float4 position"));
		assert!(msl.contains("uint type"));
		assert!(
			!msl.contains("mul("),
			"Metal material evaluation must use the native multiplication operator."
		);

		#[cfg(target_os = "macos")]
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&msl,
			"visibility-cone-light-material",
		)
		.expect("Failed to compile the MSL cone-light material pass. The most likely cause is invalid generated Metal source.");
	}

	/// Verifies the generated material pass stays in BESL so backend lowering owns storage and matrix syntax.
	#[test]
	fn material_evaluation_contains_no_backend_raw_code() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);

		assert!(
			!parser_node_contains_raw_code(&shader),
			"Material evaluation and its shared lighting helpers must remain portable BESL instead of embedding backend source."
		);
	}

	/// Compiles a production-generated trivial material evaluation pass and guards its required semantic resource access.
	#[test]
	fn trivial_generated_material_evaluation_pass_links_and_reflects_required_bindings() {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.constant(BrdfValue::Vector4([0.8, 0.6, 0.4, 1.0]));
		let metallic = builder.constant(BrdfValue::Scalar(0.25));
		let roughness = builder.constant(BrdfValue::Scalar(0.5));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);
		let material_program = generate_textured_brdf_program(&material).expect(
			"Failed to generate the trivial material program. The most likely cause is an invalid BRDF material graph.",
		);
		let material_metadata = material_metadata! {
			"variables": []
		};

		// Material evaluation reads the exact dispatch count, offset, and mapping state while retaining the lit target for transparent blending.
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(material_program, &material_metadata);
		let program = besl::lex(shader).expect(
			"Failed to link the trivial material evaluation pass. The most likely cause is a drifted visibility shader contract.",
		);
		let main = program.get_main().expect(
			"Missing trivial material evaluation main. The most likely cause is that material generation stopped producing an entry point.",
		);
		let evaluation = ProgramEvaluation::from_main(&main).expect(
			"Failed to reflect the trivial material evaluation pass. The most likely cause is an invalid visibility resource graph.",
		);

		for slot in [1034, 1036, 1037] {
			let binding = evaluation.bindings().iter().find(|binding| binding.slot == slot).unwrap_or_else(|| {
				panic!(
					"Missing required material evaluation binding at slot {slot}. The most likely cause is that generated material reachability drifted."
				)
			});
			assert!(
				binding.read,
				"Material evaluation binding at slot {slot} is not readable. The most likely cause is incorrect visibility scope access metadata."
			);
		}

		// These strides are the CPU/GPU storage contract reachable from this material variant.
		for (slot, expected_stride) in [
			(0, crate::rendering::pipelines::visibility::VIEW_DATA_BUFFER_STRIDE),
			(1, crate::rendering::pipelines::visibility::MESH_DATA_BUFFER_STRIDE),
			(2, 12),
			(3, 12),
			(4, 32),
			(5, 8),
			(6, crate::rendering::pipelines::visibility::VERTEX_INDEX_BUFFER_STRIDE),
			(7, crate::rendering::pipelines::visibility::PRIMITIVE_INDEX_BUFFER_STRIDE),
			(8, 64),
			(1034, 4),
			(1036, 16),
			(1037, 4),
			(1045, 1552),
		] {
			let binding = evaluation
				.bindings()
				.iter()
				.find(|binding| binding.slot == slot)
				.unwrap_or_else(|| {
					panic!(
					"Missing material evaluation binding at slot {slot}. The most likely cause is that visibility resource retention drifted."
				)
				});
			assert_eq!(
				binding.buffer_stride,
				Some(expected_stride),
				"Unexpected storage-buffer stride at slot {slot}. The most likely cause is that the BESL storage layout diverged from its CPU record."
			);
		}

		let lit_binding = evaluation.bindings().iter().find(|binding| binding.slot == 1041).expect(
			"Missing material evaluation lit binding. The most likely cause is that generated shading stopped retaining its output target.",
		);
		assert!(
			lit_binding.read && lit_binding.write,
			"Material evaluation lit binding is not read-write. The most likely cause is that transparent source-over access drifted."
		);
	}

	/// Verifies native material evaluation emits one bindless sample for a texture shared by several BRDF roles.
	#[test]
	fn generated_material_evaluation_reuses_shared_texture_sample() {
		let mut builder = BrdfMaterialBuilder::new();
		let texture = builder.texture(BrdfTexture {
			image_index: 3,
			texcoord_channel: 0,
		});
		let metallic = builder.extract_channel(texture, resource_management::pbr::BrdfChannel::Blue);
		let roughness = builder.extract_channel(texture, resource_management::pbr::BrdfChannel::Green);
		let normal = builder.add(BrdfNode::NormalMap {
			source: texture,
			scale: 0.5,
		});
		let occlusion = builder.add(BrdfNode::Occlusion {
			source: texture,
			strength: 0.75,
		});
		let emission = builder.add(BrdfNode::Emission { color: texture });
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color: texture,
			metallic,
			roughness,
			normal: Some(normal),
			occlusion: Some(occlusion),
			emission: Some(emission),
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);
		let material_program = generate_textured_brdf_program(&material).expect(
			"Failed to generate the shared-texture material program. The most likely cause is an invalid BRDF material graph.",
		);
		let material_metadata = material_metadata! {
			"variables": [{
				"name": "gltf_texture_3",
				"data_type": "Texture2D"
			}]
		};

		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(material_program, &material_metadata);
		let program = besl::lex(shader).expect(
			"Failed to link the shared-texture material evaluation pass. The most likely cause is a drifted visibility shader contract.",
		);
		let main = program.get_main().expect(
			"Missing shared-texture material entry point. The most likely cause is that material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		let glsl = GLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect(
				"Failed to emit the shared-texture GLSL material pass. The most likely cause is an invalid visibility shader contract.",
			);
		let hlsl = HLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect(
				"Failed to emit the shared-texture HLSL material pass. The most likely cause is an invalid visibility shader contract.",
			);
		let msl = MSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect(
				"Failed to emit the shared-texture MSL material pass. The most likely cause is an invalid visibility shader contract.",
			);
		assert_eq!(
			glsl.match_indices("texture(textures[nonuniformEXT(").count(),
			1,
			"The generated GLSL material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert_eq!(
			hlsl.match_indices("].SampleLevel(textures_sampler,").count(),
			1,
			"The generated HLSL material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert_eq!(
			msl.match_indices("].sample(resources.textures_sampler[").count(),
			1,
			"The generated material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert!(
			msl.contains("float4 material_texture_sample_0"),
			"The generated material did not retain its reusable texel local. The most likely cause is that texture-sample lowering stopped emitting the cache binding."
		);
		assert_eq!(
			msl.match_indices("decode_material_normal(material_texture_sample_0)").count(),
			1,
			"The scaled normal map decoded the shared texel more than once. The most likely cause is that normal scaling bypassed the reusable helper."
		);

		#[cfg(target_os = "macos")]
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&msl,
			"visibility-shared-material-texture-sample",
		)
		.expect(
			"Failed to compile the shared-texture MSL material pass. The most likely cause is invalid generated Metal source.",
		);
	}

	/// Ensures every reflected resource has a retained write in the material-evaluation pass.
	#[test]
	fn material_evaluation_flat_interface_matches_retained_resource_slots() {
		let retained_ranges = [
			(0, 1),
			(1, 1),
			(2, 1),
			(3, 1),
			(4, 1),
			(5, 1),
			(6, 1),
			(7, 1),
			(8, 1),
			(9, 1024),
			(1033, 1),
			(1034, 1),
			(1035, 1),
			(1036, 1),
			(1037, 1),
			(1039, 1),
			(1040, 1),
			(1041, 1),
			(1045, 1),
			(1046, 1),
			(1051, 1),
			(1052, 1),
			(1053, 1),
			(1054, 1),
			(1055, 8),
			(1064, 1),
		];
		let cases = [
			(
				material_metadata! {
					"variables": []
				},
				"main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }",
			),
			(
				material_metadata! {
					"variables": [{
						"name": "base_color",
						"data_type": "Texture2D"
					}]
				},
				"main: fn () -> void { albedo = sample_material(base_color); }",
			),
		];

		for (material, shader_source) in cases {
			let shader_node = besl::parse(shader_source).expect("expected test value");
			let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
			let shader = shader_generator.transform(shader_node, &material);
			let root = besl::lex(shader).expect("expected test value");
			let main_node = root.get_main().expect("expected test value");
			let evaluation =
				ProgramEvaluation::from_main(&main_node).expect("Expected material evaluation reflection to succeed");
			let lit_binding = evaluation.bindings().iter().find(|binding| binding.slot == 1041).expect(
				"Missing material lit binding. The most likely cause is that material output stopped retaining slot 1041.",
			);
			assert!(
				lit_binding.read && lit_binding.write,
				"Material lit binding is not read-write. The most likely cause is that transparent source-over access was removed from the visibility scope."
			);
			assert!(
				evaluation.bindings().iter().all(|binding| binding.slot != 1053),
				"Material evaluation still depends on opaque visibility depth. The most likely cause is that surface reconstruction stopped using the winning triangle's barycentrics."
			);
			let unexpected_ranges = evaluation
				.bindings()
				.iter()
				.map(|binding| (binding.slot, binding.count))
				.filter(|binding| !retained_ranges.contains(binding))
				.collect::<Vec<_>>();

			assert!(
				unexpected_ranges.is_empty(),
				"Material evaluation reflected resources that none of its retained descriptor sets writes: {unexpected_ranges:?}"
			);
		}
	}
}
