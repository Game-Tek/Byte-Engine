use super::*;

mod intrinsics;
impl Generator {
	pub(crate) fn emit_function_prototype(&mut self, string: &mut String, function_node: &besl::NodeReference) {
		let node = RefCell::borrow(function_node);
		let besl::Nodes::Function {
			name,
			return_type,
			params,
			..
		} = node.node()
		else {
			return;
		};

		Self::emit_type_name(string, return_type.borrow().get_name().unwrap());
		string.push(' ');
		Self::identifier(name).push_to(string);
		string.push('(');
		self.emit_function_parameters(string, params);
		self.emit_hidden_context(string, function_node, !params.is_empty(), true);

		string.push(')');
		self.emit_statement_end(string);
	}

	/// Extracts one vertex or primitive field write so adjacent writes can become one native vertex or primitive value.
	pub(crate) fn mesh_write_parts(&self, statement: &besl::NodeReference) -> Option<MeshWrite> {
		let node = statement.borrow();
		if let besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) = node.node()
		{
			let [index, value] = arguments.as_slice() else {
				return None;
			};
			let (per_vertex, field) = match intrinsic.borrow().get_name() {
				Some("set_mesh_primitive_render_target_array_index") => (false, "render_target_array_index"),
				Some("set_mesh_vertex_position") => (true, "position"),
				_ => return None,
			};
			return Some(MeshWrite {
				per_vertex,
				field: field.to_string(),
				index: index.clone(),
				value: value.clone(),
			});
		}

		let besl::Nodes::Expression(besl::Expressions::Operator {
			operator: besl::Operators::Assignment,
			left,
			right,
		}) = node.node()
		else {
			return None;
		};

		let left_node = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::Accessor {
			left: output,
			right: index,
		}) = left_node.node()
		else {
			return None;
		};

		crate::shader::generator::mesh_output_target(output, |name, per_vertex| MeshWrite {
			per_vertex,
			field: Self::mesh_output_field_name(name).to_string(),
			index: index.clone(),
			value: right.clone(),
		})
	}

	/// Returns one vertex or primitive field's native declaration position for Metal aggregate initialization.
	pub(crate) fn mesh_field_order(&self, per_vertex: bool, field: &str) -> usize {
		if field == if per_vertex { "position" } else { "render_target_array_index" } {
			return 0;
		}
		self.mesh_stage_context
			.as_ref()
			.and_then(|context| {
				context
					.mesh_output_fields
					.iter()
					.position(|(vertex_field, declared)| *vertex_field == per_vertex && declared == field)
			})
			.map_or(usize::MAX, |index| index + 1)
	}

	pub(crate) fn emit_statement_block(&mut self, string: &mut String, statements: &[besl::NodeReference], indent: usize) {
		let formatting = ShaderFormatting::new(self.minified);
		let mut i = 0;

		while i < statements.len() {
			if self.mesh_stage_context.is_some()
				&& let Some(first) = self.mesh_write_parts(&statements[i])
			{
				let per_vertex = first.per_vertex;
				let mut index_string = String::new();
				self.emit_node_string(&mut index_string, &first.index);
				let mut writes = vec![(first.field, first.value)];
				let mut next = i + 1;

				while let Some(write) = statements.get(next).and_then(|statement| self.mesh_write_parts(statement)) {
					let mut next_index_string = String::new();
					self.emit_node_string(&mut next_index_string, &write.index);
					if write.per_vertex != per_vertex
						|| next_index_string != index_string
						|| writes.iter().any(|(written, _)| written == &write.field)
					{
						break;
					}
					writes.push((write.field, write.value));
					next += 1;
				}
				// Metal sets a whole vertex at once, so a vertex written without its position would lose it.
				assert!(
					!per_vertex || writes.iter().any(|(field, _)| field == "position"),
					"Metal mesh vertex outputs must be written next to `set_mesh_vertex_position` with the same index. The most likely cause is that a `vertex_output` write is separated from the vertex position write by another statement."
				);
				// Metal requires designated initializers to follow the output struct's declaration order.
				writes.sort_by_key(|(field, _)| self.mesh_field_order(per_vertex, field));

				formatting.push_indentation(string, indent);
				string.push_str(if per_vertex {
					"out_mesh.set_vertex("
				} else {
					"out_mesh.set_primitive("
				});
				self.emit_node_string(string, &first.index);
				string.push_str(if per_vertex { ", VertexOutput{" } else { ", PrimitiveOutput{" });
				for (write_index, (field, value)) in writes.iter().enumerate() {
					if write_index > 0 {
						string.push_str(", ");
					}
					string.push('.');
					Self::identifier(field).push_to(string);
					string.push_str(" = ");
					self.emit_node_string(string, value);
				}
				string.push_str("})");
				formatting.push_statement_end(string);
				i = next;
				continue;
			}

			emit_statement_block(string, formatting, &statements[i..i + 1], indent, |string, statement| {
				self.emit_node_string(string, statement)
			});
			i += 1;
		}
	}

	/// Writes the MSL type of a local, parameter, or constructed value.
	///
	/// BESL arrays copy, assign, and pass by value. C arrays do none of these, so value arrays become `metal::array`.
	/// Struct fields and module constants keep C arrays, which don't change buffer layouts.
	pub(crate) fn emit_value_type(string: &mut String, type_name: &str) {
		if let Some((element_type, count)) = crate::shader::generator::value_array_parts(type_name) {
			string.push_str("metal::array<");
			Self::type_identifier(element_type).push_to(string);
			string.push_str(", ");
			string.push_str(count);
			string.push('>');
		} else {
			Self::emit_type_name(string, type_name);
		}
	}

	/// Translates BESL intrinsic type names to MSL type names, such as `vec2f` to `float2`.
	pub(crate) fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"bool" => "bool",
			"atomicu32" => "atomic_uint",
			"atomici32" => "atomic_int",
			"vec2f16" => "half2",
			"vec3f16" => "half3",
			"vec4f16" => "half4",
			"vec2f" => "float2",
			"vec2u" => "uint2",
			"vec2i" => "int2",
			"vec2u16" => "ushort2",
			"vec3u16" => "ushort3",
			"vec4u16" => "ushort4",
			"vec3u" => "uint3",
			"vec4u" => "uint4",
			"vec3f" => "float3",
			"vec4f" => "float4",
			"mat2f" => "float2x2",
			"mat3f" => "float3x3",
			"mat4f" => "float4x4",
			"mat4x3f" => "float4x3",
			"f16" => "half",
			"f32" => "float",
			"u8" => "uchar",
			"u16" => "ushort",
			"u32" => "uint",
			"i32" => "int",
			"Texture2D" => "texture2d<float>",
			"Texture3D" => "texture3d<float>",
			"TextureCube" => "texturecube<float>",
			"TextureCubeArray" => "texturecube_array<float>",
			"ArrayTexture2D" => "texture2d_array<float>",
			_ => source,
		}
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	// Keep the exhaustive node-to-MSL mapping together so adding a BESL node requires handling its backend contract here.
	#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
	pub(crate) fn emit_node_string(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(this_node);
		let formatting = ShaderFormatting::new(self.minified);

		let break_char = formatting.break_str();

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
			besl::Nodes::PushConstant { .. } => {
				self.emit_push_constant_struct(string, this_node);
				// TODO: Confirm push constant mapping for Metal argument buffers.
				self.emit_push_constant_parameter(string);
				self.emit_statement_end(string);
			}
			besl::Nodes::TaskPayload { .. } | besl::Nodes::Workgroup { .. } => {}
			besl::Nodes::Specialization { name, r#type } => self.emit_specialization_node(string, name, r#type),
			besl::Nodes::Member { name, r#type, count } => {
				if let Some(type_name) = r#type.borrow().get_name() {
					// Stage interfaces keep native vectors, because Metal rejects packed ones there.
					if self.is_packed_mat4x3_member(this_node)
						|| (!self.in_stage_interface_struct && type_name.starts_with("vec"))
					{
						Self::emit_buffer_member_type(string, type_name);
					} else {
						Self::emit_type_name(string, type_name);
					}
					string.push(' ');
				}
				Self::identifier(name).push_to(string);
				if let Some(count) = count {
					let _ = write!(string, "[{count}]");
				}
			}
			besl::Nodes::Raw { glsl, hlsl, msl, .. } => {
				if let Some(code) = msl.as_ref().or(hlsl.as_ref()).or(glsl.as_ref()) {
					string.push_str(code);
				}
			}
			besl::Nodes::Parameter { name, r#type } => {
				self.emit_variable_declaration(string, name, r#type.borrow().get_name().unwrap())
			}
			besl::Nodes::Input { name, location, format } => {
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
				// TODO: Map interpolation qualifiers to Metal (flat/linear).
				let _ = write!(
					string,
					"{type_name} {} [[attribute({location})]];{break_char}",
					Self::identifier(name)
				);
			}
			besl::Nodes::Output {
				name,
				location,
				format,
				count,
				..
			} => {
				if count.is_some() {
					return;
				}

				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
				let _ = write!(
					string,
					"{type_name} {} [[color({location})]];{break_char}",
					Self::identifier(name)
				);
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
				memory_class,
				r#type,
				count,
				..
			} => {
				if self.in_compute_body || self.mesh_stage_context.is_some() {
					self.emit_binding_reference(string, name);
					return;
				}

				let index = *slot;

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						self.emit_named_struct_start(string, format_args!("_{name}"));
						emit_statement_block(string, formatting, members, 1, |string, member| {
							self.emit_node_string(string, member)
						});
						self.emit_struct_declaration_end(string);

						let address_space = buffer_address_space(*memory_class, *write);
						let _ = write!(string, "{address_space} _{name}* {}", Self::identifier(name));
						if let Some(count) = count {
							let _ = write!(string, "[{count}]");
						}
						let _ = write!(string, " [[buffer({index})]];{break_char}");
					}
					besl::BindingTypes::BufferArray { element, .. } => {
						string.push_str(buffer_address_space(*memory_class, *write));
						string.push(' ');
						Self::emit_buffer_member_type(string, element.borrow().get_name().unwrap());
						let _ = write!(string, "* {} [[buffer({index})]];{break_char}", Self::identifier(name));
					}
					besl::BindingTypes::Image { format } => {
						let (element_type, access) = storage_image_type(format, *read, *write);
						let _ = write!(string, "texture2d<{element_type}, {access}> {}", Self::identifier(name));
						if let Some(count) = count {
							let _ = write!(string, "[{count}]");
						}
						let _ = write!(string, " [[texture({index})]];{break_char}");
					}
					besl::BindingTypes::CombinedImageSampler { format } => {
						let texture_type = match format.as_str() {
							"ArrayTexture2D" => "texture2d_array<float>",
							"TextureCube" => "texturecube<float>",
							"TextureCubeArray" => "texturecube_array<float>",
							"r8ui" | "r16ui" | "r32ui" => "texture2d<uint>",
							_ => "texture2d<float>",
						};

						let name = Self::identifier(name);
						let _ = write!(string, "{texture_type} {name}");
						if let Some(count) = count {
							let _ = write!(string, "[{count}]");
						}
						let _ = write!(
							string,
							" [[texture({index})]];{break_char}sampler {name}_sampler [[sampler({index})]];{break_char}"
						);
					}
				}
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node_string(string, element);
				}
			}
			besl::Nodes::Const { name, r#type, value } => {
				let r#type = r#type.borrow();
				let type_name = r#type.get_name().unwrap();
				string.push_str("constant ");
				Self::emit_c_declaration(string, name, type_name);
				string.push_str(" = ");
				// A C array constant initializes from braces, not from its type's constructor call.
				if let besl::Nodes::Expression(besl::Expressions::FunctionCall {
					parameters, function, ..
				}) = value.borrow().node()
					&& crate::shader::generator::scalar_array_vector_type(type_name).is_none()
					&& function.get().borrow().get_name() == Some(type_name)
				{
					string.push('{');
					self.emit_call_arguments(string, parameters);
					string.push('}');
				} else {
					self.emit_node_string(string, value);
				}
				let _ = write!(string, ";{break_char}");
			}
		}
	}

	pub(crate) fn generate_msl_header_block(
		&self,
		msl_block: &mut String,
		compilation_settings: &ShaderGenerationSettings,
		requirements: &IntrinsicRequirements,
	) {
		msl_block.push_str("#include <metal_stdlib>\n");
		msl_block.push_str("using namespace metal;\n");
		if requirements.uses_downsample_min || requirements.uses_downsample_max {
			// Metal executes min/max sampler reduction only on Apple10 GPUs and falls back to averaging elsewhere,
			// which would blend a near and a far surface into a depth that exists in neither. Gathering the four
			// texels and reducing them here is exact on every GPU and costs the same single texture instruction.
			// Metal gather has no explicit-LOD overload, so explicit pyramid levels use four reads instead.
			msl_block.push_str(
			"inline float _besl_downsample_min(texture2d<float> texture, sampler texture_sampler, float2 uv, float lod) {\n\
			 \tfloat4 samples;\n\
			 \tif (lod < 0.5) { samples = texture.gather(texture_sampler, uv, int2(0), component::x); }\n\
			 \telse { uint level = uint(lod); uint2 extent(texture.get_width(level), texture.get_height(level)); int2 base = int2(floor(uv * float2(extent) - 0.5)); uint2 a = uint2(clamp(base, int2(0), int2(extent) - 1)); uint2 b = uint2(clamp(base + int2(1, 0), int2(0), int2(extent) - 1)); uint2 c = uint2(clamp(base + int2(0, 1), int2(0), int2(extent) - 1)); uint2 d = uint2(clamp(base + int2(1), int2(0), int2(extent) - 1)); samples = float4(texture.read(a, level).x, texture.read(b, level).x, texture.read(c, level).x, texture.read(d, level).x); }\n\
			 \treturn metal::min(metal::min(samples.x, samples.y), metal::min(samples.z, samples.w));\n\
			 }\n\
			 inline float _besl_downsample_max(texture2d<float> texture, sampler texture_sampler, float2 uv, float lod) {\n\
			 \tfloat4 samples;\n\
			 \tif (lod < 0.5) { samples = texture.gather(texture_sampler, uv, int2(0), component::x); }\n\
			 \telse { uint level = uint(lod); uint2 extent(texture.get_width(level), texture.get_height(level)); int2 base = int2(floor(uv * float2(extent) - 0.5)); uint2 a = uint2(clamp(base, int2(0), int2(extent) - 1)); uint2 b = uint2(clamp(base + int2(1, 0), int2(0), int2(extent) - 1)); uint2 c = uint2(clamp(base + int2(0, 1), int2(0), int2(extent) - 1)); uint2 d = uint2(clamp(base + int2(1), int2(0), int2(extent) - 1)); samples = float4(texture.read(a, level).x, texture.read(b, level).x, texture.read(c, level).x, texture.read(d, level).x); }\n\
			 \treturn metal::max(metal::max(samples.x, samples.y), metal::max(samples.z, samples.w));\n\
			 }\n",
		);
			msl_block.push_str(
			"inline float _besl_downsample_max(texture2d_array<float> texture, sampler texture_sampler, float2 uv, uint layer, float lod) {\n\
			 \tfloat4 samples;\n\
			 \tif (lod < 0.5) { samples = texture.gather(texture_sampler, uv, layer, int2(0), component::x); }\n\
			 \telse { uint level = uint(lod); uint2 extent(texture.get_width(level), texture.get_height(level)); int2 base = int2(floor(uv * float2(extent) - 0.5)); uint2 a = uint2(clamp(base, int2(0), int2(extent) - 1)); uint2 b = uint2(clamp(base + int2(1, 0), int2(0), int2(extent) - 1)); uint2 c = uint2(clamp(base + int2(0, 1), int2(0), int2(extent) - 1)); uint2 d = uint2(clamp(base + int2(1), int2(0), int2(extent) - 1)); samples = float4(texture.read(a, layer, level).x, texture.read(b, layer, level).x, texture.read(c, layer, level).x, texture.read(d, layer, level).x); }\n\
			 \treturn metal::max(metal::max(samples.x, samples.y), metal::max(samples.z, samples.w));\n\
			 }\n",
			);
		}
		if !self.packed_mat4x3_members.is_empty() {
			// MSL has no packed matrix type. Keep native float4x3 values in expressions and
			// convert only where a logical mat4x3f crosses a buffer-storage boundary.
			msl_block.push_str(
				"struct _besl_packed_float4x3 { packed_float3 columns[4]; };\n\
				 inline float4x3 _besl_load_mat4x3(const thread _besl_packed_float4x3& value) { return float4x3(value.columns[0], value.columns[1], value.columns[2], value.columns[3]); }\n\
				 inline float4x3 _besl_load_mat4x3(const device _besl_packed_float4x3& value) { return float4x3(value.columns[0], value.columns[1], value.columns[2], value.columns[3]); }\n\
				 inline float4x3 _besl_load_mat4x3(const constant _besl_packed_float4x3& value) { return float4x3(value.columns[0], value.columns[1], value.columns[2], value.columns[3]); }\n\
				 inline _besl_packed_float4x3 _besl_pack_mat4x3(float4x3 value) { return _besl_packed_float4x3{packed_float3(value[0]), packed_float3(value[1]), packed_float3(value[2]), packed_float3(value[3])}; }\n\
				 inline void _besl_store_mat4x3(thread _besl_packed_float4x3& target, float4x3 value) { target = _besl_pack_mat4x3(value); }\n\
				 inline void _besl_store_mat4x3(device _besl_packed_float4x3& target, float4x3 value) { target.columns[0] = packed_float3(value[0]); target.columns[1] = packed_float3(value[1]); target.columns[2] = packed_float3(value[2]); target.columns[3] = packed_float3(value[3]); }\n",
			);
		}
		if requirements.uses_atomic_compare_exchange {
			// Metal returns compare-exchange success as a bool, so these helpers preserve BESL's previous-value contract.
			for (value, space) in [
				("uint", "device"),
				("uint", "threadgroup"),
				("int", "device"),
				("int", "threadgroup"),
			] {
				let _ = write!(
					msl_block,
					"inline {value} _besl_atomic_compare_exchange({space} atomic_{value}& value, {value} expected, {value} desired) {{\n\
					 \t{value} original = expected;\n\
					 \twhile (!atomic_compare_exchange_weak_explicit(&value, &expected, desired, memory_order_relaxed, memory_order_relaxed)) {{\n\
					 \t\tif (expected != original) {{ return expected; }}\n\
					 \t}}\n\
					 \treturn original;\n\
					 }}\n"
				);
			}
		}
		if requirements.uses_sincos {
			// Metal's two-result intrinsic returns sine and writes cosine through the second argument.
			msl_block.push_str(
				"inline float2 _besl_sincos(float value) {\n\
				 \tfloat cosine;\n\
				 \tfloat sine = sincos(value, cosine);\n\
				 \treturn float2(sine, cosine);\n\
				 }\n",
			);
		}
		if requirements.uses_find_lsb {
			// Metal's ctz returns 32 for zero; BESL returns 0xFFFFFFFF like GLSL findLSB and HLSL firstbitlow.
			msl_block.push_str("inline uint _besl_find_lsb(uint value) { return value == 0u ? 0xffffffffu : ctz(value); }\n");
		}
		if requirements.uses_subgroup_intrinsics {
			// Metal exposes ballot bits through simd_vote; unused high words preserve BESL's fixed 128-bit mask shape.
			msl_block.push_str(
				"inline uint4 _besl_subgroup_ballot(bool predicate) { ulong vote = ulong(simd_vote::vote_t(simd_ballot(predicate))); return uint4(uint(vote), uint(vote >> 32), 0u, 0u); }\n\
				 inline bool _besl_subgroup_ballot_any(uint4 mask) { return any(mask != uint4(0u, 0u, 0u, 0u)); }\n\
				 inline uint _besl_subgroup_ballot_find_lsb(uint4 mask) { if (mask.x != 0u) { return ctz(mask.x); } if (mask.y != 0u) { return 32u + ctz(mask.y); } if (mask.z != 0u) { return 64u + ctz(mask.z); } if (mask.w != 0u) { return 96u + ctz(mask.w); } return 0xffffffffu; }\n\
				 inline uint _besl_subgroup_ballot_count(uint4 mask) { return popcount(mask.x) + popcount(mask.y) + popcount(mask.z) + popcount(mask.w); }\n\
				 inline uint4 _besl_subgroup_ballot_and_not(uint4 mask, uint4 removed) { return mask & ~removed; }\n\
					 inline uint _besl_subgroup_broadcast_u32(uint value, uint source_lane) { return simd_broadcast(value, ushort(source_lane)); }\n\
					 inline float _besl_subgroup_broadcast_f32(float value, uint source_lane) { return simd_broadcast(value, ushort(source_lane)); }\n\
					 inline float _besl_subgroup_shuffle_xor_f32(float value, uint mask) { return simd_shuffle_xor(value, ushort(mask)); }\n",
			);
		}

		match compilation_settings.stage {
			Stages::Vertex => msl_block.push_str("// #pragma shader_stage(vertex)\n"),
			Stages::Fragment => msl_block.push_str("// #pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => msl_block.push_str("// #pragma shader_stage(compute)\n"),
			Stages::Task { .. } => msl_block.push_str("// #pragma shader_stage(object)\n"),
			Stages::Mesh { .. } => msl_block.push_str("// #pragma shader_stage(mesh)\n"),
		}

		if let Some(local_size) = compilation_settings.stage.local_size() {
			let _ = writeln!(
				msl_block,
				"// besl-threadgroup-size:{},{},{}",
				local_size.width(),
				local_size.height(),
				local_size.depth()
			);
			if matches!(compilation_settings.stage, Stages::Compute { .. }) {
				msl_block.push_str("// Note: Metal threadgroup sizes are set on the pipeline state.\n");
			}
		}

		msl_block.push_str("// Matrix layout: row major\n");

		msl_block.push_str("constant float PI = 3.14159265359;");

		msl_block.push_str(ShaderFormatting::new(self.minified).break_str());
	}
}
