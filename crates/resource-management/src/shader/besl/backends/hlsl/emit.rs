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
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			string.push('[');
			string.push_str(&count.get().to_string());
			string.push(']');
			formatting.push_statement_end(string);
		}
		self.emit_struct_declaration_end(string);
	}

	/// Emits the fixed vertex output and the authored per-primitive mesh outputs.
	pub(crate) fn emit_mesh_output_structs(&self, string: &mut String) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "VertexOutput");
		formatting.push_indentation(string, 1);
		string.push_str("float4 position : SV_Position");
		formatting.push_statement_end(string);
		self.emit_struct_declaration_end(string);

		self.emit_named_struct_start(string, "PrimitiveOutput");
		if self.mesh_uses_render_target_array_index {
			formatting.push_indentation(string, 1);
			string.push_str("uint32_t render_target_array_index : SV_RenderTargetArrayIndex");
			formatting.push_statement_end(string);
		}
		for output in &self.mesh_outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count: Some(_),
			} = output.node()
			else {
				continue;
			};

			formatting.push_indentation(string, 1);
			let format = format.borrow();
			let type_name = Self::translate_type(format.get_name().unwrap());
			if Self::is_integer_type(type_name) {
				string.push_str("nointerpolation ");
			}
			string.push_str(type_name);
			string.push(' ');
			string.push_str(name);
			string.push_str(" : TEXCOORD");
			string.push_str(&location.to_string());
			formatting.push_statement_end(string);
		}
		self.emit_struct_declaration_end(string);
	}

	/// Reports whether one reachable AST branch uses the requested intrinsic.
	pub(crate) fn uses_intrinsic(node: &besl::NodeReference, intrinsic_name: &str) -> bool {
		match node.borrow().node() {
			besl::Nodes::Function { statements, .. } => statements
				.iter()
				.any(|statement| Self::uses_intrinsic(statement, intrinsic_name)),
			besl::Nodes::Conditional { condition, statements } => {
				Self::uses_intrinsic(condition, intrinsic_name)
					|| statements
						.iter()
						.any(|statement| Self::uses_intrinsic(statement, intrinsic_name))
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				Self::uses_intrinsic(initializer, intrinsic_name)
					|| Self::uses_intrinsic(condition, intrinsic_name)
					|| Self::uses_intrinsic(update, intrinsic_name)
					|| statements
						.iter()
						.any(|statement| Self::uses_intrinsic(statement, intrinsic_name))
			}
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::IntrinsicCall {
					intrinsic, arguments, ..
				} => {
					intrinsic.borrow().get_name() == Some(intrinsic_name)
						|| arguments
							.iter()
							.any(|argument| Self::uses_intrinsic(argument, intrinsic_name))
				}
				besl::Expressions::Operator { left, right, .. } => {
					Self::uses_intrinsic(left, intrinsic_name) || Self::uses_intrinsic(right, intrinsic_name)
				}
				besl::Expressions::FunctionCall { parameters, .. } => parameters
					.iter()
					.any(|parameter| Self::uses_intrinsic(parameter, intrinsic_name)),
				besl::Expressions::Expression { elements } => {
					elements.iter().any(|element| Self::uses_intrinsic(element, intrinsic_name))
				}
				besl::Expressions::Macro { body, .. } => Self::uses_intrinsic(body, intrinsic_name),
				besl::Expressions::Member { source, .. } => Self::uses_intrinsic(source, intrinsic_name),
				besl::Expressions::Return { value } => value
					.as_ref()
					.is_some_and(|value| Self::uses_intrinsic(value, intrinsic_name)),
				besl::Expressions::Accessor { left, right } => {
					Self::uses_intrinsic(left, intrinsic_name) || Self::uses_intrinsic(right, intrinsic_name)
				}
				besl::Expressions::VariableDeclaration { .. }
				| besl::Expressions::Literal { .. }
				| besl::Expressions::Continue
				| besl::Expressions::Discard => false,
			},
			_ => false,
		}
	}

	/// Reports whether reachable code uses one of BESL's compute-only subgroup operations.
	pub(crate) fn uses_subgroup_intrinsics(order: &[besl::NodeReference]) -> bool {
		const SUBGROUP_INTRINSICS: [&str; 8] = [
			"subgroup_lane_index",
			"subgroup_ballot",
			"subgroup_ballot_any",
			"subgroup_ballot_find_lsb",
			"subgroup_ballot_count",
			"subgroup_ballot_and_not",
			"subgroup_broadcast_u32",
			"subgroup_broadcast_f32",
		];
		order.iter().any(|node| {
			SUBGROUP_INTRINSICS
				.iter()
				.any(|intrinsic| Self::uses_intrinsic(node, intrinsic))
		})
	}

	/// Recovers an indexed mesh-output declaration so HLSL can address its primitive structure field.
	pub(crate) fn hlsl_mesh_output_target(left: &besl::NodeReference) -> Option<String> {
		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { source, .. }) = left.node() else {
			return None;
		};
		let source = source.borrow();
		let besl::Nodes::Output {
			name, count: Some(_), ..
		} = source.node()
		else {
			return None;
		};
		Some(name.clone())
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
	pub(crate) fn emit_raster_entry_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		let mut has_previous_parameter = has_previous_parameter;
		for input in &self.raster_inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, location, format } = input.node() else {
				continue;
			};
			if has_previous_parameter {
				self.emit_separator(string);
			}
			let format = format.borrow();
			let type_name = Self::translate_type(format.get_name().unwrap());
			if self.current_stage == HlslStage::Vertex && crate::shader::generator::is_vertex_builtin_input(name) {
				string.push_str(type_name);
				string.push(' ');
				string.push_str(name);
				string.push_str(match name.as_str() {
					besl::VERTEX_INDEX_BUILTIN => " : SV_VertexID",
					besl::INSTANCE_INDEX_BUILTIN => " : SV_InstanceID",
					_ => unreachable!("Expected a validated vertex builtin"),
				});
				has_previous_parameter = true;
				continue;
			}
			if self.current_stage_interpolates_inputs && Self::is_integer_type(type_name) {
				string.push_str("nointerpolation ");
			}
			string.push_str(type_name);
			string.push(' ');
			string.push_str(name);
			string.push_str(" : TEXCOORD");
			string.push_str(&location.to_string());
			has_previous_parameter = true;
		}

		for output in &self.raster_outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count: None,
			} = output.node()
			else {
				continue;
			};
			if has_previous_parameter {
				self.emit_separator(string);
			}
			let format = format.borrow();
			let type_name = Self::translate_type(format.get_name().unwrap());
			if self.current_stage_interpolates_outputs && Self::is_integer_type(type_name) {
				string.push_str("nointerpolation ");
			}
			string.push_str("out ");
			string.push_str(type_name);
			string.push(' ');
			string.push_str(name);
			string.push_str(if self.current_stage == HlslStage::Fragment {
				" : SV_Target"
			} else {
				" : TEXCOORD"
			});
			string.push_str(&location.to_string());
			has_previous_parameter = true;
		}
	}

	/// Adds the vertex invocation indices to helper signatures when the shader uses them.
	pub(crate) fn emit_vertex_builtin_helper_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		let mut has_previous_parameter = has_previous_parameter;
		for input in &self.raster_inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, format, .. } = input.node() else {
				continue;
			};
			if !crate::shader::generator::is_vertex_builtin_input(name) {
				continue;
			}
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			has_previous_parameter = true;
		}
	}

	/// Forwards the vertex invocation indices through nested BESL helper calls.
	pub(crate) fn emit_vertex_builtin_helper_arguments(&self, string: &mut String, has_previous_argument: bool) {
		let mut has_previous_argument = has_previous_argument;
		for input in &self.raster_inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, .. } = input.node() else {
				continue;
			};
			if !crate::shader::generator::is_vertex_builtin_input(name) {
				continue;
			}
			if has_previous_argument {
				self.emit_separator(string);
			}
			string.push_str(name);
			has_previous_argument = true;
		}
	}

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
			..
		} = intrinsic.node()
		else {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		};

		match name.as_str() {
			"sample" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".Sample(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
				return;
			}
			"sample_texture_2d_array_grad" => {
				self.emit_texture_2d_array_grad_sample(
					string,
					&arguments[0],
					&arguments[1],
					&arguments[2],
					&arguments[3],
					&arguments[4],
				);
				return;
			}
			_ => {}
		}

		let has_body = definition
			.iter()
			.any(|element| !matches!(element.borrow().node(), besl::Nodes::Parameter { .. }));
		if has_body {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		}

		match name.as_str() {
			"pow" if arguments.len() == 2 && super::super::is_two(&arguments[0]) => {
				string.push_str("exp2(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"min" | "max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin" | "atan2"
			| "floor" | "round" | "fwidth" | "step" | "radians" | "smoothstep" | "dot" | "cross" | "normalize" | "reflect"
			| "length" => {
				string.push_str(name);
				string.push('(');
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"fract" => {
				string.push_str("frac(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"mix" => {
				string.push_str("lerp(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"f32" => {
				string.push_str("float(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"f16" => {
				string.push_str("float16_t(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"u16" => {
				string.push_str("uint(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" | "packed_vec4f" => {
				string.push_str(Self::translate_type(name));
				string.push('(');
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"u32" => {
				string.push_str("uint(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
				string.push(')');
			}
			"inversesqrt" => {
				string.push_str("rsqrt(");
				emit_comma_separated_nodes(string, ShaderFormatting::new(self.minified), arguments, |string, argument| {
					self.emit_node_string(string, argument)
				});
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
			"image_load" => {
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push(']');
			}
			"texture_lod" | "downsample_min" | "downsample_max" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".SampleLevel(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
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
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, float4(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", float(");
				self.emit_node_string(string, &arguments[2]);
				string.push_str(")), ");
				self.emit_node_string(string, &arguments[3]);
				string.push(')');
			}
			"image_atomic_or" => {
				string.push_str("({ uint _previous; InterlockedOr(");
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push_str("], ");
				self.emit_node_string(string, &arguments[2]);
				string.push_str(", _previous); _previous; })");
			}
			"image_load_u32" => {
				self.emit_node_string(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push(']');
			}
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
			"atomic_add" => {
				self.emit_atomic_add_call(string, arguments, None);
			}
			"atomic_compare_exchange" => {
				// HLSL requires an out parameter even when BESL discards the previous value.
				string.push_str("{ uint _besl_atomic_previous; ");
				self.emit_atomic_compare_exchange_call(string, arguments, Some("_besl_atomic_previous"));
				string.push_str("; }");
			}
			"atomic_load" => self.emit_node_string(string, &arguments[0]),
			"atomic_store" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" = ");
				self.emit_node_string(string, &arguments[1]);
			}
			"thread_id" => {
				string.push_str("dispatch_thread_id.xy");
			}
			"thread_position" => {
				string.push_str("dispatch_thread_id.x");
			}
			"thread_idx" => {
				string.push_str("group_thread_index");
			}
			"subgroup_lane_index" => string.push_str("WaveGetLaneIndex()"),
			"threadgroup_position" => {
				string.push_str("group_id.x");
			}
			"subgroup_ballot" => {
				string.push_str("WaveActiveBallot(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_any" => {
				string.push_str("_besl_subgroup_ballot_any(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_find_lsb" => {
				string.push_str("_besl_subgroup_ballot_find_lsb(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_count" => {
				string.push_str("_besl_subgroup_ballot_count(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_and_not" => {
				string.push_str("_besl_subgroup_ballot_and_not(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_broadcast_u32" | "subgroup_broadcast_f32" => {
				string.push_str("WaveReadLaneAt(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"fma" => {
				string.push_str("mad(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
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
			"workgroup_barrier" => {
				string.push_str("GroupMemoryBarrierWithGroupSync()");
			}
			"set_task_mesh_output_count" => {
				string.push_str("besl_mesh_output_count = ");
				self.emit_node_string(string, &arguments[0]);
			}
			"set_mesh_output_counts" => {
				string.push_str("SetMeshOutputCounts(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
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
			_ => {
				for element in elements {
					self.emit_node_string(string, element);
				}
			}
		}
	}
}
