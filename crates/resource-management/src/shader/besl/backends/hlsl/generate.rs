use super::*;
impl Generator {
	/// Generates an HLSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The shader compilation settings.
	/// * `main_function_node` - The shader's main function node.
	///
	/// # Returns
	///
	/// The HLSL shader as a string.
	///
	/// # Panics
	///
	/// Panics if the main function node is not a function node.
	pub fn generate(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<String, ()> {
		self.current_stage = match shader_compilation_settings.stage {
			Stages::Vertex => HlslStage::Vertex,
			Stages::Fragment => HlslStage::Fragment,
			Stages::Compute { .. } => HlslStage::Compute,
			Stages::Task { .. } => HlslStage::Task,
			Stages::Mesh { .. } => HlslStage::Mesh,
		};
		// Only fragment inputs and raster-producing outputs participate in interpolation.
		self.current_stage_interpolates_inputs = matches!(shader_compilation_settings.stage, Stages::Fragment);
		self.current_stage_interpolates_outputs =
			matches!(shader_compilation_settings.stage, Stages::Vertex | Stages::Mesh { .. });
		self.current_local_size = match shader_compilation_settings.stage {
			Stages::Compute { local_size } | Stages::Task { local_size, .. } | Stages::Mesh { local_size, .. } => {
				Some(local_size)
			}
			_ => None,
		};
		(self.current_mesh_maximum_vertices, self.current_mesh_maximum_primitives) = match shader_compilation_settings.stage {
			Stages::Mesh {
				maximum_vertices,
				maximum_primitives,
				..
			} => (maximum_vertices, maximum_primitives),
			_ => (0, 0),
		};
		let mut string = String::with_capacity(2048);
		let order = ordered_shader_nodes(main_function_node, "HLSL");
		crate::shader::generator::validate_workgroup_storage_stage(&shader_compilation_settings.stage, &order)?;
		crate::shader::generator::validate_vertex_builtin_inputs(&shader_compilation_settings.stage, &order)?;
		let uses_subgroup_intrinsics = Self::uses_subgroup_intrinsics(&order);
		if uses_subgroup_intrinsics && self.current_stage != HlslStage::Compute {
			return Err(());
		}
		self.mesh_uses_render_target_array_index = order
			.iter()
			.any(|node| Self::uses_intrinsic(node, "set_mesh_primitive_render_target_array_index"));
		self.task_payloads.clear();
		self.mesh_outputs.clear();
		self.raster_inputs.clear();
		self.raster_outputs.clear();
		self.packed_write_counter = 0;
		for node in &order {
			match node.borrow().node() {
				besl::Nodes::TaskPayload { .. } => self.task_payloads.push(node.clone()),
				besl::Nodes::Output { count: Some(_), .. } => self.mesh_outputs.push(node.clone()),
				besl::Nodes::Input { .. } => self.raster_inputs.push(node.clone()),
				besl::Nodes::Output { count: None, .. } => self.raster_outputs.push(node.clone()),
				_ => {}
			}
		}
		self.user_struct_constructors.clear();
		// Discover constructor calls before declarations are emitted so their HLSL factories can stay next to each struct.
		for node in &order {
			self.emit_node_string(&mut string, node);
		}
		string.clear();

		self.generate_hlsl_header_block(&mut string, shader_compilation_settings, uses_subgroup_intrinsics);
		if self.current_stage == HlslStage::Task {
			string.push_str("groupshared uint32_t besl_mesh_output_count;");
			if !self.minified {
				string.push('\n');
			}
		}
		if self.current_stage == HlslStage::Mesh {
			self.emit_mesh_output_structs(&mut string);
		}

		for node in order {
			self.emit_node_string(&mut string, &node);
		}

		Ok(string)
	}

	/// Emits one user struct and its factory when the program constructs that type.
	pub(crate) fn emit_hlsl_struct_node(
		&mut self,
		string: &mut String,
		node: &besl::NodeReference,
		name: &str,
		fields: &[besl::NodeReference],
		template: &Option<besl::NodeReference>,
	) {
		self.emit_struct_node(string, name, fields, template);
		if template.is_none()
			&& !crate::shader::generator::is_builtin_struct_type(name, self.supports_atomic_u32())
			&& self.user_struct_constructors.contains(node)
		{
			self.emit_hlsl_struct_factory(string, name, fields);
		}
	}

	/// Emits an amplification entry point with the group-shared payload required by `DispatchMesh`.
	pub(crate) fn emit_hlsl_task_entry(
		&mut self,
		string: &mut String,
		node: &besl::NodeReference,
		statements: &[besl::NodeReference],
		return_type: &besl::NodeReference,
		params: &[besl::NodeReference],
	) {
		let formatting = ShaderFormatting::new(self.minified);
		if !self.task_payloads.is_empty() {
			// Every amplification lane contributes to one payload, so it must use group-shared storage.
			string.push_str("groupshared ObjectPayload payload;");
			if !self.minified {
				string.push('\n');
			}
		}
		self.emit_function_attributes(string, node, "besl_main");
		Self::emit_type_name(string, return_type.borrow().get_name().unwrap());
		string.push_str(" besl_main(");
		emit_comma_separated_nodes(string, formatting, params, |string, parameter| {
			self.emit_node_string(string, parameter)
		});
		self.emit_function_extra_parameters(string, node, "besl_main", !params.is_empty());
		formatting.push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);
		if !self.task_payloads.is_empty() {
			// DXIL requires DispatchMesh to dominate the entry point, so every lane converges after BESL selects the count.
			formatting.push_indentation(string, 1);
			string.push_str("GroupMemoryBarrierWithGroupSync()");
			formatting.push_statement_end(string);
			formatting.push_indentation(string, 1);
			string.push_str("DispatchMesh(besl_mesh_output_count, 1, 1, payload)");
			formatting.push_statement_end(string);
		}
		self.emit_block_end(string);
	}

	/// Emits a field-by-field factory because DXC does not support user-defined struct constructor expressions.
	pub(crate) fn emit_hlsl_struct_factory(&mut self, string: &mut String, name: &str, fields: &[besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		string.push_str(name);
		string.push_str(" besl_construct_");
		string.push_str(name);
		string.push('(');
		for (index, field) in fields.iter().enumerate() {
			let field = field.borrow();
			let besl::Nodes::Member {
				name: field_name,
				r#type,
				count,
			} = field.node()
			else {
				continue;
			};
			if index > 0 {
				string.push_str(formatting.comma_str());
			}
			Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
			string.push_str(" besl_argument_");
			string.push_str(field_name);
			if let Some(count) = count {
				string.push('[');
				string.push_str(&count.to_string());
				string.push(']');
			}
		}
		formatting.push_block_start(string);

		formatting.push_indentation(string, 1);
		string.push_str(name);
		string.push_str(" besl_value");
		formatting.push_statement_end(string);
		for field in fields {
			let field = field.borrow();
			let besl::Nodes::Member {
				name: field_name, count, ..
			} = field.node()
			else {
				continue;
			};

			if let Some(count) = count {
				formatting.push_indentation(string, 1);
				string.push_str("[unroll] for(uint besl_index=0;besl_index<");
				string.push_str(&count.to_string());
				string.push_str(";++besl_index){");
				string.push_str("besl_value.");
				string.push_str(field_name);
				string.push_str("[besl_index]=besl_argument_");
				string.push_str(field_name);
				string.push_str("[besl_index];}");
				if !self.minified {
					string.push('\n');
				}
			} else {
				formatting.push_indentation(string, 1);
				string.push_str("besl_value.");
				string.push_str(field_name);
				string.push_str("=besl_argument_");
				string.push_str(field_name);
				formatting.push_statement_end(string);
			}
		}
		formatting.push_indentation(string, 1);
		string.push_str("return besl_value");
		formatting.push_statement_end(string);
		self.emit_block_end(string);
	}

	/// Translates BESL intrinsic type names to HLSL type names, such as `vec2f` to `float2`.
	pub(crate) fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"vec2f16" => "float16_t2",
			"vec3f16" => "float16_t3",
			"vec4f16" => "float16_t4",
			"vec2f" => "float2",
			"vec2u" => "uint2",
			"vec2i" => "int2",
			"vec2u16" => "uint16_t2",
			"vec3u16" => "uint16_t3",
			"vec4u16" => "uint16_t4",
			"vec3u" => "uint3",
			"vec4u" => "uint4",
			"vec3f" => "float3",
			"vec4f" => "float4",
			"packed_vec4f" => "float4",
			"mat2f" => "float2x2",
			"mat3f" => "float3x3",
			"mat4f" => "float4x4",
			"mat4x3f" => "float4x3",
			"f16" => "float16_t",
			"f32" => "float",
			"u8" => "uint",
			"u16" => "uint",
			"u32" => "uint32_t",
			"atomicu32" => "uint32_t",
			"i32" => "int32_t",
			"Texture2D" => "Texture2D",
			"Texture3D" => "Texture3D",
			"TextureCube" => "TextureCube<float4>",
			"TextureCubeArray" => "TextureCubeArray<float4>",
			"ArrayTexture2D" => "Texture2DArray<float4>",
			_ => source,
		}
	}

	/// Reports whether a backend type needs non-interpolated raster-stage I/O.
	pub(crate) fn is_integer_type(type_name: &str) -> bool {
		matches!(
			type_name,
			"int8_t"
				| "uint8_t" | "int16_t"
				| "uint16_t" | "int"
				| "int32_t" | "uint"
				| "uint32_t" | "int64_t"
				| "uint64_t" | "int2"
				| "uint2" | "uint3"
				| "uint4" | "uint16_t2"
				| "uint16_t4"
		)
	}

	/// Emits one specialization aggregate as plain HLSL constants for DXC.
	fn emit_specialization_node(&self, string: &mut String, name: &str, r#type: &besl::NodeReference) {
		let mut members = Vec::new();
		let r#type = r#type.borrow();
		let type_name = Self::translate_type(r#type.get_name().unwrap());

		if let besl::Nodes::Struct { fields, .. } = r#type.node() {
			for field in fields {
				let field = field.borrow();
				let besl::Nodes::Member {
					name: member_name,
					r#type,
					..
				} = field.node()
				else {
					continue;
				};
				let member_name = format!("{name}_{member_name}");
				string.push_str("static const ");
				string.push_str(Self::translate_type(r#type.borrow().get_name().unwrap()));
				string.push(' ');
				string.push_str(&member_name);
				string.push_str("=1.0f;");
				if !self.minified {
					string.push('\n');
				}
				members.push(member_name);
			}
		}

		string.push_str("static const ");
		string.push_str(type_name);
		string.push(' ');
		string.push_str(name);
		string.push('=');
		string.push_str(&format!("{}({})", type_name, members.join(",")));
		string.push(';');
		if !self.minified {
			string.push('\n');
		}
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	// Keep the exhaustive node-to-HLSL mapping together so adding a BESL node requires handling its backend contract here.
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
			} => {
				let hlsl_name = if name == "main" { "besl_main" } else { name };
				if hlsl_name == "besl_main" && self.current_stage == HlslStage::Task {
					self.emit_hlsl_task_entry(string, this_node, statements, return_type, params);
				} else {
					self.emit_function_node(string, this_node, hlsl_name, statements, return_type, params);
				}
			}
			besl::Nodes::Struct {
				name, fields, template, ..
			} => self.emit_hlsl_struct_node(string, this_node, name, fields, template),
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment && self.emit_atomic_add_assignment(string, left, right) => {}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment
					&& self.emit_atomic_compare_exchange_assignment(string, left, right) => {}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment && self.emit_image_size_assignment(string, left, right) => {}
			besl::Nodes::PushConstant { members } => {
				// Root constants use the constant-buffer namespace, while flat resources use t/u/s registers in space 0.
				if self.minified {
					string.push_str("struct PushConstant{");
				} else {
					string.push_str("// Root constants\n");
					string.push_str("struct PushConstant {\n");
				}

				for member in members {
					formatting.push_indentation(string, 1);
					self.emit_node_string(string, member);
					formatting.push_statement_end(string);
				}

				if self.minified {
					string.push_str("};ConstantBuffer<PushConstant> push_constant : register(b0, space0);");
				} else {
					string.push_str("};\n");
					string.push_str("ConstantBuffer<PushConstant> push_constant : register(b0, space0);\n");
				}
			}
			// DXC treats Vulkan specialization attributes as resource metadata, so use plain HLSL constants.
			besl::Nodes::Specialization { name, r#type } => self.emit_specialization_node(string, name, r#type),
			besl::Nodes::Member { name, r#type, count } => {
				if let Some(type_name) = r#type.borrow().get_name() {
					let type_name = Self::translate_type(type_name);

					string.push_str(type_name);
					string.push(' ');
				}
				string.push_str(name.as_str());
				if let Some(count) = count {
					string.push('[');
					string.push_str(count.to_string().as_str());
					string.push(']');
				}
			}
			besl::Nodes::Raw { glsl, hlsl, .. } => {
				// Use HLSL code if available, otherwise fall back to GLSL
				if let Some(code) = hlsl {
					string.push_str(code);
				} else if let Some(code) = glsl {
					// Fall back to GLSL code (may need translation for HLSL-specific features)
					string.push_str(code);
				}
			}
			besl::Nodes::Parameter { name, r#type } => self.emit_parameter_node(string, name, r#type),
			besl::Nodes::Input { name, location, format } => {
				if matches!(self.current_stage, HlslStage::Vertex | HlslStage::Fragment) {
					return;
				}
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());

				// HLSL uses semantics like TEXCOORD0, TEXCOORD1, etc.
				string.push_str(&format!(
					"{}{} {} : TEXCOORD{};{break_char}",
					if self.current_stage_interpolates_inputs && Self::is_integer_type(type_name) {
						"nointerpolation "
					} else {
						""
					},
					type_name,
					name,
					location
				));
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
				if matches!(self.current_stage, HlslStage::Vertex | HlslStage::Fragment) {
					return;
				}
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());

				// HLSL uses SV_Target0, SV_Target1, etc. for render targets
				string.push_str(&format!(
					"{}{} {} : SV_Target{};{break_char}",
					if self.current_stage_interpolates_outputs && Self::is_integer_type(type_name) {
						"nointerpolation "
					} else {
						""
					},
					type_name,
					name,
					location
				));
			}
			besl::Nodes::TaskPayload { .. } => {
				if self.task_payloads.first() == Some(this_node) {
					self.emit_object_payload_struct(string);
				}
			}
			besl::Nodes::Workgroup { name, format, count } => {
				string.push_str("groupshared ");
				string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
				string.push(' ');
				string.push_str(name);
				if let Some(count) = count {
					string.push('[');
					string.push_str(&count.to_string());
					string.push(']');
				}
				string.push(';');
				if !self.minified {
					string.push('\n');
				}
			}
			besl::Nodes::Expression(expression) => self.emit_expression_node(string, expression),
			besl::Nodes::Conditional { statements, .. }
				if self.current_stage == HlslStage::Mesh && Self::mesh_output_count_arguments(statements).is_some() =>
			{
				let (vertices, primitives) = Self::mesh_output_count_arguments(statements).unwrap();
				// DXIL requires SetMeshOutputCounts to dominate every mesh output, so remove BESL's portable lane-zero guard.
				string.push_str("SetMeshOutputCounts(");
				self.emit_node_string(string, &vertices);
				self.emit_separator(string);
				self.emit_node_string(string, &primitives);
				string.push(')');
			}
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
				r#type,
				count,
				..
			} => {
				// HLSL preserves the flat slot in the matching register namespace and always uses space 0.
				let register_index = *slot;
				let read_only = *read && !*write;
				let buffer_type = if read_only { "StructuredBuffer" } else { "RWStructuredBuffer" };
				let register_type = if read_only { "t" } else { "u" };

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						if let Some((member_name, element_type)) = Self::hlsl_flattened_array_member(members) {
							string.push_str(buffer_type);
							string.push('<');
							string.push_str(Self::translate_type(&element_type));
							string.push_str("> ");
							string.push_str(name);
							if let Some(count) = count {
								string.push('[');
								string.push_str(count.to_string().as_str());
								string.push(']');
							}
							string.push_str(&format!(" : register({register_type}{register_index}, space0);"));
							if !self.minified {
								string.push('\n');
							}
							let _ = member_name;
							return;
						}

						self.emit_named_struct_start(string, &format!("_{name}"));

						for member in members.iter() {
							self.emit_indentation(string, 1);
							self.emit_node_string(string, member);
							self.emit_statement_end(string);
						}

						if self.minified {
							string.push_str("};");
						} else {
							string.push_str("};\n");
						}

						string.push_str(&format!("{buffer_type}<_{name}> "));
						string.push_str(name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" : register({register_type}{register_index}, space0);"));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::BufferArray { element } => {
						string.push_str(buffer_type);
						string.push('<');
						string.push_str(Self::translate_type(element.borrow().get_name().unwrap()));
						string.push_str("> ");
						string.push_str(name);
						string.push_str(&format!(" : register({register_type}{register_index}, space0);"));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::Image { format } => {
						// UAV (unordered access view) for images
						let texture_type = match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "RWTexture2D<uint>",
							_ => "RWTexture2D<float4>",
						};

						string.push_str(texture_type);
						string.push(' ');
						string.push_str(name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" : register(u{register_index}, space0);"));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::CombinedImageSampler { format } => {
						// HLSL separates textures and samplers, but for combined sampler we use Texture2D
						let texture_type = match format.as_str() {
							"Texture3D" => "Texture3D",
							"TextureCube" => "TextureCube",
							"TextureCubeArray" => "TextureCubeArray",
							"ArrayTexture2D" => "Texture2DArray",
							_ => "Texture2D",
						};

						string.push_str(texture_type);
						string.push_str(match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "<uint>",
							_ => "<float4>",
						});
						string.push(' ');
						string.push_str(name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" : register(t{register_index}, space0);"));
						if !self.minified {
							string.push('\n');
						}

						// Also declare a sampler with the same name + _sampler suffix
						string.push_str("SamplerState ");
						string.push_str(name);
						string.push_str("_sampler");
						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}
						string.push_str(&format!(" : register(s{register_index}, space0);"));
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
				self.emit_const_node(string, name, r#type, value);
			}
		}
	}

	pub(crate) fn generate_hlsl_header_block(
		&self,
		hlsl_block: &mut String,
		compilation_settings: &ShaderGenerationSettings,
		uses_subgroup_intrinsics: bool,
	) {
		// HLSL doesn't use #version, but we can add shader model target as a comment
		hlsl_block.push_str("// Shader Model 6.0+\n");

		// Shader type as comment (user preference: Option B)
		match compilation_settings.stage {
			Stages::Vertex => hlsl_block.push_str("// #pragma shader_stage(vertex)\n"),
			Stages::Fragment => hlsl_block.push_str("// #pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => hlsl_block.push_str("// #pragma shader_stage(compute)\n"),
			Stages::Task { .. } => hlsl_block.push_str("// #pragma shader_stage(amplification)\n"),
			Stages::Mesh { .. } => hlsl_block.push_str("// #pragma shader_stage(mesh)\n"),
		}

		// Feature requirements (Option A & C: skip most, add specific where applicable)
		// HLSL SM 6.0+ has most features built-in, so we mainly document what's expected
		hlsl_block.push_str("// Requires: 16-bit types, explicit arithmetic types\n");

		match compilation_settings.stage {
			Stages::Compute { .. } => {
				hlsl_block.push_str("// Requires: Wave intrinsics (WaveGetLaneCount, WaveGetLaneIndex, etc.)\n");
			}
			Stages::Mesh { .. } => {
				hlsl_block.push_str("// Requires: Mesh shader support\n");
			}
			Stages::Task { .. } => hlsl_block.push_str("// Requires: Amplification shader support\n"),
			_ => {}
		}

		// Matrix layout
		match compilation_settings.matrix_layout {
			MatrixLayouts::RowMajor => hlsl_block.push_str("#pragma pack_matrix(row_major)\n"),
			MatrixLayouts::ColumnMajor => hlsl_block.push_str("#pragma pack_matrix(column_major)\n"),
		}

		// Constants
		hlsl_block.push_str("static const float PI = 3.14159265359;");

		if !self.minified {
			hlsl_block.push('\n');
		}
		if uses_subgroup_intrinsics {
			hlsl_block.push_str(
				"bool _besl_subgroup_ballot_any(uint4 mask) { return any(mask); }\n\
				 uint _besl_subgroup_ballot_find_lsb(uint4 mask) { if (mask.x != 0u) { return firstbitlow(mask.x); } if (mask.y != 0u) { return 32u + firstbitlow(mask.y); } if (mask.z != 0u) { return 64u + firstbitlow(mask.z); } if (mask.w != 0u) { return 96u + firstbitlow(mask.w); } return 0xffffffffu; }\n\
				 uint _besl_subgroup_ballot_count(uint4 mask) { return countbits(mask.x) + countbits(mask.y) + countbits(mask.z) + countbits(mask.w); }\n\
				 uint4 _besl_subgroup_ballot_and_not(uint4 mask, uint4 removed) { return mask & ~removed; }\n",
			);
		}
	}

	/// Emits the 32-bit word containing one packed logical narrow-buffer element.
	pub(crate) fn emit_packed_word_access_by_name(
		&self,
		string: &mut String,
		binding_name: &str,
		index_name: &str,
		elements_per_word: u32,
	) {
		string.push_str(binding_name);
		string.push('[');
		string.push_str(index_name);
		let _ = write!(string, "/{elements_per_word}u]");
	}
}
