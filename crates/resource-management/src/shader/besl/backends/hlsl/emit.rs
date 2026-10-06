use super::*;
impl Generator {
	pub(crate) fn emit_object_payload_struct(&self, string: &mut String) {
		if self.task_payloads.is_empty() {
			return;
		}

		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "ObjectPayload");
		for payload in &self.task_payloads {
			let payload = payload.borrow();
			let besl::Nodes::TaskPayload { name, format, count } = payload.node() else {
				continue;
			};

			formatting.push_indentation(string, 1);
			Self::type_identifier(format.borrow().get_name().unwrap()).push_to(string);
			let _ = write!(string, " {}[{count}]", Self::identifier(name));
			formatting.push_statement_end(string);
		}
		self.emit_struct_declaration_end(string);
	}

	/// Emits the fixed vertex output and the authored per-vertex and per-primitive mesh outputs.
	pub(crate) fn emit_mesh_output_structs(&self, string: &mut String) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "VertexOutput");
		formatting.push_indentation(string, 1);
		string.push_str("float4 position : SV_Position");
		formatting.push_statement_end(string);
		self.emit_mesh_output_fields(string, true);
		self.emit_struct_declaration_end(string);

		self.emit_named_struct_start(string, "PrimitiveOutput");
		if self.mesh_uses_render_target_array_index {
			formatting.push_indentation(string, 1);
			string.push_str("uint32_t render_target_array_index : SV_RenderTargetArrayIndex");
			formatting.push_statement_end(string);
		}
		self.emit_mesh_output_fields(string, false);
		self.emit_struct_declaration_end(string);
	}

	/// Emits the mesh output arrays of one rate as fields of their native vertex or primitive struct.
	fn emit_mesh_output_fields(&self, string: &mut String, vertex_rate: bool) {
		let formatting = ShaderFormatting::new(self.minified);
		for output in &self.mesh_outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count: Some(_),
				per_vertex,
			} = output.node()
			else {
				continue;
			};
			if *per_vertex != vertex_rate {
				continue;
			}

			formatting.push_indentation(string, 1);
			let format = format.borrow();
			let besl_type = format.get_name().unwrap();
			let type_name = Self::translate_type(besl_type);
			if is_integer_besl_type(besl_type) {
				string.push_str("nointerpolation ");
			}
			let _ = write!(string, "{} {} : TEXCOORD{location}", type_name, Self::identifier(name));
			formatting.push_statement_end(string);
		}
	}

	/// Finds a lane-guarded BESL mesh-count statement that HLSL must execute uniformly.
	pub(crate) fn mesh_output_count_arguments(
		statements: &[besl::NodeReference],
	) -> Option<(besl::NodeReference, besl::NodeReference)> {
		let [statement] = statements else {
			return None;
		};
		let statement = statement.borrow();
		let besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) = statement.node()
		else {
			return None;
		};
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic { name, .. } = intrinsic.node() else {
			return None;
		};
		let [vertices, primitives] = arguments.as_slice() else {
			return None;
		};
		(name == "set_mesh_output_counts").then(|| (vertices.clone(), primitives.clone()))
	}

	/// Emits raster stage I/O as mutable entry-point parameters because HLSL semantic globals are immutable.
	pub(crate) fn emit_raster_entry_parameters(&self, string: &mut String, mut has_previous_parameter: bool) {
		for input in &self.raster_inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, location, format } = input.node() else {
				continue;
			};
			if has_previous_parameter {
				self.emit_separator(string);
			}
			let format = format.borrow();
			let besl_type = format.get_name().unwrap();
			let type_name = Self::translate_type(besl_type);
			if matches!(self.stage, Stages::Vertex) && crate::shader::generator::is_vertex_builtin_input(name) {
				let _ = write!(string, "{} {}", type_name, Self::identifier(name));
				string.push_str(match name.as_str() {
					besl::VERTEX_INDEX_BUILTIN => " : SV_VertexID",
					besl::INSTANCE_INDEX_BUILTIN => " : SV_InstanceID",
					_ => unreachable!("Expected a validated vertex builtin"),
				});
				has_previous_parameter = true;
				continue;
			}
			if self.stage.interpolates_inputs() && is_integer_besl_type(besl_type) {
				string.push_str("nointerpolation ");
			}
			let _ = write!(string, "{} {} : TEXCOORD{location}", type_name, Self::identifier(name));
			has_previous_parameter = true;
		}

		for output in &self.raster_outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count: None,
				..
			} = output.node()
			else {
				continue;
			};
			if has_previous_parameter {
				self.emit_separator(string);
			}
			let format = format.borrow();
			let besl_type = format.get_name().unwrap();
			let type_name = Self::translate_type(besl_type);
			if self.stage.interpolates_outputs() && is_integer_besl_type(besl_type) {
				string.push_str("nointerpolation ");
			}
			string.push_str("out ");
			let _ = write!(string, "{} {}", type_name, Self::identifier(name));
			if matches!(self.stage, Stages::Vertex) && besl::is_position_output(name) {
				string.push_str(" : SV_Position");
			} else {
				let semantic = if matches!(self.stage, Stages::Fragment) {
					"SV_Target"
				} else {
					"TEXCOORD"
				};
				let _ = write!(string, " : {semantic}{location}");
			}
			has_previous_parameter = true;
		}
	}

	/// Adds the vertex invocation indices to helper signatures when the shader uses them, or forwards them through
	/// nested BESL helper calls when `with_types` is false.
	pub(crate) fn emit_vertex_builtin_helper_list(&self, string: &mut String, mut has_previous: bool, with_types: bool) {
		for input in &self.raster_inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, format, .. } = input.node() else {
				continue;
			};
			if !crate::shader::generator::is_vertex_builtin_input(name) {
				continue;
			}
			if has_previous {
				self.emit_separator(string);
			}
			if with_types {
				string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
				string.push(' ');
			}
			Self::identifier(name).push_to(string);
			has_previous = true;
		}
	}

	/// Emits the subscript that picks one texture or sampler of a descriptor array.
	pub(crate) fn emit_descriptor_array_index(&mut self, string: &mut String, index: &besl::NodeReference) {
		// Any expression may pick the element, so lanes of one wave can disagree, which D3D12 leaves undefined unless the
		// index is marked non-uniform.
		string.push_str("[NonUniformResourceIndex(");
		self.emit_node_string(string, index);
		string.push_str(")]");
	}

	/// Emits the sampler paired with a texture argument. Inside a descriptor array it shares the texture's index.
	pub(crate) fn emit_sampler(&mut self, string: &mut String, texture: &besl::NodeReference) {
		let accessor = resource_accessor(texture);
		self.emit_node_string(string, accessor.as_ref().map_or(texture, |(_, resource, _)| resource));
		string.push_str("_sampler");
		if let Some((ResourceAccessorKind::DescriptorArray, _, index)) = &accessor {
			self.emit_descriptor_array_index(string, index);
		}
	}

	// Keep the intrinsic table contiguous because each arm defines one exact HLSL lowering contract.
	#[allow(clippy::too_many_lines)]
	pub(crate) fn emit_intrinsic_call(
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
			..
		} = intrinsic.node()
		else {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		};

		let has_body = definition
			.iter()
			.any(|element| !matches!(element.borrow().node(), besl::Nodes::Parameter { .. }));
		match name.as_str() {
			// Texture samples bypass intrinsic bodies.
			"sample" => {
				let accessor = resource_accessor(&arguments[0]);
				self.emit_node_string(string, accessor.as_ref().map_or(&arguments[0], |(_, resource, _)| resource));
				if let Some((ResourceAccessorKind::DescriptorArray, _, index)) = &accessor {
					self.emit_descriptor_array_index(string, index);
				}
				string.push_str(".Sample(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				if let Some((ResourceAccessorKind::Texture2DArrayLayer, _, index)) = &accessor {
					string.push_str("float3(");
					self.emit_node_string(string, &arguments[1]);
					string.push_str(", float(");
					self.emit_node_string(string, index);
					string.push_str("))");
				} else {
					self.emit_node_string(string, &arguments[1]);
				}
				string.push(')');
			}
			"sample_texture_2d_array_grad" => {
				self.emit_node_string(string, &arguments[0]);
				self.emit_descriptor_array_index(string, &arguments[1]);
				string.push_str(".SampleGrad(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler");
				self.emit_descriptor_array_index(string, &arguments[1]);
				for argument in &arguments[2..5] {
					self.emit_separator(string);
					self.emit_node_string(string, argument);
				}
				string.push(')');
			}
			// Every other intrinsic with a body emits its expansion.
			_ if has_body => {
				for element in elements {
					self.emit_node_string(string, element);
				}
			}
			"pow" if arguments.len() == 2 && super::super::is_two(&arguments[0]) => {
				string.push_str("exp2(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"fetch" => {
				self.emit_node_string(string, &arguments[0]);
				if arguments.len() == 3 {
					string.push_str(".Load(int4(");
				} else {
					string.push_str(".Load(int3(");
				}
				self.emit_node_string(string, &arguments[1]);
				if let Some(layer) = arguments.get(2) {
					string.push_str(", int(");
					self.emit_node_string(string, layer);
					string.push(')');
				}
				string.push_str(", 0))");
			}
			"fetch_u32" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".Load(int3(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", 0)).x");
			}
			"image_load" | "image_load_u32" => {
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push(']');
			}
			"gather" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".Gather(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				if let Some(layer) = arguments.get(2) {
					string.push_str("float3(");
					self.emit_node_string(string, &arguments[1]);
					string.push_str(", float(");
					self.emit_node_string(string, layer);
					string.push_str("))");
				} else {
					self.emit_node_string(string, &arguments[1]);
				}
				string.push(')');
			}
			"texture_lod" | "downsample_min" | "downsample_max" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".SampleLevel(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				if arguments.len() == 4 {
					string.push_str("float3(");
					self.emit_node_string(string, &arguments[1]);
					string.push_str(", float(");
					self.emit_node_string(string, &arguments[2]);
					string.push_str("))");
				} else {
					self.emit_node_string(string, &arguments[1]);
				}
				string.push_str(", ");
				if let Some(lod) = arguments.get(if arguments.len() == 4 { 3 } else { 2 }) {
					self.emit_node_string(string, lod);
				} else {
					string.push_str("0.0");
				}
				string.push(')');
				if name != "texture_lod" {
					string.push_str(".x");
				}
			}
			"texture_cube_array_lod" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".SampleLevel(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", float4(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", float(");
				self.emit_node_string(string, &arguments[2]);
				string.push_str(")), ");
				self.emit_node_string(string, &arguments[3]);
				string.push(')');
			}
			"image_atomic_or" => unreachable!("HLSL image atomics must be lifted before expression emission"),
			"guard_image_bounds" => {
				// HLSL has no portable image bounds guard intrinsic, so emit the guard inline at the call site.
				string.push_str("uint2 _besl_image_size; ");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".GetDimensions(_besl_image_size.x, _besl_image_size.y); if (any(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(" >= _besl_image_size)) { return; }");
			}
			"image_size" | "texture_size" => {
				string.push_str("/* image_size requires assignment lowering for HLSL */");
				self.emit_node_string(string, &arguments[0]);
			}
			"write" => {
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push_str("] = ");
				self.emit_node_string(string, &arguments[2]);
			}
			"atomic_load"
			| "atomic_exchange"
			| "atomic_compare_exchange"
			| "atomic_add"
			| "atomic_sub"
			| "atomic_min"
			| "atomic_max"
			| "atomic_and"
			| "atomic_or"
			| "atomic_xor" => unreachable!("HLSL value atomics must be lifted before expression emission"),
			"atomic_store" => {
				let temporary_id = self.atomic_temporary_counter;
				self.atomic_temporary_counter = self.atomic_temporary_counter.checked_add(1).expect(
					"HLSL atomic temporary count overflowed. The most likely cause is an invalid shader with billions of atomic calls.",
				);
				let value_type = Self::node_type_name(&arguments[1]).unwrap_or_else(|| "u32".to_string());
				string.push('{');
				Self::emit_type_name(string, &value_type);
				let _ = write!(string, " besl_atomic_stored_{temporary_id};InterlockedExchange(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(ShaderFormatting::new(self.minified).comma_str());
				self.emit_node_string(string, &arguments[1]);
				string.push_str(ShaderFormatting::new(self.minified).comma_str());
				let _ = write!(string, "besl_atomic_stored_{temporary_id});}}");
			}
			"thread_id" => string.push_str("dispatch_thread_id.xy"),
			"thread_position" => string.push_str("dispatch_thread_id.x"),
			"thread_idx" => string.push_str("group_thread_index"),
			"subgroup_lane_index" => string.push_str("WaveGetLaneIndex()"),
			"threadgroup_position" => string.push_str("group_id.x"),
			"sincos" => {
				string.push_str("float2(sin(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("), cos(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("))");
			}
			"round_to_i32" => {
				string.push_str("int2(round(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("))");
			}
			"workgroup_barrier" => string.push_str("GroupMemoryBarrierWithGroupSync()"),
			"set_task_mesh_output_count" => {
				string.push_str("besl_mesh_output_count = ");
				self.emit_node_string(string, &arguments[0]);
			}
			"set_mesh_vertex_position" => {
				string.push_str("besl_vertices[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("].position = ");
				self.emit_node_string(string, &arguments[1]);
			}
			"set_mesh_triangle" => {
				string.push_str("besl_triangles[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("] = ");
				self.emit_node_string(string, &arguments[1]);
			}
			"set_mesh_primitive_render_target_array_index" => {
				string.push_str("besl_primitives[");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("].render_target_array_index = ");
				self.emit_node_string(string, &arguments[1]);
			}
			// Every other intrinsic is one HLSL call, renamed where HLSL spells the operation differently.
			_ => {
				let call = match name.as_str() {
					"is_nan" => "isnan",
					"is_infinite" => "isinf",
					"is_finite" => "isfinite",
					"is_normal" => "isnormal",
					// firstbitlow already returns 0xFFFFFFFF for zero.
					"find_lsb" => "firstbitlow",
					"fract" => "frac",
					"mix" => "lerp",
					"u32" => "uint",
					"f32" | "f16" | "u16" | "vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" => {
						Self::translate_type(name)
					}
					"inversesqrt" => "rsqrt",
					"subgroup_ballot" => "WaveActiveBallot",
					"subgroup_ballot_any" => "_besl_subgroup_ballot_any",
					"subgroup_ballot_find_lsb" => "_besl_subgroup_ballot_find_lsb",
					"subgroup_ballot_count" => "_besl_subgroup_ballot_count",
					"subgroup_ballot_and_not" => "_besl_subgroup_ballot_and_not",
					"subgroup_broadcast_u32" | "subgroup_broadcast_f32" => "WaveReadLaneAt",
					"subgroup_shuffle_xor_f32" => "_besl_subgroup_shuffle_xor_f32",
					"fma" if matches!(r#return.borrow().get_name(), Some("f16" | "vec2f16" | "vec3f16" | "vec4f16")) => {
						"_besl_fma_f16"
					}
					"fma" => "mad",
					"set_mesh_output_counts" => "SetMeshOutputCounts",
					"min" | "max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin"
					| "atan2" | "floor" | "round" | "fwidth" | "step" | "radians" | "smoothstep" | "dot" | "cross"
					| "normalize" | "reflect" | "length" => name,
					// Intrinsics without an HLSL call emit their elements.
					_ => {
						for element in elements {
							self.emit_node_string(string, element);
						}
						return;
					}
				};
				string.push_str(call);
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
		}
	}
}
