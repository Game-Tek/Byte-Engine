use super::*;
impl Generator {
	pub(crate) fn hlsl_buffer_binding_source(source: &besl::NodeReference) -> Option<HlslBufferBindingSource> {
		match source.borrow().node() {
			besl::Nodes::Binding {
				name,
				r#type: besl::BindingTypes::Buffer { .. },
				write,
				..
			} => Some(HlslBufferBindingSource {
				name: name.to_string(),
				write: *write,
				narrow_element: None,
			}),
			besl::Nodes::Binding {
				name,
				r#type: besl::BindingTypes::BufferArray { element, .. },
				write,
				..
			} => Some(HlslBufferBindingSource {
				name: name.to_string(),
				write: *write,
				// DX12 stores narrow elements in shared 32-bit words; every other element indexes directly.
				narrow_element: element.borrow().get_name().and_then(super::hlsl_narrow_element),
			}),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => Self::hlsl_buffer_binding_source(source),
			_ => None,
		}
	}

	/// Recovers a buffer member name and its source from either BESL member representation.
	pub(crate) fn hlsl_buffer_member_reference(member: &besl::NodeReference) -> Option<(String, besl::NodeReference)> {
		let member = member.borrow();
		match member.node() {
			besl::Nodes::Expression(besl::Expressions::Member { name, source }) => Some((name.to_string(), source.clone())),
			besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
				Some((Self::hlsl_member_name(right)?, left.clone()))
			}
			_ => None,
		}
	}

	/// Recovers the underlying HLSL buffer, the accessed field, and narrow-element metadata for an indexed BESL
	/// member expression. An array buffer's field is its own binding name.
	pub(crate) fn hlsl_buffer_member_target(
		member: &besl::NodeReference,
	) -> Option<(String, String, bool, Option<&'static str>)> {
		// Lexed buffer-member access can retain its dot operation as an accessor,
		// so recover both sides before indexing it.
		let (name, source) = Self::hlsl_buffer_member_reference(member)?;
		let binding = Self::hlsl_buffer_binding_source(&source)?;
		Some((binding.name, name, binding.write, binding.narrow_element))
	}

	/// Reports whether an accessor selects one element from a declared buffer-member array.
	pub(crate) fn hlsl_buffer_member_is_array(member: &besl::NodeReference) -> bool {
		let Some((name, source)) = Self::hlsl_buffer_member_reference(member) else {
			return false;
		};
		Self::hlsl_buffer_source_member_is_array(&source, &name)
	}

	/// Finds whether the named member is an array in the underlying buffer declaration.
	pub(crate) fn hlsl_buffer_source_member_is_array(source: &besl::NodeReference, member_name: &str) -> bool {
		if runtime_buffer_element(source).is_some() {
			return true;
		}
		match source.borrow().node() {
			besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} => members.iter().any(|member| {
				matches!(
					member.borrow().node(),
					besl::Nodes::Member {
						name,
						count: Some(_),
						..
					} if name == member_name
				)
			}),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => {
				Self::hlsl_buffer_source_member_is_array(source, member_name)
			}
			_ => false,
		}
	}

	pub(crate) fn hlsl_member_name(member: &besl::NodeReference) -> Option<String> {
		let member = member.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = member.node() else {
			return None;
		};
		Some(name.to_string())
	}

	/// Returns the BESL name of the value type `node` produces, or `None` when linking cannot know it.
	///
	/// The HLSL emitters use it to pick type-dependent spellings, such as `mul()` for matrix products. It delegates to
	/// [`besl::infer_expression_type`], so every backend types expressions the same way.
	pub(crate) fn node_type_name(node: &besl::NodeReference) -> Option<String> {
		besl::infer_expression_type(node).and_then(|r#type| r#type.borrow().get_name().map(str::to_string))
	}

	pub(crate) fn hlsl_square_matrix_column_type(type_name: &str) -> Option<(&'static str, usize)> {
		match type_name {
			"mat2f" => Some(("vec2f", 2)),
			"mat3f" => Some(("vec3f", 3)),
			"mat4f" => Some(("vec4f", 4)),
			_ => None,
		}
	}

	pub(crate) fn is_square_column_vector_matrix_constructor(type_name: &str, parameters: &[besl::NodeReference]) -> bool {
		let Some((column_type, column_count)) = Self::hlsl_square_matrix_column_type(type_name) else {
			return false;
		};

		parameters.len() == column_count
			&& parameters
				.iter()
				.all(|parameter| Self::node_type_name(parameter).as_deref() == Some(column_type))
	}

	pub(crate) fn image_size_arguments(expression: &besl::NodeReference) -> Option<Vec<besl::NodeReference>> {
		let expression = expression.borrow();
		let besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) = expression.node()
		else {
			return None;
		};
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic { name, .. } = intrinsic.node() else {
			return None;
		};
		matches!(name.as_str(), "image_size" | "texture_size").then(|| arguments.clone())
	}

	/// Returns the value-producing atomic call represented by `node`.
	pub(crate) fn hlsl_atomic_call(node: &besl::NodeReference) -> Option<(String, Vec<besl::NodeReference>, String)> {
		let node = node.borrow();
		let besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) = node.node()
		else {
			return None;
		};
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic { name, r#return, .. } = intrinsic.node() else {
			return None;
		};
		if !matches!(
			name.as_str(),
			"image_atomic_or"
				| "atomic_load"
				| "atomic_exchange"
				| "atomic_compare_exchange"
				| "atomic_add"
				| "atomic_sub"
				| "atomic_min"
				| "atomic_max"
				| "atomic_and"
				| "atomic_or"
				| "atomic_xor"
		) {
			return None;
		}
		Some((name.clone(), arguments.clone(), r#return.borrow().get_name()?.to_string()))
	}

	/// Reports whether an expression tree contains an atomic call that returns a value.
	pub(crate) fn contains_hlsl_value_atomic(node: &besl::NodeReference) -> bool {
		any_code_node(node, false, &mut |node| Self::hlsl_atomic_call(node).is_some())
	}

	/// Splits a `let` with an initializer into its declaration and its initializer, or returns `None` for any other
	/// expression.
	pub(crate) fn declaration_with_initializer(
		expression: &besl::Expressions,
	) -> Option<(&besl::NodeReference, &besl::NodeReference)> {
		let besl::Expressions::Operator {
			operator: besl::Operators::Assignment,
			left,
			right,
		} = expression
		else {
			return None;
		};
		matches!(
			left.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::VariableDeclaration { .. })
		)
		.then_some((left, right))
	}

	/// Reports whether an array constructor appears anywhere except as the initializer of a `let` or a constant.
	///
	/// HLSL has no array expressions, only brace initializers in declarations, so other uses can't be lowered.
	pub(crate) fn has_misplaced_array_constructor(node: &besl::NodeReference) -> bool {
		let borrowed = node.borrow();
		let initializer = match borrowed.node() {
			besl::Nodes::Expression(expression) => {
				Self::declaration_with_initializer(expression).map(|(_, initializer)| initializer)
			}
			besl::Nodes::Const { value, .. } => Some(value),
			_ => None,
		};
		if let Some(elements) = initializer.and_then(crate::shader::generator::array_constructor_elements) {
			return elements.iter().any(Self::has_misplaced_array_constructor);
		}
		crate::shader::generator::array_constructor_elements(node).is_some()
			|| borrowed.node().children().any(Self::has_misplaced_array_constructor)
	}

	/// Rejects contexts where statement lifting would change when an atomic executes.
	pub(crate) fn has_unsupported_hlsl_atomic_context(node: &besl::NodeReference) -> bool {
		let node = node.borrow();
		match node.node() {
			besl::Nodes::Function { statements, .. } => statements.iter().any(Self::has_unsupported_hlsl_atomic_context),
			branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => {
				branch.branch_children().any(Self::has_unsupported_hlsl_atomic_context)
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				// atomic_store lowers to a statement block, which is invalid in every
				// for-loop header field and cannot be moved without changing timing.
				uses_intrinsic(initializer, "atomic_store")
					|| uses_intrinsic(condition, "atomic_store")
					|| uses_intrinsic(update, "atomic_store")
					|| Self::contains_hlsl_value_atomic(condition)
					|| Self::contains_hlsl_value_atomic(update)
					|| Self::has_unsupported_hlsl_atomic_context(initializer)
					|| Self::has_unsupported_hlsl_atomic_context(condition)
					|| Self::has_unsupported_hlsl_atomic_context(update)
					|| statements.iter().any(Self::has_unsupported_hlsl_atomic_context)
			}
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::Operator { operator, left, right } => {
					(matches!(operator, besl::Operators::LogicalAnd | besl::Operators::LogicalOr)
						&& (Self::contains_hlsl_value_atomic(left) || Self::contains_hlsl_value_atomic(right)))
						|| Self::has_unsupported_hlsl_atomic_context(left)
						|| Self::has_unsupported_hlsl_atomic_context(right)
				}
				besl::Expressions::Unary { operand, .. } => Self::has_unsupported_hlsl_atomic_context(operand),
				// Only the selected branch runs, so lifting an atomic out of a branch would run it unconditionally, as
				// for `&&` and `||`. The condition always runs, so its atomics lift.
				besl::Expressions::Ternary {
					condition,
					if_true,
					if_false,
				} => {
					Self::contains_hlsl_value_atomic(if_true)
						|| Self::contains_hlsl_value_atomic(if_false)
						|| Self::has_unsupported_hlsl_atomic_context(condition)
				}
				besl::Expressions::Return { value } => value.as_ref().is_some_and(Self::has_unsupported_hlsl_atomic_context),
				besl::Expressions::Expression { elements } => elements.iter().any(Self::has_unsupported_hlsl_atomic_context),
				besl::Expressions::FunctionCall { parameters, .. } => {
					parameters.iter().any(Self::has_unsupported_hlsl_atomic_context)
				}
				besl::Expressions::IntrinsicCall { arguments, .. } => {
					arguments.iter().any(Self::has_unsupported_hlsl_atomic_context)
				}
				besl::Expressions::Accessor { left, right } => {
					Self::has_unsupported_hlsl_atomic_context(left) || Self::has_unsupported_hlsl_atomic_context(right)
				}
				besl::Expressions::Macro { body, .. } => Self::has_unsupported_hlsl_atomic_context(body),
				besl::Expressions::Continue
				| besl::Expressions::Break
				| besl::Expressions::Discard
				| besl::Expressions::Member { .. }
				| besl::Expressions::VariableDeclaration { .. }
				| besl::Expressions::Literal { .. } => false,
			},
			_ => false,
		}
	}

	/// Emits one HLSL Interlocked call with the previous value written to `previous_value`.
	pub(crate) fn emit_hlsl_atomic_call(
		&mut self,
		string: &mut String,
		name: &str,
		arguments: &[besl::NodeReference],
		return_type: &str,
		previous_value: &str,
	) {
		let operation = match name {
			"atomic_load" => "InterlockedOr",
			"atomic_exchange" => "InterlockedExchange",
			"atomic_compare_exchange" => "InterlockedCompareExchange",
			"atomic_add" | "atomic_sub" => "InterlockedAdd",
			"atomic_min" => "InterlockedMin",
			"atomic_max" => "InterlockedMax",
			"atomic_and" => "InterlockedAnd",
			"atomic_or" | "image_atomic_or" => "InterlockedOr",
			"atomic_xor" => "InterlockedXor",
			_ => unreachable!("Only value-producing atomic intrinsics are lifted"),
		};
		string.push_str(operation);
		string.push('(');
		if name == "image_atomic_or" {
			self.emit_node_string(string, &arguments[0]);
			string.push('[');
			self.emit_node_string(string, &arguments[1]);
			string.push(']');
			string.push_str(ShaderFormatting::new(self.minified).comma_str());
			self.emit_node_string(string, &arguments[2]);
		} else {
			self.emit_node_string(string, &arguments[0]);
			string.push_str(ShaderFormatting::new(self.minified).comma_str());
			match name {
				"atomic_load" => string.push('0'),
				"atomic_sub" if return_type == "i32" => {
					// Negating INT_MIN as a signed value overflows. Form the additive
					// inverse modulo 2^32, then preserve its bits for InterlockedAdd.
					string.push_str("asint(0u-asuint(");
					self.emit_node_string(string, &arguments[1]);
					string.push_str("))");
				}
				"atomic_sub" => {
					string.push_str("-(");
					self.emit_node_string(string, &arguments[1]);
					string.push(')');
				}
				_ => self.emit_node_string(string, &arguments[1]),
			}
			if name == "atomic_compare_exchange" {
				string.push_str(ShaderFormatting::new(self.minified).comma_str());
				self.emit_node_string(string, &arguments[2]);
			}
		}
		string.push_str(ShaderFormatting::new(self.minified).comma_str());
		string.push_str(previous_value);
		string.push(')');
	}

	/// Lifts expression-valued atomics into HLSL statements because Interlocked intrinsics return through out parameters.
	pub(crate) fn emit_hlsl_atomic_temporaries(&mut self, string: &mut String, node: &besl::NodeReference, indent: usize) {
		// Lift the atomics of the expression children that are evaluated before `node` first.
		let mut lift = |child: &besl::NodeReference| self.emit_hlsl_atomic_temporaries(string, child, indent);
		match node.borrow().node() {
			besl::Nodes::Conditional { condition, .. }
			| besl::Nodes::Match {
				scrutinee: condition, ..
			} => lift(condition),
			// A for-loop initializer runs once, so it can be lifted before the loop.
			// Atomics in the repeated condition or update are rejected by validation.
			besl::Nodes::ForLoop { initializer, .. } => lift(initializer),
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::Return { value } => value.iter().for_each(&mut lift),
				besl::Expressions::Expression { elements: children }
				| besl::Expressions::FunctionCall {
					parameters: children, ..
				}
				| besl::Expressions::IntrinsicCall { arguments: children, .. } => children.iter().for_each(&mut lift),
				besl::Expressions::Operator { left, right, .. } | besl::Expressions::Accessor { left, right } => {
					lift(left);
					lift(right);
				}
				besl::Expressions::Unary { operand, .. } => lift(operand),
				// Validation rejects atomics in a ternary's branches, which run conditionally.
				besl::Expressions::Ternary { condition, .. } => lift(condition),
				besl::Expressions::Macro { body, .. } => lift(body),
				besl::Expressions::Continue
				| besl::Expressions::Break
				| besl::Expressions::Discard
				| besl::Expressions::Member { .. }
				| besl::Expressions::VariableDeclaration { .. }
				| besl::Expressions::Literal { .. } => {}
			},
			_ => {}
		}
		if self.atomic_temporaries.contains_key(node) {
			return;
		}
		let Some((name, arguments, return_type)) = Self::hlsl_atomic_call(node) else {
			return;
		};

		let temporary_id = self.atomic_temporary_counter;
		self.atomic_temporary_counter = self.atomic_temporary_counter.checked_add(1).expect(
			"HLSL atomic temporary count overflowed. The most likely cause is an invalid shader with billions of atomic calls.",
		);
		let temporary = format!("besl_atomic_previous_{temporary_id}");
		let formatting = ShaderFormatting::new(self.minified);
		formatting.push_indentation(string, indent);
		Self::emit_type_name(string, &return_type);
		string.push(' ');
		string.push_str(&temporary);
		formatting.push_statement_end(string);
		formatting.push_indentation(string, indent);
		self.emit_hlsl_atomic_call(string, &name, &arguments, &return_type, &temporary);
		formatting.push_statement_end(string);
		self.atomic_temporaries.insert(node.clone(), temporary);
	}

	pub(crate) fn emit_image_size_assignment(
		&mut self,
		string: &mut String,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) -> bool {
		let Some(arguments) = Self::image_size_arguments(right) else {
			return false;
		};
		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::VariableDeclaration { name, r#type }) = left.node() else {
			return false;
		};

		// HLSL exposes texture dimensions through an out-parameter method instead of an expression value.
		let name = Self::identifier(name);
		let array_texture = Self::node_type_name(&arguments[0]).as_deref() == Some("ArrayTexture2D");
		Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
		let _ = write!(string, " {name};");
		if array_texture {
			// The layer count is derived from the escaped name, so it stays unique beside it.
			let _ = write!(string, "uint {name}_layers;");
		}
		self.emit_node_string(string, &arguments[0]);
		let _ = write!(string, ".GetDimensions({name}.x, {name}.y");
		if array_texture {
			let _ = write!(string, ", {name}_layers");
		}
		string.push(')');
		true
	}

	pub(crate) fn emit_array_initializer(&mut self, string: &mut String, value: &besl::NodeReference) -> bool {
		let value = value.borrow();
		let besl::Nodes::Expression(besl::Expressions::FunctionCall { parameters, .. }) = value.node() else {
			return false;
		};

		// HLSL array constants use brace initializers rather than constructor syntax like float[3](...).
		string.push('{');
		self.emit_call_arguments(string, parameters);
		string.push('}');
		true
	}

	pub(crate) fn emit_const_node(
		&mut self,
		string: &mut String,
		name: &str,
		r#type: &besl::NodeReference,
		value: &besl::NodeReference,
	) {
		let type_node = r#type.borrow();
		let type_name = type_node.get_name().unwrap();
		string.push_str("static const ");
		Self::emit_c_declaration(string, name, type_name);
		string.push_str(" = ");
		// Short scalar arrays are vectors, so only real arrays take a brace initializer.
		if crate::shader::generator::value_array_parts(type_name).is_none() || !self.emit_array_initializer(string, value) {
			self.emit_node_string(string, value);
		}
		string.push(';');
		string.push_str(ShaderFormatting::new(self.minified).break_str());
	}
}
