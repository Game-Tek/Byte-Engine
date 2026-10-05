use std::{cell::RefCell, fmt::Write as _};

use super::super::{ResourceAccessorKind, intrinsic_requirements, is_two, resource_accessor, resource_reference_kind};
use crate::shader::generator::{
	NodeEmitter, ShaderFormatting, ShaderGenerationSettings, Stages, emit_statement_block, is_integer_besl_type,
	ordered_shader_nodes,
};

/// The `Generator` struct exists to produce GLSL source for Vulkan-backed shader pipelines.
///
/// # Parameters
///
/// - `minified`: Controls compact shader output. The default is `true` in release builds.
pub struct Generator {
	minified: bool,
	/// The stage being generated, which decides interpolation qualifiers and workgroup storage support.
	stage: Stages,
	match_break_depth: Option<usize>,
}

impl Generator {
	/// Creates a GLSL transpiler with the default formatting mode.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			stage: Stages::Vertex,
			match_break_depth: None,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	/// Generates a GLSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The shader compilation settings.
	/// * `main_function_node` - The shader's main function node.
	///
	/// # Returns
	///
	/// The GLSL shader as a string.
	///
	/// # Panics
	///
	/// Panics if the main function node is not a function node.
	pub fn generate(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<String, ()> {
		self.stage = shader_compilation_settings.stage;
		let mut string = String::with_capacity(2048);
		let order = ordered_shader_nodes(main_function_node, "GLSL");
		crate::shader::generator::validate_workgroup_storage_stage(&shader_compilation_settings.stage, &order)?;
		crate::shader::generator::validate_vertex_builtin_inputs(&shader_compilation_settings.stage, &order)?;
		let requirements = intrinsic_requirements(&order);
		let uses_subgroup_intrinsics = requirements.uses_subgroup_intrinsics;
		// Reachable code needs native 16-bit floating-point arithmetic when it declares or converts to an f16 type.
		let uses_f16_types = order.iter().any(|node| {
			matches!(node.borrow().node(), besl::Nodes::Struct { name, .. } if matches!(name.as_str(), "f16" | "vec2f16" | "vec3f16" | "vec4f16"))
		}) || requirements.uses_f16;
		if uses_subgroup_intrinsics && !matches!(shader_compilation_settings.stage, Stages::Compute { .. }) {
			return Err(());
		}

		self.generate_glsl_header_block(
			&mut string,
			shader_compilation_settings,
			uses_subgroup_intrinsics,
			uses_f16_types,
		);

		for node in order {
			self.emit_node(&mut string, &node);
		}

		Ok(string)
	}

	/// Emits the GLSL version, stage, extension, and layout declarations.
	fn generate_glsl_header_block(
		&self,
		glsl_block: &mut String,
		compilation_settings: &ShaderGenerationSettings,
		uses_subgroup_intrinsics: bool,
		uses_f16_types: bool,
	) {
		glsl_block.push_str("#version 450 core\n");

		match compilation_settings.stage {
			Stages::Vertex => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
			Stages::Fragment => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => glsl_block.push_str("#pragma shader_stage(compute)\n"),
			Stages::Task { .. } => panic!(
				"GLSL task shader lowering is unsupported. The most likely cause is that a task BESL shader was sent to the deferred GLSL backend."
			),
			Stages::Mesh { .. } => glsl_block.push_str("#pragma shader_stage(mesh)\n"),
		}

		glsl_block.push_str("#extension GL_EXT_shader_16bit_storage:require\n");
		glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types:require\n");
		if uses_f16_types {
			glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types_float16:require\n");
		}
		glsl_block.push_str("#extension GL_EXT_nonuniform_qualifier:require\n");
		glsl_block.push_str("#extension GL_EXT_scalar_block_layout:require\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference:enable\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference2:enable\n");
		glsl_block.push_str("#extension GL_EXT_shader_image_load_formatted:enable\n");

		match compilation_settings.stage {
			Stages::Compute { .. } if uses_subgroup_intrinsics => {
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_basic:require\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_ballot:require\n");
			}
			Stages::Mesh {
				maximum_vertices,
				maximum_primitives,
				..
			} => {
				glsl_block.push_str("#extension GL_EXT_mesh_shader:require\n");
				let _ = writeln!(
					glsl_block,
					"layout(triangles,max_vertices={maximum_vertices},max_primitives={maximum_primitives}) out;"
				);
			}
			_ => {}
		}

		if let Stages::Compute { local_size } | Stages::Mesh { local_size, .. } = compilation_settings.stage {
			let _ = writeln!(
				glsl_block,
				"layout(local_size_x={},local_size_y={},local_size_z={}) in;",
				local_size.width(),
				local_size.height(),
				local_size.depth()
			);
		}

		// BESL matrices are always row major in uniform and storage buffers.
		glsl_block.push_str("layout(row_major) uniform;layout(row_major) buffer;\n");

		glsl_block.push_str("const float PI = 3.14159265359;");
		glsl_block.push_str(
			"bool _besl_is_finite(float value){return !isnan(value)&&!isinf(value);}\n\
			 bool _besl_is_normal(float value){return _besl_is_finite(value)&&abs(value)>=1.1754943508222875e-38;}\n",
		);
		if uses_f16_types {
			glsl_block.push_str(
				"bool _besl_is_finite(float16_t value){return !isnan(value)&&!isinf(value);}\n\
				 bool _besl_is_normal(float16_t value){return _besl_is_finite(value)&&abs(value)>=float16_t(0.00006103515625);}\n",
			);
		}
		glsl_block.push_str(ShaderFormatting::new(self.minified).break_str());
	}

	// Emits ordinary 2D samples, descriptor-array samples, and one selected 2D-array layer.
	fn emit_sample(&mut self, string: &mut String, arguments: &[besl::NodeReference]) {
		string.push_str("texture(");
		let accessor = resource_accessor(&arguments[0]);
		if let Some((ResourceAccessorKind::Texture2DArrayLayer, resource, _)) = &accessor {
			self.emit_node(string, resource);
		} else {
			self.emit_node(string, &arguments[0]);
		}
		self.emit_separator(string);
		if let Some((ResourceAccessorKind::Texture2DArrayLayer, _, layer)) = accessor {
			string.push_str("vec3(");
			self.emit_node(string, &arguments[1]);
			self.emit_separator(string);
			string.push_str("float(");
			self.emit_node(string, &layer);
			string.push_str("))");
		} else {
			self.emit_node(string, &arguments[1]);
		}
		string.push(')');
	}
}

impl NodeEmitter for Generator {
	/// Translates BESL intrinsic type names to GLSL type names, such as `vec2f` to `vec2`.
	fn type_from_besl(source: &str) -> &str {
		match source {
			"void" => "void",
			"atomicu32" => "uint32_t",
			"atomici32" => "int32_t",
			"vec2f16" => "f16vec2",
			"vec3f16" => "f16vec3",
			"vec4f16" => "f16vec4",
			"vec2f" => "vec2",
			"vec2u" => "uvec2",
			"vec2i" => "ivec2",
			"vec2u16" => "u16vec2",
			"vec3u16" => "u16vec3",
			"vec4u16" => "u16vec4",
			"vec3u" => "uvec3",
			"vec4u" => "uvec4",
			"vec3f" => "vec3",
			"vec4f" => "vec4",
			"packed_vec4f" => "vec4",
			"mat2f" => "mat2",
			"mat3f" => "mat3",
			"mat4f" => "mat4",
			"mat4x3f" => "mat4x3",
			"f16" => "float16_t",
			"f32" => "float",
			"u8" => "uint8_t",
			"u16" => "uint16_t",
			"u32" => "uint32_t",
			"i32" => "int32_t",
			"Texture2D" => "in sampler2D",
			"Texture3D" => "in sampler3D",
			"TextureCube" => "in samplerCube",
			"TextureCubeArray" => "in samplerCubeArray",
			"ArrayTexture2D" => "in sampler2DArray",
			_ => source,
		}
	}
	const SPECIALIZATION_QUALIFIER: &'static str = "const";
	fn emit_specialization_constant(&self, string: &mut String, type_name: &str, name: std::fmt::Arguments<'_>, index: usize) {
		let _ = write!(string, "layout(constant_id={index})const {type_name} {name}=1.0f;");
	}
	fn minified(&self) -> bool {
		self.minified
	}
	fn match_break_depth(&mut self) -> &mut Option<usize> {
		&mut self.match_break_depth
	}
	fn is_reserved_identifier(name: &str) -> bool {
		super::reserved::is_reserved(name)
	}
	// Keep the intrinsic table contiguous so unsupported names cannot silently drift between GLSL call forms.
	#[allow(clippy::too_many_lines)]
	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeReference,
		arguments: &[besl::NodeReference],
		elements: &[besl::NodeReference],
	) {
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic {
			name,
			elements: definition,
			r#return,
		} = intrinsic.node()
		else {
			for element in elements {
				self.emit_node(string, element);
			}
			return;
		};

		let has_body = definition
			.iter()
			.any(|element| !matches!(element.borrow().node(), besl::Nodes::Parameter { .. }));
		match name.as_str() {
			// Texture lowerings bypass intrinsic bodies.
			"sample" => self.emit_sample(string, arguments),
			"sample_texture_2d_array_grad" => {
				string.push_str("textureGrad(");
				self.emit_node(string, &arguments[0]);
				string.push_str("[nonuniformEXT(");
				self.emit_node(string, &arguments[1]);
				string.push_str(")]");
				for argument in &arguments[2..5] {
					self.emit_separator(string);
					self.emit_node(string, argument);
				}
				string.push(')');
			}
			"gather" => {
				string.push_str("textureGather(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				if let Some(layer) = arguments.get(2) {
					string.push_str("vec3(");
					self.emit_node(string, &arguments[1]);
					self.emit_separator(string);
					string.push_str("float(");
					self.emit_node(string, layer);
					string.push_str("))");
				} else {
					self.emit_node(string, &arguments[1]);
				}
				string.push(')');
			}
			"texture_lod" | "downsample_min" | "downsample_max" => {
				string.push_str("textureLod(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				if arguments.len() == 4 {
					string.push_str("vec3(");
					self.emit_node(string, &arguments[1]);
					self.emit_separator(string);
					string.push_str("float(");
					self.emit_node(string, &arguments[2]);
					string.push_str("))");
				} else {
					self.emit_node(string, &arguments[1]);
				}
				self.emit_separator(string);
				if let Some(lod) = arguments.get(if arguments.len() == 4 { 3 } else { 2 }) {
					self.emit_node(string, lod);
				} else {
					string.push_str("0.0");
				}
				string.push(')');
				if name != "texture_lod" {
					string.push_str(".x");
				}
			}
			"texture_cube_array_lod" => {
				string.push_str("textureLod(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("vec4(");
				self.emit_node(string, &arguments[1]);
				self.emit_separator(string);
				string.push_str("float(");
				self.emit_node(string, &arguments[2]);
				string.push_str("))");
				self.emit_separator(string);
				self.emit_node(string, &arguments[3]);
				string.push(')');
			}
			// Every other intrinsic with a body emits its expansion.
			_ if has_body => {
				for element in elements {
					self.emit_node(string, element);
				}
			}
			"fetch_u32" => {
				string.push_str("texelFetch(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("ivec2(");
				self.emit_node(string, &arguments[1]);
				string.push_str("),0).x");
			}
			"fetch" => {
				string.push_str("texelFetch(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				if arguments.len() == 3 {
					string.push_str("ivec3(ivec2(");
				} else {
					string.push_str("ivec2(");
				}
				self.emit_node(string, &arguments[1]);
				if let Some(layer) = arguments.get(2) {
					string.push_str("),int(");
					self.emit_node(string, layer);
					string.push_str(")),0)");
				} else {
					string.push_str("),0)");
				}
			}
			"texture_size" => {
				string.push_str("uvec2(textureSize(");
				self.emit_node(string, &arguments[0]);
				string.push_str(",0))");
			}
			"image_load" | "image_load_u32" => {
				string.push_str("imageLoad(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("ivec2(");
				self.emit_node(string, &arguments[1]);
				string.push_str(if name == "image_load_u32" { ")).x" } else { "))" });
			}
			"image_size" => {
				string.push_str("uvec2(imageSize(");
				self.emit_node(string, &arguments[0]);
				string.push_str("))");
			}
			"write" | "image_atomic_or" => {
				string.push_str(if name == "write" { "imageStore(" } else { "imageAtomicOr(" });
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("ivec2(");
				self.emit_node(string, &arguments[1]);
				string.push(')');
				self.emit_separator(string);
				self.emit_node(string, &arguments[2]);
				string.push(')');
			}
			"guard_image_bounds" => {
				string.push_str("if(");
				self.emit_node(string, &arguments[1]);
				string.push_str(".x>=uint(imageSize(");
				self.emit_node(string, &arguments[0]);
				string.push_str(").x)||");
				self.emit_node(string, &arguments[1]);
				string.push_str(".y>=uint(imageSize(");
				self.emit_node(string, &arguments[0]);
				string.push_str(").y)){return;}");
			}
			"pow" if arguments.len() == 2 && is_two(&arguments[0]) => {
				string.push_str("exp2(");
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			// findLSB returns -1 for zero, which converts to BESL's 0xFFFFFFFF.
			"find_lsb" => {
				string.push_str("uint(findLSB(");
				self.emit_call_arguments(string, arguments);
				string.push_str("))");
			}
			"sincos" => {
				string.push_str("vec2(sin(");
				self.emit_node(string, &arguments[0]);
				string.push_str("), cos(");
				self.emit_node(string, &arguments[0]);
				string.push_str("))");
			}
			"round_to_i32" => {
				string.push_str("ivec2(round(");
				self.emit_node(string, &arguments[0]);
				string.push_str("))");
			}
			"atomic_exchange" | "atomic_add" | "atomic_min" | "atomic_max" | "atomic_and" | "atomic_or" | "atomic_xor" => {
				string.push_str(match name.as_str() {
					"atomic_exchange" => "atomicExchange(",
					"atomic_add" => "atomicAdd(",
					"atomic_min" => "atomicMin(",
					"atomic_max" => "atomicMax(",
					"atomic_and" => "atomicAnd(",
					"atomic_or" => "atomicOr(",
					"atomic_xor" => "atomicXor(",
					_ => unreachable!("Expected an atomic binary intrinsic"),
				});
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			"atomic_sub" => {
				string.push_str("atomicAdd(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("-(");
				self.emit_node(string, &arguments[1]);
				string.push_str("))");
			}
			"atomic_compare_exchange" => {
				string.push_str("atomicCompSwap(");
				self.emit_node(string, &arguments[0]);
				for argument in &arguments[1..] {
					self.emit_separator(string);
					self.emit_node(string, argument);
				}
				string.push(')');
			}
			"atomic_load" => {
				string.push_str("atomicAdd(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str(if r#return.borrow().get_name() == Some("i32") {
					"0"
				} else {
					"0u"
				});
				string.push(')');
			}
			"atomic_store" => {
				string.push_str("atomicExchange(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			"thread_id" => string.push_str("uvec2(gl_GlobalInvocationID.xy)"),
			"thread_idx" => string.push_str("uint(gl_LocalInvocationIndex)"),
			"subgroup_lane_index" => string.push_str("gl_SubgroupInvocationID"),
			"threadgroup_position" => string.push_str("uint(gl_WorkGroupID.x)"),
			"subgroup_ballot_any" => {
				string.push_str("any(notEqual(");
				self.emit_node(string, &arguments[0]);
				string.push_str(", uvec4(0u)))");
			}
			"subgroup_ballot_and_not" => {
				string.push('(');
				self.emit_node(string, &arguments[0]);
				string.push_str(" & ~");
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			"workgroup_barrier" => string.push_str("barrier()"),
			"set_mesh_vertex_position" => {
				string.push_str("gl_MeshVerticesEXT[");
				self.emit_node(string, &arguments[0]);
				string.push_str("].gl_Position = ");
				self.emit_node(string, &arguments[1]);
			}
			"set_mesh_triangle" => {
				string.push_str("gl_PrimitiveTriangleIndicesEXT[");
				self.emit_node(string, &arguments[0]);
				string.push_str("] = ");
				self.emit_node(string, &arguments[1]);
			}
			"set_mesh_primitive_render_target_array_index" => {
				string.push_str("gl_MeshPrimitivesEXT[");
				self.emit_node(string, &arguments[0]);
				string.push_str("].gl_Layer = int(");
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			// Every other intrinsic is a GLSL call, renamed where GLSL spells the operation differently.
			_ => {
				string.push_str(match name.as_str() {
					"atan2" => "atan",
					"is_nan" => "isnan",
					"is_infinite" => "isinf",
					"is_finite" => "_besl_is_finite",
					"is_normal" => "_besl_is_normal",
					"u32" => "uint",
					"f32" | "f16" | "u16" | "vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16"
					| "packed_vec4f" => Self::type_from_besl(name),
					"subgroup_ballot" => "subgroupBallot",
					"subgroup_ballot_find_lsb" => "subgroupBallotFindLSB",
					"subgroup_ballot_count" => "subgroupBallotBitCount",
					"subgroup_broadcast_u32" | "subgroup_broadcast_f32" => "subgroupBroadcast",
					"set_mesh_output_counts" => "SetMeshOutputsEXT",
					name => name,
				});
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
		}
	}
	fn emit_expression_member(&mut self, string: &mut String, name: &str, source: &besl::NodeReference) -> bool {
		let source = source.borrow();
		match source.node() {
			besl::Nodes::Input { name: input_name, .. } if name == input_name => match name {
				besl::VERTEX_INDEX_BUILTIN => string.push_str("uint(gl_VertexIndex)"),
				besl::INSTANCE_INDEX_BUILTIN => string.push_str("uint(gl_InstanceIndex)"),
				_ => return false,
			},
			besl::Nodes::Output {
				name: output_name,
				count: None,
				..
			} if self.stage.interpolates_outputs() && name == output_name && besl::is_position_output(name) => {
				string.push_str("gl_Position");
			}
			_ => return false,
		}
		true
	}
	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		self.emit_node(string, left);
		if resource_reference_kind(left) == Some(ResourceAccessorKind::DescriptorArray) {
			// Any expression may pick the element, so the index cannot be assumed uniform across a draw.
			string.push_str("[nonuniformEXT(");
			self.emit_node(string, right);
			string.push_str(")]");
		} else if !matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		) && left.borrow().node().is_indexable()
		{
			string.push('[');
			self.emit_node(string, right);
			string.push(']');
		} else {
			string.push('.');
			self.emit_node(string, right);
		}
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	// Keep the exhaustive node-to-GLSL mapping together so adding a BESL node requires handling its backend contract here.
	#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
	fn emit_node(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(this_node);
		let formatting = ShaderFormatting::new(self.minified);

		let break_char = formatting.break_str();
		let space_char = formatting.space_str();

		match node.node() {
			besl::Nodes::Scope { .. } => {}
			besl::Nodes::Function {
				name,
				statements,
				return_type,
				params,
				..
			} => self.emit_function_node(string, this_node, name, statements, return_type, params),
			besl::Nodes::Struct {
				name, fields, template, ..
			} => self.emit_struct_node(string, name, fields, template),
			besl::Nodes::PushConstant { members } => {
				let _ = write!(
					string,
					"layout(push_constant){space_char}uniform PushConstant{space_char}{{{break_char}"
				);
				emit_statement_block(string, formatting, members, 1, |string, member| {
					self.emit_node(string, member)
				});
				let _ = write!(string, "}}{space_char}push_constant;{break_char}");
			}
			besl::Nodes::Specialization { name, r#type } => self.emit_specialization_node(string, name, r#type),
			besl::Nodes::Member { name, r#type, count } => {
				if let Some(type_name) = r#type.borrow().get_name() {
					// A member may be a user struct, which is declared under its escaped name.
					Self::type_identifier(type_name).push_to(string);
					string.push(' ');
				}
				Self::identifier(name).push_to(string);
				if let Some(count) = count {
					let _ = write!(string, "[{count}]");
				}
			}
			besl::Nodes::Raw { glsl, .. } => {
				if let Some(code) = glsl {
					string.push_str(code);
				}
			}
			besl::Nodes::Parameter { name, r#type } => {
				self.emit_variable_declaration(string, name, r#type.borrow().get_name().unwrap())
			}
			besl::Nodes::Input { name, location, format } => {
				if crate::shader::generator::is_vertex_builtin_input(name) {
					return;
				}
				let format = format.borrow();
				let besl_type = format.get_name().unwrap();
				let _ = write!(
					string,
					"layout(location={location}){space_char}{}in {} {};{break_char}",
					if self.stage.interpolates_inputs() && is_integer_besl_type(besl_type) {
						"flat "
					} else {
						""
					},
					Self::type_from_besl(besl_type),
					Self::identifier(name)
				);
			}
			besl::Nodes::Output {
				name,
				location,
				format,
				count,
				per_vertex,
			} => {
				if count.is_none() && self.stage.interpolates_outputs() && besl::is_position_output(name) {
					return;
				}
				let format = format.borrow();
				let besl_type = format.get_name().unwrap();
				// Per-vertex mesh outputs interpolate like vertex-shader outputs, so integers stay flat.
				let interpolates = count.map_or(self.stage.interpolates_outputs(), |_| *per_vertex);
				let qualifier = if count.is_some() && !*per_vertex {
					"perprimitiveEXT "
				} else if interpolates && is_integer_besl_type(besl_type) {
					"flat "
				} else {
					""
				};
				let _ = write!(
					string,
					"layout(location={location}){space_char}{qualifier}out {} {}",
					Self::type_from_besl(besl_type),
					Self::identifier(name)
				);
				if let Some(count) = count {
					let _ = write!(string, "[{count}]");
				}
				let _ = write!(string, ";{break_char}");
			}
			besl::Nodes::Workgroup { name, format, count } if matches!(self.stage, Stages::Compute { .. }) => {
				let _ = write!(
					string,
					"shared {} {}",
					Self::type_identifier(format.borrow().get_name().unwrap()),
					Self::identifier(name)
				);
				if let Some(count) = count {
					let _ = write!(string, "[{count}]");
				}
				let _ = write!(string, ";{break_char}");
			}
			besl::Nodes::TaskPayload { .. } | besl::Nodes::Workgroup { .. } => {
				panic!(
					"GLSL task storage lowering is unsupported. The most likely cause is that a task or mesh BESL shader was sent to the deferred GLSL backend."
				)
			}
			besl::Nodes::Expression(expression) => self.emit_expression_node(string, expression),
			besl::Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => self.emit_conditional_node(string, condition, statements, else_branch.as_ref()),
			besl::Nodes::Match {
				scrutinee,
				r#type,
				arms,
				default,
			} => self.emit_match_node(string, scrutinee, r#type, arms, default),
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => self.emit_for_loop_node(string, initializer, condition, update, statements),
			besl::Nodes::Binding {
				name,
				slot,
				read,
				write,
				r#type,
				count,
				..
			} => {
				let binding_type = match r#type {
					besl::BindingTypes::Buffer { .. } | besl::BindingTypes::BufferArray { .. } => "buffer",
					besl::BindingTypes::Image { format, .. } => match format.as_str() {
						"r8ui" | "r16ui" | "r32ui" => "uniform uimage2D",
						_ => "uniform image2D",
					},
					besl::BindingTypes::CombinedImageSampler { format } => match format.as_str() {
						"Texture3D" => "uniform sampler3D",
						"TextureCube" => "uniform samplerCube",
						"TextureCubeArray" => "uniform samplerCubeArray",
						"ArrayTexture2D" => "uniform sampler2DArray",
						"r8ui" | "r16ui" | "r32ui" => "uniform usampler2D",
						_ => "uniform sampler2D",
					},
				};

				let _ = write!(string, "layout(set=0,binding={slot}");
				match r#type {
					besl::BindingTypes::Buffer { .. } | besl::BindingTypes::BufferArray { .. } => string.push_str(",scalar"),
					besl::BindingTypes::Image { format } if format != "unknown" => {
						string.push(',');
						string.push_str(format);
					}
					_ => {}
				}
				// Sampled images take no access qualifier.
				let access = match r#type {
					besl::BindingTypes::CombinedImageSampler { .. } => "",
					_ if *read && !*write => "readonly ",
					_ if *write && !*read => "writeonly ",
					_ => "",
				};
				let _ = write!(string, ") {access}{binding_type} ");

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						let _ = write!(string, "_{name}{{");
						for member in members {
							self.emit_node(string, member);
							self.emit_statement_end(string);
						}
						string.push('}');
						Self::identifier(name).push_to(string);
					}
					besl::BindingTypes::BufferArray { element, fixed } => {
						let _ = write!(string, "_{name}{{");
						Self::emit_type_name(string, element.borrow().get_name().unwrap());
						string.push(' ');
						Self::identifier(name).push_to(string);
						// Runtime arrays leave the count to the bound buffer.
						match fixed {
							Some(fixed) => {
								let _ = write!(string, "[{}];", fixed.count);
							}
							None => string.push_str("[];"),
						}
						string.push('}');
					}
					besl::BindingTypes::Image { .. } | besl::BindingTypes::CombinedImageSampler { .. } => {
						Self::identifier(name).push_to(string);
					}
				}

				if !matches!(r#type, besl::BindingTypes::BufferArray { .. })
					&& let Some(count) = count
				{
					let _ = write!(string, "[{count}]");
				}

				self.emit_statement_end(string);
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node(string, element);
				}
			}
			besl::Nodes::Const { name, r#type, value } => {
				string.push_str("const ");
				Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
				string.push(' ');
				Self::identifier(name).push_to(string);
				string.push_str(" = ");
				self.emit_node(string, value);
				let _ = write!(string, ";{break_char}");
			}
		}
	}
}
