use super::*;
impl<A: Allocator + Clone> crate::shader::generator::NodeEmitter for Generator<A> {
	fn type_from_besl(source: &str) -> &str {
		Generator::<A>::translate_type(source)
	}
	fn minified(&self) -> bool {
		self.minified
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
		name: &str,
		has_previous_parameter: bool,
	) {
		if self.task_stage_context.is_some() && name != "main" {
			self.emit_task_hidden_parameters(string, has_previous_parameter);
		} else if self.in_compute_body {
			let uses_simd_lane_id = Self::uses_intrinsic(node, "subgroup_lane_index");
			if uses_simd_lane_id || self.function_requires_resource_context(node, true) {
				self.emit_compute_hidden_parameters(string, has_previous_parameter, uses_simd_lane_id);
			}
		} else if self.raster_stage_context.is_some() && name != "main" && self.function_requires_resource_context(node, false)
		{
			self.emit_raster_hidden_parameters(string, has_previous_parameter);
		}
		if self.mesh_stage_context.is_some() && name == "main" {
			self.emit_mesh_hidden_parameters(string, has_previous_parameter);
		}
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
		let function_node = RefCell::borrow(function);
		if matches!(function_node.node(), besl::Nodes::Function { name, .. } if name != "main") {
			if self.task_stage_context.is_some() {
				self.emit_task_hidden_call_arguments(string, has_previous_argument);
			} else if self.in_compute_body {
				let uses_simd_lane_id = Self::uses_intrinsic(function, "subgroup_lane_index");
				if uses_simd_lane_id || self.function_requires_resource_context(function, true) {
					self.emit_compute_hidden_call_arguments(string, has_previous_argument, uses_simd_lane_id);
				}
			} else if self.raster_stage_context.is_some() && self.function_requires_resource_context(function, false) {
				self.emit_raster_hidden_call_arguments(string, has_previous_argument);
			}
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
			name,
			fields,
			template: None,
			..
		} = function_node.node()
		else {
			return false;
		};
		if crate::shader::generator::is_builtin_struct_type(name, self.supports_atomic_u32()) {
			return false;
		}

		// Metal user structs are aggregates, so their portable BESL constructors lower to brace initialization.
		string.push_str(name);
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
				string.push_str(name);
				return true;
			}
			besl::Nodes::Workgroup { .. } => {
				string.push_str(name);
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
