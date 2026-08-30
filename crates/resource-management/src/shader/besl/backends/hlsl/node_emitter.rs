use super::*;
impl crate::shader::generator::NodeEmitter for Generator {
	fn type_from_besl(source: &str) -> &str {
		Generator::translate_type(source)
	}
	fn minified(&self) -> bool {
		self.minified
	}
	fn supports_atomic_u32(&self) -> bool {
		true
	}
	fn emit_function_attributes(&mut self, string: &mut String, _node: &besl::NodeReference, name: &str) {
		if name != "besl_main" {
			return;
		}

		if self.current_stage == HlslStage::Mesh {
			string.push_str("[outputtopology(\"triangle\")]");
			if !self.minified {
				string.push('\n');
			}
		}

		let Some(local_size) = self.current_local_size else {
			return;
		};
		// HLSL attaches compute-like stage thread-group sizes directly to their entry functions.
		string.push_str(&format!(
			"[numthreads({}, {}, {})]",
			local_size.width().max(1),
			local_size.height().max(1),
			local_size.depth().max(1)
		));
		if !self.minified {
			string.push('\n');
		}
	}
	fn emit_function_extra_parameters(
		&mut self,
		string: &mut String,
		_node: &besl::NodeReference,
		name: &str,
		has_previous_parameter: bool,
	) {
		if name != "besl_main" {
			if self.current_stage == HlslStage::Vertex {
				self.emit_vertex_builtin_helper_parameters(string, has_previous_parameter);
			}
			return;
		}
		if matches!(self.current_stage, HlslStage::Vertex | HlslStage::Fragment) {
			self.emit_raster_entry_parameters(string, has_previous_parameter);
			return;
		}
		if !matches!(self.current_stage, HlslStage::Compute | HlslStage::Task | HlslStage::Mesh) {
			return;
		}

		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("uint3 dispatch_thread_id : SV_DispatchThreadID");
		self.emit_separator(string);
		string.push_str("uint3 group_thread_id : SV_GroupThreadID");
		self.emit_separator(string);
		string.push_str("uint3 group_id : SV_GroupID");
		self.emit_separator(string);
		string.push_str("uint group_thread_index : SV_GroupIndex");

		if self.current_stage == HlslStage::Mesh {
			if !self.task_payloads.is_empty() {
				self.emit_separator(string);
				string.push_str("in payload ObjectPayload payload");
			}
			self.emit_separator(string);
			string.push_str("out vertices VertexOutput besl_vertices[");
			string.push_str(&self.current_mesh_maximum_vertices.to_string());
			string.push(']');
			self.emit_separator(string);
			string.push_str("out primitives PrimitiveOutput besl_primitives[");
			string.push_str(&self.current_mesh_maximum_primitives.to_string());
			string.push(']');
			self.emit_separator(string);
			string.push_str("out indices uint3 besl_triangles[");
			string.push_str(&self.current_mesh_maximum_primitives.to_string());
			string.push(']');
		}
	}
	fn emit_function_call_extra_arguments(
		&mut self,
		string: &mut String,
		function: &besl::NodeReference,
		has_previous_argument: bool,
	) {
		if self.current_stage != HlslStage::Vertex {
			return;
		}
		let function = function.borrow();
		if matches!(function.node(), besl::Nodes::Function { name, .. } if name != "besl_main") {
			self.emit_vertex_builtin_helper_arguments(string, has_previous_argument);
		}
	}
	fn emit_function_call(
		&mut self,
		string: &mut String,
		function: &besl::NodeReference,
		parameters: &[besl::NodeReference],
	) -> bool {
		let function_node = function.borrow();
		let besl::Nodes::Struct {
			name, template: None, ..
		} = function_node.node()
		else {
			return false;
		};
		if Self::is_square_column_vector_matrix_constructor(name, parameters) {
			// Square BESL matrix constructors take columns, while their HLSL equivalents take rows.
			string.push_str("transpose(");
			string.push_str(Self::translate_type(name));
			string.push('(');
			self.emit_call_arguments(string, parameters);
			string.push_str("))");
			return true;
		}
		if crate::shader::generator::is_builtin_struct_type(name, self.supports_atomic_u32()) {
			return false;
		}
		if !self.user_struct_constructors.contains(function) {
			self.user_struct_constructors.push(function.clone());
		}

		// Route portable BESL construction through the field-by-field factory emitted with the struct.
		string.push_str("besl_construct_");
		string.push_str(name);
		string.push('(');
		self.emit_call_arguments(string, parameters);
		string.push(')');
		true
	}
	fn emit_expression_member(&mut self, string: &mut String, name: &str, source: &besl::NodeReference) -> bool {
		match source.borrow().node() {
			besl::Nodes::TaskPayload { .. } => {
				string.push_str("payload.");
				string.push_str(name);
				return true;
			}
			besl::Nodes::Workgroup { .. } => {
				string.push_str(name);
				return true;
			}
			_ => {}
		}

		let Some(binding) = Self::hlsl_buffer_binding_source(source) else {
			return false;
		};
		if name == binding.name || binding.flattened_member.as_deref() == Some(name) {
			string.push_str(&binding.name);
			return true;
		}

		// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
		string.push_str(&binding.name);
		string.push_str("[0].");
		string.push_str(name);
		true
	}
	fn emit_expression_override(&mut self, string: &mut String, expression: &besl::Expressions) -> bool {
		if let besl::Expressions::Operator { operator, left, right } = expression {
			if *operator == besl::Operators::Assignment {
				let indexed_target = {
					let left = left.borrow();
					let besl::Nodes::Expression(besl::Expressions::Accessor {
						left: member,
						right: index,
					}) = left.node()
					else {
						return false;
					};
					Some((member.clone(), index.clone()))
				};
				if let Some((member, index)) = indexed_target {
					if let Some((binding_name, _, write, element_type, flattened)) = Self::hlsl_buffer_member_target(&member) {
						if write && flattened && matches!(element_type.as_deref(), Some("u8" | "u16")) {
							let (elements_per_word, bits_per_element, element_mask) = if element_type.as_deref() == Some("u8") {
								(4u32, 8u32, "0xffu")
							} else {
								(2u32, 16u32, "0xffffu")
							};
							let temporary_id = self.packed_write_counter;
							self.packed_write_counter = self.packed_write_counter.checked_add(1).expect(
								"Packed narrow-buffer write count overflowed. The most likely cause is an invalid shader with billions of assignment nodes.",
							);
							let index_name = format!("besl_packed_index_{temporary_id}");
							let value_name = format!("besl_packed_value_{temporary_id}");

							// Adjacent logical narrow elements share one DX12 word. Clear
							// and set only this lane so concurrent writes preserve neighbors.
							// Evaluate both source expressions before changing the shared word.
							string.push_str("uint ");
							string.push_str(&index_name);
							string.push('=');
							self.emit_node_string(string, &index);
							string.push_str(";uint ");
							string.push_str(&value_name);
							string.push_str("=(uint(");
							self.emit_node_string(string, right);
							string.push_str(")&");
							string.push_str(element_mask);
							string.push_str(");InterlockedAnd(");
							self.emit_packed_word_access_by_name(string, &binding_name, &index_name, elements_per_word);
							string.push_str(",~(");
							string.push_str(element_mask);
							string.push_str("<<((");
							string.push_str(&index_name);
							let _ = write!(string, "%{elements_per_word}u)*{bits_per_element}u)));InterlockedOr(");
							self.emit_packed_word_access_by_name(string, &binding_name, &index_name, elements_per_word);
							string.push(',');
							string.push_str(&value_name);
							string.push_str("<<((");
							string.push_str(&index_name);
							let _ = write!(string, "%{elements_per_word}u)*{bits_per_element}u))");
							return true;
						}
					}
				}
			}

			let left_type = Self::node_type_name(left);
			let right_type = Self::node_type_name(right);
			if *operator == besl::Operators::Multiply
				&& left_type.as_deref() == Some("mat4x3f")
				&& right_type.as_deref() == Some("vec4f")
			{
				// HLSL float4x3 stores the four BESL columns as rows, so the vector must be the left mul operand.
				string.push_str("mul(");
				self.emit_node_string(string, right);
				string.push_str(", ");
				self.emit_node_string(string, left);
				string.push(')');
				return true;
			}
			if *operator == besl::Operators::Multiply
				&& matches!(
					(left_type.as_deref(), right_type.as_deref()),
					(Some("mat4f"), Some("mat4f" | "vec4f"))
						| (Some("mat2f" | "mat3f" | "mat4f" | "mat4x3f"), Some("f32"))
						| (Some("f32"), Some("mat2f" | "mat3f" | "mat4f" | "mat4x3f"))
				) {
				// BESL reserves algebraic multiplication for these matrix
				// shapes. Same-shaped mat4x3 values use component-wise `*`.
				string.push_str("mul(");
				self.emit_node_string(string, left);
				string.push_str(", ");
				self.emit_node_string(string, right);
				string.push(')');
				return true;
			}
			if *operator == besl::Operators::Multiply
				&& !matches!(
					(left_type.as_deref(), right_type.as_deref()),
					(Some(left), Some(right))
						if Self::is_matrix_type(Some(left)) && Self::is_matrix_type(Some(right))
				) {
				let left_name = left.borrow().get_name().map(str::to_string);
				if left_name.as_deref().is_some_and(Self::hlsl_name_likely_matrix_operand) {
					// Some expression references do not retain resolved types, so preserve known matrix operand names.
					string.push_str("mul(");
					self.emit_node_string(string, left);
					string.push_str(", ");
					self.emit_node_string(string, right);
					string.push(')');
					return true;
				}
				let mut left_operand = String::new();
				self.emit_node_string(&mut left_operand, left);
				if Self::hlsl_name_likely_matrix_operand(&left_operand) {
					// Buffer member references can lose their source type but still expose matrix field names.
					string.push_str("mul(");
					string.push_str(&left_operand);
					string.push_str(", ");
					self.emit_node_string(string, right);
					string.push(')');
					return true;
				}
			}
		}

		false
	}

	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		let right_is_member = matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		);
		if right_is_member {
			if let Some((binding_name, field_name, _, _, flattened)) = Self::hlsl_buffer_member_target(left) {
				if field_name != binding_name {
					// A component selected from a buffer field remains an HLSL swizzle after the buffer access itself is lowered.
					string.push_str(&binding_name);
					if !flattened {
						string.push_str("[0].");
						string.push_str(&field_name);
					}
					string.push('.');
					self.emit_node_string(string, right);
					return;
				}
			}
		}

		if let Some(field_name) = Self::hlsl_mesh_output_target(left) {
			// Mesh primitive attributes live in the native per-primitive output array rather than module globals.
			string.push_str("besl_primitives[");
			self.emit_node_string(string, right);
			string.push_str("].");
			string.push_str(&field_name);
			return;
		}

		if let (Some(binding), Some(field_name)) = (Self::hlsl_buffer_binding_source(left), Self::hlsl_member_name(right)) {
			if binding.flattened_member.as_deref() == Some(&field_name) {
				string.push_str(&binding.name);
			} else {
				// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
				string.push_str(&binding.name);
				string.push_str("[0].");
				string.push_str(&field_name);
			}
			return;
		}

		if !right_is_member
			&& !Self::hlsl_buffer_member_is_array(left)
			&& Self::node_type_name(left)
				.as_deref()
				.and_then(Self::hlsl_square_matrix_column_type)
				.is_some()
		{
			// BESL indexes square matrices by column. HLSL indexes them by row,
			// regardless of the storage packing selected for the shader.
			string.push_str("transpose(");
			self.emit_node_string(string, left);
			string.push_str(")[");
			self.emit_node_string(string, right);
			string.push(']');
			return;
		}

		if let Some((binding_name, field_name, _, element_type, flattened)) = Self::hlsl_buffer_member_target(left) {
			if flattened && matches!(element_type.as_deref(), Some("u8" | "u16")) {
				let (word_index, bit_offset, element_mask) = if element_type.as_deref() == Some("u8") {
					(") / 4u] >> (((", ") % 4u) * 8u)) & ", "0xffu")
				} else {
					(") / 2u] >> (((", ") % 2u) * 16u)) & ", "0xffffu")
				};

				// DX12 exposes packed narrow-index buffers as 32-bit structured words, so recover the logical element here.
				string.push_str("((");
				string.push_str(&binding_name);
				string.push_str("[(");
				self.emit_node_string(string, right);
				string.push_str(word_index);
				self.emit_node_string(string, right);
				string.push_str(bit_offset);
				string.push_str(element_mask);
				string.push(')');
				return;
			}

			if field_name == binding_name || flattened {
				string.push_str(&binding_name);
			} else {
				// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
				string.push_str(&binding_name);
				string.push_str("[0].");
				string.push_str(&field_name);
			}
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
			return;
		}

		self.emit_node_string(string, left);
		// BESL numeric access always remains an HLSL subscript, including when
		// its left side is itself an array-element expression.
		if !right_is_member {
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
		} else {
			string.push('.');
			self.emit_node_string(string, right);
		}
	}
	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeReference,
		arguments: &[besl::NodeReference],
		elements: &[besl::NodeReference],
	) {
		Generator::emit_intrinsic_call(self, string, intrinsic, arguments, elements)
	}
	fn emit_node(&mut self, string: &mut String, node: &besl::NodeReference) {
		self.emit_node_string(string, node)
	}
}
