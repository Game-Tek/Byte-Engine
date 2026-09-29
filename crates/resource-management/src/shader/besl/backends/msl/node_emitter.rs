use super::*;
impl<A: Allocator + Clone> crate::shader::generator::NodeEmitter for Generator<A> {
	fn type_from_besl(source: &str) -> &str {
		Generator::<A>::translate_type(source)
	}
	const SPECIALIZATION_QUALIFIER: &'static str = "constant";
	fn emit_specialization_constant(&self, string: &mut String, type_name: &str, name: std::fmt::Arguments<'_>, index: usize) {
		let _ = write!(string, "constant {type_name} {name} [[function_constant({index})]];");
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
	fn emit_discard(&mut self, string: &mut String) {
		string.push_str("discard_fragment()");
	}
	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeReference,
		arguments: &[besl::NodeReference],
		elements: &[besl::NodeReference],
	) {
		Generator::<A>::emit_intrinsic_call(self, string, intrinsic, arguments, elements)
	}
	fn emit_function_extra_parameters(
		&mut self,
		string: &mut String,
		node: &besl::NodeReference,
		_name: &str,
		has_previous_parameter: bool,
	) {
		self.emit_hidden_context(string, node, has_previous_parameter, true);
	}
	fn emit_function_statement_block(&mut self, string: &mut String, statements: &[besl::NodeReference], indent: usize) {
		self.emit_statement_block(string, statements, indent);
	}
	fn emit_function_call_extra_arguments(
		&mut self,
		string: &mut String,
		function: &besl::NodeReference,
		has_previous_argument: bool,
	) {
		self.emit_hidden_context(string, function, has_previous_argument, false);
	}
	fn emit_variable_declaration(&mut self, string: &mut String, name: &str, type_name: &str) {
		// Metal declares an array in C position, so the count follows the variable name rather than the
		// element type. A short scalar array stays a vector type and keeps the portable spelling.
		if crate::shader::generator::scalar_array_vector_type(type_name).is_none()
			&& let Some((element_type, count)) = crate::shader::generator::array_type_parts(type_name)
		{
			Self::type_identifier(element_type).push_to(string);
			string.push(' ');
			Self::identifier(name).push_to(string);
			string.push('[');
			string.push_str(count);
			string.push(']');
			return;
		}

		Self::emit_type_name(string, type_name);
		string.push(' ');
		Self::identifier(name).push_to(string);
	}
	fn emit_function_call(
		&mut self,
		string: &mut String,
		function: &besl::NodeReference,
		parameters: &[besl::NodeReference],
	) -> bool {
		let function_node = function.borrow();

		// An array constructor has the array type for its name. Metal has no `float4[3](...)` constructor
		// syntax, so the elements lower to a brace initializer instead.
		if let Some(type_name) = function_node.get_name()
			&& crate::shader::generator::scalar_array_vector_type(type_name).is_none()
			&& crate::shader::generator::array_type_parts(type_name).is_some()
		{
			string.push('{');
			self.emit_call_arguments(string, parameters);
			string.push('}');
			return true;
		}

		let besl::Nodes::Struct {
			name,
			fields,
			template: None,
			..
		} = function_node.node()
		else {
			return false;
		};
		if crate::shader::generator::is_builtin_struct_type(name) {
			return false;
		}

		// Metal user structs are aggregates, so their portable BESL constructors lower to brace initialization.
		Self::identifier(name).push_to(string);
		string.push('{');
		for (index, parameter) in parameters.iter().enumerate() {
			if index > 0 {
				self.emit_separator(string);
			}
			if fields.get(index).is_some_and(|field| self.is_packed_mat4x3_member(field)) {
				string.push_str("_besl_pack_mat4x3(");
				self.emit_node_string(string, parameter);
				string.push(')');
			} else {
				self.emit_node_string(string, parameter);
			}
		}
		string.push('}');
		true
	}
	fn emit_expression_override(&mut self, string: &mut String, expression: &besl::Expressions) -> bool {
		let besl::Expressions::Operator { operator, left, right } = expression else {
			return false;
		};
		if *operator != besl::Operators::Assignment || !self.expression_is_packed_mat4x3_accessor(left) {
			return false;
		}

		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::Accessor { left, right: target }) = left.node() else {
			return false;
		};
		string.push_str("_besl_store_mat4x3(");
		self.emit_accessor_expression_raw(string, left, target);
		self.emit_separator(string);
		self.emit_node_string(string, right);
		string.push(')');
		true
	}
	fn emit_expression_member(&mut self, string: &mut String, name: &str, source: &besl::NodeReference) -> bool {
		match source.borrow().node() {
			besl::Nodes::Binding { .. } => {
				if self.raster_stage_context.is_some() {
					self.emit_raster_binding_reference(string, name);
					return true;
				}
				if self.in_compute_body || self.mesh_stage_context.is_some() {
					self.emit_compute_binding_reference(string, name);
					return true;
				}
			}
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
		false
	}
	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		if self.accessor_returns_packed_mat4x3(left, right) {
			string.push_str("_besl_load_mat4x3(");
			self.emit_accessor_expression_raw(string, left, right);
			string.push(')');
		} else {
			self.emit_accessor_expression_raw(string, left, right);
		}
	}
	fn emit_node(&mut self, string: &mut String, node: &besl::NodeReference) {
		self.emit_node_string(string, node)
	}
}
