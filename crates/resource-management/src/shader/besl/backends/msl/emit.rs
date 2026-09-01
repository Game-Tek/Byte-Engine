use super::*;

mod intrinsics;
impl<A: Allocator + Clone> Generator<A> {
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
		string.push_str(name);
		string.push('(');

		let formatting = ShaderFormatting::new(self.minified);
		emit_comma_separated_nodes(string, formatting, params, |string, param| {
			self.emit_node_string(string, param)
		});

		if self.task_stage_context.is_some() {
			self.emit_task_hidden_parameters(string, !params.is_empty());
		} else if self.in_compute_body {
			let uses_simd_lane_id = Self::uses_intrinsic(function_node, "subgroup_lane_index");
			if uses_simd_lane_id || self.function_requires_resource_context(function_node, true) {
				self.emit_compute_hidden_parameters(string, !params.is_empty(), uses_simd_lane_id);
			}
		} else if self
			.raster_stage_context
			.as_ref()
			.is_some_and(|context| context.has_vertex_builtins())
			|| self.raster_stage_context.is_some() && self.function_requires_resource_context(function_node, false)
		{
			self.emit_raster_hidden_parameters(string, !params.is_empty());
		}

		string.push(')');
		self.emit_statement_end(string);
	}

	/// Extracts one primitive field write so adjacent writes can become one native primitive value.
	pub(crate) fn mesh_primitive_write_parts(
		&mut self,
		statement: &besl::NodeReference,
	) -> Option<(String, besl::NodeReference, besl::NodeReference)> {
		let node = statement.borrow();
		if let besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) = node.node()
		{
			let [index, array_index] = arguments.as_slice() else {
				return None;
			};
			if intrinsic.borrow().get_name() == Some("set_mesh_primitive_render_target_array_index") {
				return Some(("render_target_array_index".to_string(), index.clone(), array_index.clone()));
			}
			return None;
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

		let output_node = output.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { source, .. }) = output_node.node() else {
			return None;
		};

		let source = source.borrow();
		let besl::Nodes::Output { name, count, .. } = source.node() else {
			return None;
		};

		if count.is_none() {
			return None;
		}

		Some((Self::mesh_output_field_name(name).to_string(), index.clone(), right.clone()))
	}

	/// Returns one primitive field's native declaration position for Metal aggregate initialization.
	pub(crate) fn mesh_primitive_field_order(&self, field: &str) -> usize {
		if field == "render_target_array_index" {
			return 0;
		}
		self.mesh_stage_context
			.as_ref()
			.and_then(|context| context.primitive_output_fields.iter().position(|declared| declared == field))
			.map_or(usize::MAX, |index| index + 1)
	}

	pub(crate) fn emit_statement_block(&mut self, string: &mut String, statements: &[besl::NodeReference], indent: usize) {
		let formatting = ShaderFormatting::new(self.minified);
		let mut i = 0;

		while i < statements.len() {
			if self.mesh_stage_context.is_some()
				&& let Some((field, index, value)) = self.mesh_primitive_write_parts(&statements[i])
			{
				let mut index_string = String::new();
				self.emit_node_string(&mut index_string, &index);
				let mut writes = vec![(field, value)];
				let mut next = i + 1;

				while next < statements.len() {
					let Some((field, next_index, value)) = self.mesh_primitive_write_parts(&statements[next]) else {
						break;
					};
					let mut next_index_string = String::new();
					self.emit_node_string(&mut next_index_string, &next_index);
					if next_index_string != index_string || writes.iter().any(|(written, _)| written == &field) {
						break;
					}
					writes.push((field, value));
					next += 1;
				}
				// Metal requires designated initializers to follow the PrimitiveOutput declaration order.
				writes.sort_by_key(|(field, _)| self.mesh_primitive_field_order(field));

				formatting.push_indentation(string, indent);
				string.push_str("out_mesh.set_primitive(");
				self.emit_node_string(string, &index);
				string.push_str(", PrimitiveOutput{");
				for (write_index, (field, value)) in writes.iter().enumerate() {
					if write_index > 0 {
						string.push_str(", ");
					}
					string.push('.');
					string.push_str(field);
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
			"packed_vec4f" => "packed_float4",
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
			besl::Nodes::Null => {}
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
				self.emit_named_struct_start(string, "PushConstant");

				for member in members {
					formatting.push_indentation(string, 1);
					self.emit_node_string(string, member);
					formatting.push_statement_end(string);
				}

				self.emit_struct_declaration_end(string);

				// TODO: Confirm push constant mapping for Metal argument buffers.
				if self.minified {
					string.push_str(&format!(
						"constant PushConstant& push_constant [[buffer({})]];",
						PUSH_CONSTANT_BINDING_INDEX
					));
				} else {
					string.push_str(&format!(
						"constant PushConstant& push_constant [[buffer({})]];\n",
						PUSH_CONSTANT_BINDING_INDEX
					));
				}
			}
			besl::Nodes::TaskPayload { .. } | besl::Nodes::Workgroup { .. } => {}
			besl::Nodes::Specialization { name, r#type } => {
				let mut members = Vec::new();

				let r#type = r#type.borrow();

				let t = r#type.get_name().unwrap();
				let type_name = Self::translate_type(t);

				if let besl::Nodes::Struct { fields, .. } = r#type.node() {
					for (i, field) in fields.iter().enumerate() {
						if let besl::Nodes::Member {
							name: member_name,
							r#type,
							..
						} = field.borrow().node()
						{
							let member_name = format!("{}_{}", name, { member_name });
							string.push_str(&format!(
								"constant {} {} [[function_constant({})]];{}",
								Self::translate_type(r#type.borrow().get_name().unwrap()),
								member_name,
								i,
								if !self.minified { "\n" } else { "" }
							));
							members.push(member_name);
						}
					}
				}

				string.push_str(&format!(
					"constant {} {}={};{}",
					type_name,
					name,
					format!("{}({})", &type_name, members.join(",")),
					if !self.minified { "\n" } else { "" }
				));
			}
			besl::Nodes::Member { name, r#type, count } => {
				if let Some(type_name) = r#type.borrow().get_name() {
					if self.is_packed_mat4x3_member(this_node) {
						string.push_str(Self::translate_buffer_member_type(type_name));
					} else if self.in_buffer_binding_struct
						&& (count.is_some() || matches!(type_name, "vec2f16" | "vec3f16" | "vec4f16" | "vec2u16" | "vec4u16"))
					{
						string.push_str(Self::translate_buffer_member_type(type_name));
					} else if type_name.contains('[') {
						Self::emit_type_name(string, type_name);
					} else {
						string.push_str(Self::translate_type(type_name));
					}
					string.push(' ');
				}
				string.push_str(name.as_str());
				if let Some(count) = count {
					string.push('[');
					string.push_str(count.to_string().as_str());
					string.push(']');
				}
			}
			besl::Nodes::Raw { glsl, hlsl, msl, .. } => {
				if let Some(code) = msl.as_ref().or(hlsl.as_ref()).or(glsl.as_ref()) {
					string.push_str(code);
				}
			}
			besl::Nodes::Parameter { name, r#type } => self.emit_parameter_node(string, name, r#type),
			besl::Nodes::Input { name, location, format } => {
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
				// TODO: Map interpolation qualifiers to Metal (flat/linear).
				string.push_str(&format!("{} {} [[attribute({})]];{break_char}", type_name, name, location));
			}
			besl::Nodes::Output {
				name,
				location,
				format,
				count,
			} => {
				if count.is_some() {
					return;
				}

				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
				string.push_str(&format!("{} {} [[color({})]];{break_char}", type_name, name, location));
			}
			besl::Nodes::Expression(expression) => self.emit_expression_node(string, expression),
			besl::Nodes::Conditional { condition, statements } => self.emit_conditional_node(string, condition, statements),
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
					self.emit_compute_binding_reference(string, name);
					return;
				}

				let index = *slot;

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						self.emit_named_struct_start(string, &format!("_{name}"));

						for member in members.iter() {
							self.emit_indentation(string, 1);
							self.emit_node_string(string, member);
							self.emit_statement_end(string);
						}

						self.emit_struct_declaration_end(string);

						let address_space = buffer_address_space(*memory_class, *write);

						string.push_str(address_space);
						string.push(' ');
						string.push_str(&format!("_{}* {}", name, name));

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[buffer({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::BufferArray { element } => {
						let address_space = buffer_address_space(*memory_class, *write);
						string.push_str(address_space);
						string.push(' ');
						string.push_str(Self::translate_type(element.borrow().get_name().unwrap()));
						string.push_str("* ");
						string.push_str(name);
						string.push_str(&format!(" [[buffer({index})]];"));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::Image { format } => {
						let element_type = match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "uint",
							_ => "float",
						};

						let access = if *read && *write {
							"access::read_write"
						} else if *write {
							"access::write"
						} else {
							"access::read"
						};

						string.push_str(&format!("texture2d<{}, {}> {}", element_type, access, name));

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[texture({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::CombinedImageSampler { format } => {
						let texture_type = match format.as_str() {
							"ArrayTexture2D" => "texture2d_array<float>",
							"TextureCube" => "texturecube<float>",
							"TextureCubeArray" => "texturecube_array<float>",
							"r8ui" | "r16ui" | "r32ui" => "texture2d<uint>",
							_ => "texture2d<float>",
						};

						string.push_str(texture_type);
						string.push(' ');
						string.push_str(name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[texture({})]];", index));
						if !self.minified {
							string.push('\n');
						}

						string.push_str("sampler ");
						string.push_str(&format!("{}_sampler", name));
						string.push_str(&format!(" [[sampler({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
				}
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node_string(string, element);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				self.emit_node_string(string, value);
			}
			besl::Nodes::Const { name, r#type, value } => {
				string.push_str("constant ");
				let type_name = r#type.borrow().get_name().unwrap().to_string();
				let short_scalar_array = crate::shader::generator::scalar_array_vector_type(&type_name);
				if let Some(vector_type) = short_scalar_array {
					string.push_str(Self::translate_type(vector_type));
					string.push(' ');
					string.push_str(name);
				} else if let Some((element_type, count)) = type_name.split_once('[') {
					string.push_str(Self::translate_type(element_type));
					string.push(' ');
					string.push_str(name);
					string.push('[');
					string.push_str(count.trim_end_matches(']'));
					string.push(']');
				} else {
					Self::emit_type_name(string, &type_name);
					string.push(' ');
					string.push_str(name);
				}
				string.push_str(" = ");
				if let besl::Nodes::Expression(besl::Expressions::FunctionCall {
					parameters, function, ..
				}) = value.borrow().node()
				{
					if short_scalar_array.is_none() && function.borrow().get_name() == Some(type_name.as_str()) {
						string.push('{');
						self.emit_call_arguments(string, parameters);
						string.push('}');
					} else {
						self.emit_node_string(string, value);
					}
				} else {
					self.emit_node_string(string, value);
				}
				string.push_str(&format!(";{break_char}"));
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
		if self.downsample_strategy == DownsampleStrategy::ShaderGather
			&& (requirements.uses_downsample_min || requirements.uses_downsample_max)
		{
			// Metal gather has no explicit-LOD overload. Use it for mip zero and preserve explicit
			// pyramid levels with four reads. Native reduction needs no fallback source.
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
			msl_block.push_str(
				"inline uint _besl_atomic_compare_exchange(device atomic_uint& value, uint expected, uint desired) {\n\
				 \tuint original = expected;\n\
				 \twhile (!atomic_compare_exchange_weak_explicit(&value, &expected, desired, memory_order_relaxed, memory_order_relaxed)) {\n\
				 \t\tif (expected != original) { return expected; }\n\
				 \t}\n\
				 \treturn original;\n\
				 }\n\
				 inline uint _besl_atomic_compare_exchange(threadgroup atomic_uint& value, uint expected, uint desired) {\n\
				 \tuint original = expected;\n\
				 \twhile (!atomic_compare_exchange_weak_explicit(&value, &expected, desired, memory_order_relaxed, memory_order_relaxed)) {\n\
				 \t\tif (expected != original) { return expected; }\n\
				 \t}\n\
				 \treturn original;\n\
				 }\n\
				 inline int _besl_atomic_compare_exchange(device atomic_int& value, int expected, int desired) {\n\
				 \tint original = expected;\n\
				 \twhile (!atomic_compare_exchange_weak_explicit(&value, &expected, desired, memory_order_relaxed, memory_order_relaxed)) {\n\
				 \t\tif (expected != original) { return expected; }\n\
				 \t}\n\
				 \treturn original;\n\
				 }\n\
				 inline int _besl_atomic_compare_exchange(threadgroup atomic_int& value, int expected, int desired) {\n\
				 \tint original = expected;\n\
				 \twhile (!atomic_compare_exchange_weak_explicit(&value, &expected, desired, memory_order_relaxed, memory_order_relaxed)) {\n\
				 \t\tif (expected != original) { return expected; }\n\
				 \t}\n\
				 \treturn original;\n\
				 }\n",
			);
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
		if requirements.uses_subgroup_intrinsics {
			// Metal exposes ballot bits through simd_vote; unused high words preserve BESL's fixed 128-bit mask shape.
			msl_block.push_str(
				"inline uint4 _besl_subgroup_ballot(bool predicate) { ulong vote = ulong(simd_vote::vote_t(simd_ballot(predicate))); return uint4(uint(vote), uint(vote >> 32), 0u, 0u); }\n\
				 inline bool _besl_subgroup_ballot_any(uint4 mask) { return any(mask != uint4(0u, 0u, 0u, 0u)); }\n\
				 inline uint _besl_subgroup_ballot_find_lsb(uint4 mask) { if (mask.x != 0u) { return ctz(mask.x); } if (mask.y != 0u) { return 32u + ctz(mask.y); } if (mask.z != 0u) { return 64u + ctz(mask.z); } if (mask.w != 0u) { return 96u + ctz(mask.w); } return 0xffffffffu; }\n\
				 inline uint _besl_subgroup_ballot_count(uint4 mask) { return popcount(mask.x) + popcount(mask.y) + popcount(mask.z) + popcount(mask.w); }\n\
				 inline uint4 _besl_subgroup_ballot_and_not(uint4 mask, uint4 removed) { return mask & ~removed; }\n\
					 inline uint _besl_subgroup_broadcast_u32(uint value, uint source_lane) { return simd_broadcast(value, ushort(source_lane)); }\n\
					 inline float _besl_subgroup_broadcast_f32(float value, uint source_lane) { return simd_broadcast(value, ushort(source_lane)); }\n",
			);
		}

		match compilation_settings.stage {
			Stages::Vertex => msl_block.push_str("// #pragma shader_stage(vertex)\n"),
			Stages::Fragment => msl_block.push_str("// #pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => msl_block.push_str("// #pragma shader_stage(compute)\n"),
			Stages::Task { .. } => msl_block.push_str("// #pragma shader_stage(object)\n"),
			Stages::Mesh { .. } => msl_block.push_str("// #pragma shader_stage(mesh)\n"),
		}

		match compilation_settings.stage {
			Stages::Compute { local_size } => {
				msl_block.push_str(&format!(
					"// besl-threadgroup-size:{},{},{}\n",
					local_size.width().max(1),
					local_size.height().max(1),
					local_size.depth().max(1)
				));
				msl_block.push_str("// Note: Metal threadgroup sizes are set on the pipeline state.\n");
			}
			Stages::Task { local_size, .. } | Stages::Mesh { local_size, .. } => {
				msl_block.push_str(&format!(
					"// besl-threadgroup-size:{},{},{}\n",
					local_size.width().max(1),
					local_size.height().max(1),
					local_size.depth().max(1)
				));
			}
			_ => {}
		}

		match compilation_settings.matrix_layout {
			MatrixLayouts::RowMajor => msl_block.push_str("// Matrix layout: row major\n"),
			MatrixLayouts::ColumnMajor => msl_block.push_str("// Matrix layout: column major\n"),
		}

		msl_block.push_str("constant float PI = 3.14159265359;");

		if !self.minified {
			msl_block.push('\n');
		}
	}
}
