use super::*;
impl crate::shader::generator::NodeEmitter for Generator {
	fn type_from_besl(source: &str) -> &str {
		Generator::translate_type(source)
	}
	const SPECIALIZATION_QUALIFIER: &'static str = "static const";
	fn emit_specialization_constant(&self, string: &mut String, type_name: &str, name: std::fmt::Arguments<'_>, _index: usize) {
		// HLSL has no pipeline specialization, so each constant takes the default value the other backends override.
		let _ = write!(string, "static const {type_name} {name}=1.0f;");
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
	fn emit_function_attributes(&mut self, string: &mut String, _node: &besl::NodeReference, name: &str) {
		if name != "main" {
			return;
		}

		let break_char = ShaderFormatting::new(self.minified).break_str();
		if matches!(self.stage, Stages::Mesh { .. }) {
			let _ = write!(string, "[outputtopology(\"triangle\")]{break_char}");
		}

		let Some(local_size) = self.stage.local_size() else {
			return;
		};
		// HLSL attaches compute-like stage thread-group sizes directly to their entry functions.
		let _ = write!(
			string,
			"[numthreads({}, {}, {})]{break_char}",
			local_size.width(),
			local_size.height(),
			local_size.depth()
		);
	}
	// A value atomic in the condition is lifted into a statement, which needs a block that runs only when the branch is reached.
	fn else_if_needs_block(&self, conditional: &besl::NodeReference) -> bool {
		matches!(
			conditional.borrow().node(),
			besl::Nodes::Conditional { condition, .. } if Self::contains_hlsl_value_atomic(condition)
		)
	}

	fn emit_function_statement_block(&mut self, string: &mut String, statements: &[besl::NodeReference], indent: usize) {
		let formatting = ShaderFormatting::new(self.minified);
		for statement in statements {
			self.atomic_temporaries.clear();
			self.emit_hlsl_atomic_temporaries(string, statement, indent);
			formatting.push_indentation(string, indent);
			self.emit_node_string(string, statement);
			formatting.push_statement_end(string);
		}
		self.atomic_temporaries.clear();
	}
	fn emit_function_extra_parameters(
		&mut self,
		string: &mut String,
		_node: &besl::NodeReference,
		name: &str,
		has_previous_parameter: bool,
	) {
		if name != "main" {
			if matches!(self.stage, Stages::Vertex) {
				self.emit_vertex_builtin_helper_list(string, has_previous_parameter, true);
			}
			return;
		}
		if matches!(self.stage, Stages::Vertex | Stages::Fragment) {
			self.emit_raster_entry_parameters(string, has_previous_parameter);
			return;
		}

		// Compute, task, and mesh entry points receive the dispatch builtins.
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

		if let Stages::Mesh {
			maximum_vertices,
			maximum_primitives,
			..
		} = self.stage
		{
			if !self.task_payloads.is_empty() {
				self.emit_separator(string);
				string.push_str("in payload ObjectPayload payload");
			}
			self.emit_separator(string);
			let _ = write!(string, "out vertices VertexOutput besl_vertices[{maximum_vertices}]");
			self.emit_separator(string);
			let _ = write!(string, "out primitives PrimitiveOutput besl_primitives[{maximum_primitives}]");
			self.emit_separator(string);
			let _ = write!(string, "out indices uint3 besl_triangles[{maximum_primitives}]");
		}
	}
	fn emit_function_call_extra_arguments(
		&mut self,
		string: &mut String,
		function: &besl::NodeReference,
		has_previous_argument: bool,
	) {
		if !matches!(self.stage, Stages::Vertex) {
			return;
		}
		let function = function.borrow();
		if matches!(function.node(), besl::Nodes::Function { name, .. } if name != "main") {
			self.emit_vertex_builtin_helper_list(string, has_previous_argument, false);
		}
	}
	// HLSL textures carry no sampler, so a texture parameter is paired with one named after it, as bindings are.
	fn emit_texture_parameter_sampler(&mut self, string: &mut String, name: &str) {
		self.emit_separator(string);
		let _ = write!(string, "SamplerState {}_sampler", Self::identifier(name));
	}
	fn emit_texture_argument_sampler(&mut self, string: &mut String, argument: &besl::NodeReference) {
		self.emit_separator(string);
		self.emit_sampler(string, argument);
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
		if crate::shader::generator::is_builtin_struct_type(name) {
			return false;
		}
		if !self.user_struct_constructors.contains(function) {
			self.user_struct_constructors.push(function.clone());
		}

		// Route portable BESL construction through the field-by-field factory emitted with the struct.
		// The factory name is derived from the raw BESL name, so it cannot collide with a reserved word.
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
				Self::identifier(name).push_to(string);
				return true;
			}
			besl::Nodes::Workgroup { .. } => {
				Self::identifier(name).push_to(string);
				return true;
			}
			_ => {}
		}

		let Some(binding) = Self::hlsl_buffer_binding_source(source) else {
			return false;
		};
		Self::identifier(&binding.name).push_to(string);
		if name != binding.name {
			// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
			string.push_str("[0].");
			Self::identifier(name).push_to(string);
		}
		true
	}
	fn emit_variable_declaration(&mut self, string: &mut String, name: &str, type_name: &str) {
		// HLSL declares arrays in C position, so the count follows the variable name.
		Self::emit_c_declaration(string, name, type_name);
	}
	fn emit_expression_override(&mut self, string: &mut String, expression: &besl::Expressions) -> bool {
		if let Some((declaration, initializer)) = Self::declaration_with_initializer(expression)
			&& let Some(elements) = crate::shader::generator::array_constructor_elements(initializer)
		{
			// HLSL has no array expressions, so the constructor becomes the declaration's brace initializer.
			// Validation rejects array constructors anywhere else.
			self.emit_node_string(string, declaration);
			string.push_str(if self.minified { "={" } else { " = {" });
			self.emit_call_arguments(string, &elements);
			string.push('}');
			return true;
		}
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
				if let Some((member, index)) = indexed_target
					&& let Some((binding_name, _, true, Some(element_type))) = Self::hlsl_buffer_member_target(&member)
				{
					let (elements_per_word, bits_per_element, element_mask) = if element_type == "u8" {
						(4u32, 8u32, "0xffu")
					} else {
						(2u32, 16u32, "0xffffu")
					};
					let id = self.packed_write_counter;
					self.packed_write_counter = self.packed_write_counter.checked_add(1).expect(
								"Packed narrow-buffer write count overflowed. The most likely cause is an invalid shader with billions of assignment nodes.",
							);
					let binding = Self::identifier(&binding_name);

					// Adjacent logical narrow elements share one DX12 word. Replace the
					// selected lane with one compare-exchange loop so another lane cannot
					// change between separate clear and set operations.
					let _ = write!(string, "{{uint besl_packed_index_{id}=");
					self.emit_node_string(string, &index);
					let _ = write!(string, ";uint besl_packed_value_{id}=(uint(");
					self.emit_node_string(string, right);
					let _ = write!(
						string,
						")&{element_mask});uint besl_packed_shift_{id}=(besl_packed_index_{id}%{elements_per_word}u)*{bits_per_element}u;\
						 uint besl_packed_mask_{id}={element_mask}<<besl_packed_shift_{id};uint besl_packed_expected_{id};\
						 InterlockedOr({binding}[besl_packed_index_{id}/{elements_per_word}u],0u,besl_packed_expected_{id});\
						 for(;;){{uint besl_packed_desired_{id}=(besl_packed_expected_{id}&~besl_packed_mask_{id})|(besl_packed_value_{id}<<besl_packed_shift_{id});\
						 uint besl_packed_observed_{id};\
						 InterlockedCompareExchange({binding}[besl_packed_index_{id}/{elements_per_word}u],besl_packed_expected_{id},besl_packed_desired_{id},besl_packed_observed_{id});\
						 if(besl_packed_observed_{id}==besl_packed_expected_{id}){{break;}}besl_packed_expected_{id}=besl_packed_observed_{id};}}}}"
					);
					return true;
				}
			}

			// Only multiplication depends on the operand types, so other operators skip type inference.
			if *operator != besl::Operators::Multiply {
				return false;
			}
			let left_type = Self::node_type_name(left);
			let right_type = Self::node_type_name(right);
			if left_type.as_deref() == Some("mat4x3f") && right_type.as_deref() == Some("vec4f") {
				// HLSL float4x3 stores the four BESL columns as rows, so the vector must be the left mul operand.
				string.push_str("mul(");
				self.emit_node_string(string, right);
				string.push_str(", ");
				self.emit_node_string(string, left);
				string.push(')');
				return true;
			}
			if matches!(
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
		}

		false
	}

	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		if resource_reference_kind(left) == Some(ResourceAccessorKind::DescriptorArray) {
			// Every texture intrinsic other than `sample` reaches its descriptor-array element through here.
			self.emit_node_string(string, left);
			self.emit_descriptor_array_index(string, right);
			return;
		}

		let right_is_member = matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		);
		// Resolved at most once: the swizzle path needs it only for member accessors, and the indexing path always.
		let buffer_target = right_is_member.then(|| Self::hlsl_buffer_member_target(left));
		if let Some(Some((binding_name, field_name, ..))) = &buffer_target
			&& field_name != binding_name
		{
			// A component selected from a buffer field remains an HLSL swizzle after the buffer access itself is lowered.
			Self::identifier(&binding_name).push_to(string);
			string.push_str("[0].");
			Self::identifier(&field_name).push_to(string);
			string.push('.');
			self.emit_node_string(string, right);
			return;
		}

		if let Some((field_name, per_vertex)) =
			crate::shader::generator::mesh_output_target(left, |name, per_vertex| (name.to_string(), per_vertex))
		{
			// Mesh attributes live in the native per-vertex or per-primitive output array rather than module globals.
			string.push_str(if per_vertex { "besl_vertices[" } else { "besl_primitives[" });
			self.emit_node_string(string, right);
			string.push_str("].");
			Self::identifier(&field_name).push_to(string);
			return;
		}

		if let Some(binding) = Self::hlsl_buffer_binding_source(left)
			&& let Some(field_name) = Self::hlsl_member_name(right)
		{
			// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
			Self::identifier(&binding.name).push_to(string);
			string.push_str("[0].");
			Self::identifier(&field_name).push_to(string);
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

		if let Some((binding_name, field_name, _, narrow_element)) =
			buffer_target.unwrap_or_else(|| Self::hlsl_buffer_member_target(left))
		{
			if let Some(element_type) = narrow_element {
				let (word_index, bit_offset, element_mask) = if element_type == "u8" {
					(") / 4u] >> (((", ") % 4u) * 8u)) & ", "0xffu")
				} else {
					(") / 2u] >> (((", ") % 2u) * 16u)) & ", "0xffffu")
				};

				// DX12 exposes packed narrow-index buffers as 32-bit structured words, so recover the logical element here.
				string.push_str("((");
				Self::identifier(&binding_name).push_to(string);
				string.push_str("[(");
				self.emit_node_string(string, right);
				string.push_str(word_index);
				self.emit_node_string(string, right);
				string.push_str(bit_offset);
				string.push_str(element_mask);
				string.push(')');
				return;
			}

			Self::identifier(&binding_name).push_to(string);
			if field_name != binding_name {
				// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
				string.push_str("[0].");
				Self::identifier(&field_name).push_to(string);
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
