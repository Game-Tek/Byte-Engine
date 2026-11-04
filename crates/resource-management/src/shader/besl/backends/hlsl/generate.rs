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
		self.stage = shader_compilation_settings.stage;
		let mut string = String::with_capacity(2048);
		let order = ordered_shader_nodes(main_function_node, "HLSL");
		crate::shader::generator::validate_workgroup_storage_stage(&shader_compilation_settings.stage, &order)?;
		crate::shader::generator::validate_no_ray_tracing(&shader_compilation_settings.stage, &order)?;
		crate::shader::generator::validate_vertex_builtin_inputs(&shader_compilation_settings.stage, &order)?;
		if order.iter().any(Self::has_unsupported_hlsl_atomic_context) {
			return Err(());
		}
		if order.iter().any(Self::has_misplaced_array_constructor) {
			return Err(());
		}
		let requirements = intrinsic_requirements(&order);
		if requirements.uses_subgroup_intrinsics && !matches!(self.stage, Stages::Compute { .. }) {
			return Err(());
		}
		self.mesh_uses_render_target_array_index = requirements.uses_render_target_array_index;
		self.task_payloads.clear();
		self.mesh_outputs.clear();
		self.raster_inputs.clear();
		self.raster_outputs.clear();
		self.user_struct_constructors.clear();
		self.packed_write_counter = 0;
		self.atomic_temporary_counter = 0;
		self.atomic_temporaries.clear();
		self.plan_byte_windows(&order);
		for node in &order {
			match node.borrow().node() {
				besl::Nodes::TaskPayload { .. } => self.task_payloads.push(node.clone()),
				besl::Nodes::Output { count: Some(_), .. } => self.mesh_outputs.push(node.clone()),
				besl::Nodes::Input { .. } => self.raster_inputs.push(node.clone()),
				besl::Nodes::Output { count: None, .. } => self.raster_outputs.push(node.clone()),
				_ => {}
			}
		}
		self.generate_hlsl_header_block(&mut string, shader_compilation_settings, &requirements);
		emit_set_bases(&mut string, &order, ShaderFormatting::new(self.minified).break_str())?;
		if matches!(self.stage, Stages::Task { .. }) {
			string.push_str("groupshared uint32_t besl_mesh_output_count;");
			string.push_str(ShaderFormatting::new(self.minified).break_str());
		}
		if matches!(self.stage, Stages::Mesh { .. }) {
			self.emit_mesh_output_structs(&mut string);
		}

		// Each constructed user struct gets its factory right after its declaration. Constructor calls are emitted after
		// the struct they construct, so the factories are inserted once emission has recorded every call.
		let mut struct_ends = Vec::new();
		for node in order {
			self.emit_node_string(&mut string, &node);
			if matches!(node.borrow().node(), besl::Nodes::Struct { .. }) {
				struct_ends.push((string.len(), node));
			}
		}
		for (end, node) in struct_ends.into_iter().rev() {
			if let besl::Nodes::Struct { name, fields, .. } = node.borrow().node()
				&& self.user_struct_constructors.contains(&node)
			{
				let mut factory = String::new();
				self.emit_hlsl_struct_factory(&mut factory, name, fields);
				string.insert_str(end, &factory);
			}
		}

		Ok(string)
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
			string.push_str(formatting.break_str());
		}
		self.emit_function_attributes(string, node, "main");
		Self::emit_type_name(string, return_type.borrow().get_name().unwrap());
		string.push(' ');
		// `main` is reserved, so the entry point is written as `besl_main` like any other escaped name.
		Self::identifier("main").push_to(string);
		string.push('(');
		self.emit_call_arguments(string, params);
		self.emit_function_extra_parameters(string, node, "main", !params.is_empty());
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
		let _ = write!(string, "{} besl_construct_{name}(", Self::identifier(name));
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
			let _ = write!(string, " besl_argument_{field_name}");
			if let Some(count) = count {
				let _ = write!(string, "[{count}]");
			}
		}
		formatting.push_block_start(string);

		formatting.push_indentation(string, 1);
		let _ = write!(string, "{} besl_value", Self::identifier(name));
		formatting.push_statement_end(string);
		for field_node in fields {
			let field = field_node.borrow();
			let besl::Nodes::Member {
				name: field_name, count, ..
			} = field.node()
			else {
				continue;
			};
			let member = Self::identifier(field_name);
			let mut window = String::new();
			if self.emit_windowed_factory_field(&mut window, field_node, fields) {
				if !window.is_empty() {
					formatting.push_indentation(string, 1);
					string.push_str(&window);
					formatting.push_statement_end(string);
				}
				continue;
			}
			formatting.push_indentation(string, 1);
			if let Some(count) = count {
				let _ = write!(
					string,
					"[unroll] for(uint besl_index=0;besl_index<{count};++besl_index){{besl_value.{member}[besl_index]=besl_argument_{field_name}[besl_index];}}{}",
					formatting.break_str()
				);
			} else {
				let _ = write!(string, "besl_value.{member}=besl_argument_{field_name}");
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
			"mat2f" => "float2x2",
			"mat3f" => "float3x3",
			"mat4f" => "float4x4",
			"mat4x3f" => "float4x3",
			"f16" => "float16_t",
			"f32" => "float",
			"u8" => "uint",
			"u16" => "uint16_t",
			"u32" => "uint32_t",
			"atomicu32" => "uint32_t",
			"i32" => "int32_t",
			"atomici32" => "int32_t",
			"Texture2D" => "Texture2D",
			"Texture3D" => "Texture3D",
			"TextureCube" => "TextureCube<float4>",
			"TextureCubeArray" => "TextureCubeArray<float4>",
			"ArrayTexture2D" => "Texture2DArray<float4>",
			_ => source,
		}
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	// Keep the exhaustive node-to-HLSL mapping together so adding a BESL node requires handling its backend contract here.
	#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
	pub(crate) fn emit_node_string(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		if let Some(temporary) = self.atomic_temporaries.get(this_node) {
			string.push_str(temporary);
			return;
		}

		let node = RefCell::borrow(this_node);
		let formatting = ShaderFormatting::new(self.minified);

		let break_char = formatting.break_str();

		match node.node() {
			// A `use` and a signature only link names, and a layer's members emit where code reads them.
			besl::Nodes::Scope { .. } | besl::Nodes::Use { .. } | besl::Nodes::Layer { .. } | besl::Nodes::Signature { .. } => {
			}
			besl::Nodes::Function {
				name,
				statements,
				return_type,
				params,
				..
			} => {
				// The shared emitter escapes the reserved `main` to the `besl_main` entry point.
				if name == "main" && matches!(self.stage, Stages::Task { .. }) {
					self.emit_hlsl_task_entry(string, this_node, statements, return_type, params);
				} else {
					self.emit_function_node(string, this_node, name, statements, return_type, params);
				}
			}
			besl::Nodes::Struct {
				name, fields, template, ..
			} => {
				let fields = self.windowed_members(fields);
				self.emit_struct_node(string, name, &fields, template)
			}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right })
				if *operator == besl::Operators::Assignment && self.emit_image_size_assignment(string, left, right) => {}
			besl::Nodes::PushConstant { members } => {
				// Push constants are the root constants at b0; the set bases follow at b1 and descriptors take no register.
				if !self.minified {
					string.push_str("// Root constants\n");
				}
				self.emit_named_struct_start(string, "PushConstant");
				let members = self.windowed_members(members);
				emit_statement_block(string, formatting, &members, 1, |string, member| {
					self.emit_node_string(string, member)
				});
				self.emit_struct_declaration_end(string);
				let _ = write!(
					string,
					"ConstantBuffer<PushConstant> push_constant : register(b0, space0);{break_char}"
				);
			}
			// DXC treats Vulkan specialization attributes as resource metadata, so use plain HLSL constants.
			besl::Nodes::Specialization { name, r#type, id } => self.emit_specialization_node(string, name, r#type, *id),
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
			besl::Nodes::Parameter { name, r#type } => {
				self.emit_variable_declaration(string, name, r#type.borrow().get_name().unwrap())
			}
			besl::Nodes::Input {
				name, location, format, ..
			} => {
				if matches!(self.stage, Stages::Vertex | Stages::Fragment) {
					return;
				}
				let format = format.borrow();
				let besl_type = format.get_name().unwrap();

				// HLSL uses semantics like TEXCOORD0, TEXCOORD1, etc.
				let _ = write!(
					string,
					"{}{} {} : TEXCOORD{location};{break_char}",
					if self.stage.interpolates_inputs() && is_integer_besl_type(besl_type) {
						"nointerpolation "
					} else {
						""
					},
					Self::translate_type(besl_type),
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
				if count.is_some() || matches!(self.stage, Stages::Vertex | Stages::Fragment) {
					return;
				}
				let format = format.borrow();
				let besl_type = format.get_name().unwrap();

				// HLSL uses SV_Target0, SV_Target1, etc. for render targets
				let _ = write!(
					string,
					"{}{} {} : SV_Target{location};{break_char}",
					if self.stage.interpolates_outputs() && is_integer_besl_type(besl_type) {
						"nointerpolation "
					} else {
						""
					},
					Self::translate_type(besl_type),
					Self::identifier(name)
				);
			}
			besl::Nodes::TaskPayload { .. } => {
				if self.task_payloads.first() == Some(this_node) {
					self.emit_object_payload_struct(string);
				}
			}
			besl::Nodes::Workgroup { name, format, count } => {
				let _ = write!(
					string,
					"groupshared {} {}",
					Self::type_identifier(format.borrow().get_name().unwrap()),
					Self::identifier(name)
				);
				if let Some(count) = count {
					let _ = write!(string, "[{count}]");
				}
				let _ = write!(string, ";{break_char}");
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
				read,
				write,
				r#type,
				count,
				placement,
				..
			} => {
				// Descriptors live in the shader-visible heaps, at their set's base plus their placement in the set.
				// `generate` rejects reachable bindings without a placement before it emits any node.
				let placement = placement.expect("HLSL generation emits only placed bindings");
				let count = count.map(|count| count.get());
				let read_only = *read && !*write;
				let buffer_type = if read_only { "StructuredBuffer" } else { "RWStructuredBuffer" };
				let resource = DescriptorIndex::Set {
					set: placement.set,
					sampler: false,
					offset: placement.resource,
				};

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						self.emit_named_struct_start(string, format_args!("_{name}"));
						let members = self.windowed_members(members);
						emit_statement_block(string, formatting, &members, 1, |string, member| {
							self.emit_node_string(string, member)
						});
						self.emit_struct_declaration_end(string);

						emit_descriptor_declaration(
							string,
							break_char,
							format_args!("{buffer_type}<_{name}>"),
							Self::identifier(name),
							resource,
							count,
						);
					}
					besl::BindingTypes::BufferArray { element, .. } => {
						let element = element.borrow();
						let element_type = element.get_name().unwrap();
						// Narrow elements share 32-bit words so their lane writes can use InterlockedCompareExchange.
						let narrow = super::hlsl_narrow_element(element_type).is_some();
						let element_type = Self::type_identifier(element_type);
						let element_type: &dyn std::fmt::Display = if narrow { &"uint" } else { &element_type };
						emit_descriptor_declaration(
							string,
							break_char,
							format_args!("{buffer_type}<{element_type}>"),
							Self::identifier(name),
							resource,
							count,
						);
					}
					besl::BindingTypes::Image { format } => {
						// A UAV (unordered access view) for images the shader writes, and an SRV (shader resource view) for
						// images it only reads, which binds as a sampled image. Both read with `image[coord]`.
						let texture_type = if read_only { "Texture2D" } else { "RWTexture2D" };
						let element_type = match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "uint",
							"r16" => "unorm float4",
							_ => "float4",
						};

						emit_descriptor_declaration(
							string,
							break_char,
							format_args!("{texture_type}<{element_type}>"),
							Self::identifier(name),
							resource,
							count,
						);
					}
					besl::BindingTypes::CombinedImageSampler { format, sampler } => {
						// HLSL separates textures and samplers, so a combined sampler becomes a texture and a sampler.
						let texture_type = match format.as_str() {
							"Texture3D" => "Texture3D",
							"TextureCube" => "TextureCube",
							"TextureCubeArray" => "TextureCubeArray",
							"ArrayTexture2D" => "Texture2DArray",
							_ => "Texture2D",
						};
						let element_type = match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "<uint>",
							_ => "<float4>",
						};
						// References build the sampler name from the escaped texture name, so both use one identifier.
						let name = Self::identifier(name);

						emit_descriptor_declaration(
							string,
							break_char,
							format_args!("{texture_type}{element_type}"),
							name,
							resource,
							count,
						);
						// A fixed sampler sits at the sampler heap index its state decides; the application binds the
						// others into the set's sampler range.
						let sampler = match sampler {
							Some(state) => DescriptorIndex::FixedSampler(state.fixed_index()),
							None => DescriptorIndex::Set {
								set: placement.set,
								sampler: true,
								offset: placement.sampler,
							},
						};
						emit_descriptor_declaration(
							string,
							break_char,
							"SamplerState",
							format_args!("{name}_sampler"),
							sampler,
							count,
						);
					}
					besl::BindingTypes::AccelerationStructure => {
						unreachable!("HLSL generation rejects ray tracing programs before it emits their bindings")
					}
				}
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node_string(string, element);
				}
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
		requirements: &IntrinsicRequirements,
	) {
		// Generated HLSL uses the engine's modern-only DXIL contract.
		hlsl_block.push_str("// Shader Model 6.9\n");

		// Shader type as comment (user preference: Option B), followed by the stage's feature requirements.
		let (stage, requirement) = match compilation_settings.stage {
			Stages::Vertex => ("vertex", ""),
			Stages::Fragment => ("fragment", ""),
			Stages::Compute { .. } => (
				"compute",
				"// Requires: Wave intrinsics (WaveGetLaneCount, WaveGetLaneIndex, etc.)\n",
			),
			Stages::Task { .. } => ("amplification", "// Requires: Amplification shader support\n"),
			Stages::Mesh { .. } => ("mesh", "// Requires: Mesh shader support\n"),
			Stages::RayGeneration | Stages::ClosestHit => {
				unreachable!("`validate_no_ray_tracing` rejects ray tracing stages before HLSL emits a header.")
			}
			Stages::Tile { .. } => {
				unreachable!(
					"A tile shader reaches HLSL already rewritten into a compute shader by `backends::tile::lower_for`."
				)
			}
		};
		let _ = write!(
			hlsl_block,
			"// #pragma shader_stage({stage})\n// Requires: native 16-bit, wave, and 64-bit integer shader operations\n{requirement}"
		);

		// Matrix layout, then constants.
		hlsl_block.push_str("#pragma pack_matrix(row_major)\nstatic const float PI = 3.14159265359;");

		hlsl_block.push_str(ShaderFormatting::new(self.minified).break_str());
		if requirements.uses_subgroup_intrinsics {
			hlsl_block.push_str(
				"bool _besl_subgroup_ballot_any(uint4 mask) { return any(mask); }\n\
				 uint _besl_subgroup_ballot_find_lsb(uint4 mask) { if (mask.x != 0u) { return firstbitlow(mask.x); } if (mask.y != 0u) { return 32u + firstbitlow(mask.y); } if (mask.z != 0u) { return 64u + firstbitlow(mask.z); } if (mask.w != 0u) { return 96u + firstbitlow(mask.w); } return 0xffffffffu; }\n\
				 uint _besl_subgroup_ballot_count(uint4 mask) { return countbits(mask.x) + countbits(mask.y) + countbits(mask.z) + countbits(mask.w); }\n\
				 uint4 _besl_subgroup_ballot_and_not(uint4 mask, uint4 removed) { return mask & ~removed; }\n\
				 float _besl_subgroup_shuffle_xor_f32(float value, uint mask) { return WaveReadLaneAt(value, WaveGetLaneIndex() ^ mask); }\n",
			);
		}
		if requirements.uses_fma {
			// These helpers preserve BESL's one-rounding FMA contract.
			hlsl_block.push_str(
				"// A binary16 product is exact in binary32. TwoSum recovers the addition residual so a binary32 midpoint can be rounded on the exact side.\n\
float16_t _besl_fma_f16(float16_t first, float16_t second, float16_t third) { precise float product = float(first) * float(second); precise float addend = float(third); precise float sum = product + addend; if (!isfinite(sum)) { return float16_t(sum); } precise float virtual_addend = sum - product; precise float residual = (product - (sum - virtual_addend)) + (addend - virtual_addend); uint rounded_bits = f32tof16(sum) & 0xffffu; float rounded = f16tof32(rounded_bits); if (residual == 0.0 || rounded == sum) { return float16_t(rounded); } bool rounded_below = rounded < sum; bool negative = (rounded_bits & 0x8000u) != 0u; uint adjacent_bits = rounded_bits + (rounded_below != negative ? 1u : 0xffffffffu); float adjacent = f16tof32(adjacent_bits); float midpoint = !isfinite(rounded) || !isfinite(adjacent) ? (sum < 0.0 ? -65520.0 : 65520.0) : rounded + (adjacent - rounded) * 0.5; if (sum != midpoint) { return float16_t(rounded); } bool rounded_follows_residual = (residual > 0.0 && rounded > sum) || (residual < 0.0 && rounded < sum); return float16_t(rounded_follows_residual ? rounded : adjacent); }\n\
float16_t2 _besl_fma_f16(float16_t2 first, float16_t2 second, float16_t2 third) { return float16_t2(_besl_fma_f16(first.x, second.x, third.x), _besl_fma_f16(first.y, second.y, third.y)); }\n\
float16_t3 _besl_fma_f16(float16_t3 first, float16_t3 second, float16_t3 third) { return float16_t3(_besl_fma_f16(first.x, second.x, third.x), _besl_fma_f16(first.y, second.y, third.y), _besl_fma_f16(first.z, second.z, third.z)); }\n\
float16_t4 _besl_fma_f16(float16_t4 first, float16_t4 second, float16_t4 third) { return float16_t4(_besl_fma_f16(first.x, second.x, third.x), _besl_fma_f16(first.y, second.y, third.y), _besl_fma_f16(first.z, second.z, third.z), _besl_fma_f16(first.w, second.w, third.w)); }\n",
			);
		}
	}
}

/// The `DescriptorIndex` enum names where one descriptor of a binding sits in a shader-visible heap, so a declaration
/// can read it with `ResourceDescriptorHeap` or `SamplerDescriptorHeap`.
#[derive(Clone, Copy)]
pub(crate) enum DescriptorIndex {
	/// `offset` descriptors past the base of the set at position `set`: the set's resource base, or its sampler base
	/// when `sampler` is set.
	Set { set: u32, sampler: bool, offset: u32 },
	/// A fixed sampler, at the sampler heap index its state decides. See [`besl::SamplerState::fixed_index`].
	FixedSampler(u32),
}

impl std::fmt::Display for DescriptorIndex {
	/// Writes the heap index expression. A set's two bases are root constants: set `s` keeps its resource base in
	/// `_besl_sets[s >> 1][(s & 1) * 2]` and its sampler base in the component after it.
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match *self {
			Self::Set { set, sampler, offset } => {
				let component = ["x", "y", "z", "w"][((set & 1) * 2 + u32::from(sampler)) as usize];
				write!(formatter, "_besl_sets[{}].{component}", set >> 1)?;
				if offset != 0 {
					write!(formatter, "+{offset}u")?;
				}
				Ok(())
			}
			Self::FixedSampler(index) => write!(formatter, "{index}u"),
		}
	}
}

/// Declares the root constants that hold the base heap indices of each set the shader reads, sized to the last of
/// them, or nothing when the shader reads no descriptor.
///
/// Returns an error when a reachable binding has no placement, which linking gives every binding of a set `main`
/// takes, or when the shader reads more sets than the root constants hold.
fn emit_set_bases(string: &mut String, order: &[besl::NodeReference], break_char: &str) -> Result<(), ()> {
	let mut set_count = 0u32;
	for node in order {
		if let besl::Nodes::Binding { placement, .. } = node.borrow().node() {
			set_count = set_count.max(placement.ok_or(())?.set + 1);
		}
	}
	if set_count == 0 {
		return Ok(());
	}
	if set_count > besl::MAX_DESCRIPTOR_SETS {
		return Err(());
	}
	let _ = write!(
		string,
		"cbuffer _BeslSets : register(b1, space0) {{ uint4 _besl_sets[{}]; }};{break_char}",
		set_count.div_ceil(2)
	);
	Ok(())
}

/// Declares the binding `name` of `count` descriptors of `type_name` read from the heap at `index`.
///
/// A single descriptor is a static global. An array is a static global of a struct whose `operator[]` reads element
/// `i` at `index + i`, so references index it as they would a native array. Lanes may pick different elements, so
/// the element's heap index is always marked non-uniform.
fn emit_descriptor_declaration(
	string: &mut String,
	break_char: &str,
	type_name: impl std::fmt::Display,
	name: impl std::fmt::Display,
	index: DescriptorIndex,
	count: Option<u32>,
) {
	let heap = match index {
		DescriptorIndex::Set { sampler: false, .. } => "ResourceDescriptorHeap",
		DescriptorIndex::Set { sampler: true, .. } | DescriptorIndex::FixedSampler(_) => "SamplerDescriptorHeap",
	};
	if count.is_none() {
		let _ = write!(string, "static {type_name} {name} = {heap}[{index}];{break_char}");
		return;
	}
	let _ = write!(
		string,
		"struct _besl_array_{name} {{ {type_name} operator[](uint i) {{ return "
	);
	match index {
		// Every element of a fixed sampler array is the same sampler.
		DescriptorIndex::FixedSampler(_) => {
			let _ = write!(string, "{heap}[{index}]");
		}
		DescriptorIndex::Set { .. } => {
			let _ = write!(string, "{heap}[NonUniformResourceIndex({index}+i)]");
		}
	}
	let _ = write!(string, "; }} }};{break_char}static _besl_array_{name} {name};{break_char}");
}
