use super::*;
impl Generator {
	pub(crate) fn hlsl_flattened_array_member(members: &[besl::NodeReference]) -> Option<(String, String)> {
		let [member] = members else {
			return None;
		};
		let member = member.borrow();
		let besl::Nodes::Member {
			name,
			r#type,
			count: Some(_),
		} = member.node()
		else {
			return None;
		};
		let element_type = r#type.borrow().get_name()?.to_string();
		Some((name.to_string(), element_type))
	}

	pub(crate) fn hlsl_buffer_binding_source(source: &besl::NodeReference) -> Option<HlslBufferBindingSource> {
		match source.borrow().node() {
			besl::Nodes::Binding {
				name,
				r#type: besl::BindingTypes::Buffer { members },
				write,
				..
			} => {
				let (flattened_member, flattened_element_type) = Self::hlsl_flattened_array_member(members)
					.map_or((None, None), |(name, element_type)| (Some(name), Some(element_type)));
				Some(HlslBufferBindingSource {
					name: name.to_string(),
					write: *write,
					flattened_member,
					flattened_element_type,
				})
			}
			besl::Nodes::Binding {
				name,
				r#type: besl::BindingTypes::BufferArray { .. },
				write,
				..
			} => Some(HlslBufferBindingSource {
				name: name.to_string(),
				write: *write,
				flattened_member: None,
				flattened_element_type: None,
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

	/// Recovers the underlying HLSL buffer and flattened-field metadata for an indexed BESL member expression.
	pub(crate) fn hlsl_buffer_member_target(
		member: &besl::NodeReference,
	) -> Option<(String, String, bool, Option<String>, bool)> {
		// Lexed buffer-member access can retain its dot operation as an accessor,
		// so recover both sides before indexing it.
		let (name, source) = Self::hlsl_buffer_member_reference(member)?;
		let binding = Self::hlsl_buffer_binding_source(&source)?;
		let flattened = binding.flattened_member.as_deref() == Some(name.as_str());
		Some((binding.name, name, binding.write, binding.flattened_element_type, flattened))
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

	pub(crate) fn hlsl_buffer_member_type(source: &besl::NodeReference, member_name: &str) -> Option<String> {
		match source.borrow().node() {
			besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} => members.iter().find_map(|member| {
				let member = member.borrow();
				let besl::Nodes::Member { name, r#type, .. } = member.node() else {
					return None;
				};
				(name == member_name)
					.then(|| r#type.borrow().get_name().map(str::to_string))
					.flatten()
			}),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => {
				Self::hlsl_buffer_member_type(source, member_name)
			}
			_ => None,
		}
	}

	pub(crate) fn hlsl_member_name(member: &besl::NodeReference) -> Option<String> {
		let member = member.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = member.node() else {
			return None;
		};
		Some(name.to_string())
	}

	pub(crate) fn node_type_name(node: &besl::NodeReference) -> Option<String> {
		match node.borrow().node() {
			besl::Nodes::Parameter { r#type, .. }
			| besl::Nodes::Member { r#type, .. }
			| besl::Nodes::Input { format: r#type, .. }
			| besl::Nodes::Output { format: r#type, .. }
			| besl::Nodes::TaskPayload { format: r#type, .. }
			| besl::Nodes::Workgroup { format: r#type, .. }
			| besl::Nodes::Specialization { r#type, .. }
			| besl::Nodes::Const { r#type, .. }
			| besl::Nodes::Expression(besl::Expressions::VariableDeclaration { r#type, .. }) => {
				r#type.borrow().get_name().map(str::to_string)
			}
			besl::Nodes::Struct { name, .. } => Some(name.to_string()),
			besl::Nodes::Function { return_type, .. }
			| besl::Nodes::Intrinsic {
				r#return: return_type, ..
			} => return_type.borrow().get_name().map(str::to_string),
			besl::Nodes::Expression(besl::Expressions::FunctionCall { function, .. }) => Self::node_type_name(function),
			besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) => Self::node_type_name(intrinsic),
			besl::Nodes::Expression(besl::Expressions::Literal { value }) => Some(
				if matches!(value.as_str(), "true" | "false") {
					"bool"
				} else if value.contains(['.', 'e', 'E']) {
					"f32"
				} else {
					"u32"
				}
				.to_string(),
			),
			besl::Nodes::Expression(besl::Expressions::Member { name, source }) => {
				Self::referenced_member_type_name(name, source)
			}
			besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
				if matches!(
					right.borrow().node(),
					besl::Nodes::Expression(besl::Expressions::Member { .. })
				) {
					Self::node_type_name(right)
				} else {
					Self::accessor_type_name(left)
				}
			}
			besl::Nodes::Expression(besl::Expressions::Operator { operator, left, right }) => {
				Self::operator_result_type_name(operator, left, right)
			}
			besl::Nodes::Expression(besl::Expressions::Expression { elements }) if elements.len() == 1 => {
				Self::node_type_name(&elements[0])
			}
			_ => None,
		}
	}

	pub(crate) fn referenced_member_type_name(name: &str, source: &besl::NodeReference) -> Option<String> {
		match source.borrow().node() {
			besl::Nodes::Parameter { .. }
			| besl::Nodes::Member { .. }
			| besl::Nodes::Input { .. }
			| besl::Nodes::Output { .. }
			| besl::Nodes::TaskPayload { .. }
			| besl::Nodes::Workgroup { .. }
			| besl::Nodes::Specialization { .. }
			| besl::Nodes::Const { .. }
			| besl::Nodes::Expression(besl::Expressions::VariableDeclaration { .. }) => {
				return Self::node_type_name(source);
			}
			besl::Nodes::Function { params, .. } => {
				return params
					.iter()
					.find(|parameter| parameter.borrow().get_name() == Some(name))
					.and_then(Self::node_type_name);
			}
			_ => {}
		}

		for child in source.borrow().get_children()? {
			if child.borrow().get_name() == Some(name) {
				return Self::node_type_name(&child);
			}
			if let Some(type_name) = Self::referenced_member_type_name(name, &child) {
				return Some(type_name);
			}
		}
		None
	}

	pub(crate) fn accessor_type_name(left: &besl::NodeReference) -> Option<String> {
		if let Some(element) = runtime_buffer_element(left) {
			return element.borrow().get_name().map(str::to_string);
		}
		if let Some((name, source)) = Self::hlsl_buffer_member_reference(left)
			&& let Some(binding) = Self::hlsl_buffer_binding_source(&source)
		{
			let flattened = binding.flattened_member.as_deref() == Some(name.as_str());
			let member_type = if flattened {
				binding.flattened_element_type
			} else {
				Self::hlsl_buffer_member_type(&source, &name)
			}?;
			// The first index on an array member selects its declared element.
			// Only a later index into that element selects a matrix column or vector component.
			return if Self::hlsl_buffer_member_is_array(left) {
				Some(member_type)
			} else {
				Some(Self::indexed_value_type_name(&member_type).to_string())
			};
		}

		// Local and parameter references do not carry buffer metadata. Their
		// resolved value type still determines the result of one index operation.
		Self::node_type_name(left).map(|type_name| Self::indexed_value_type_name(&type_name).to_string())
	}

	/// Returns the BESL value type produced by indexing one matrix, vector, or scalar-like value.
	pub(crate) fn indexed_value_type_name(type_name: &str) -> &str {
		if let Some((element_type, _)) = Self::hlsl_array_type(type_name) {
			return element_type;
		}
		match type_name {
			"mat2f" => "vec2f",
			"mat3f" => "vec3f",
			"mat4f" => "vec4f",
			"mat4x3f" => "vec3f",
			"vec2u16" | "vec4u16" => "u16",
			"vec2i" => "i32",
			"vec2u" | "vec3u" | "vec4u" => "u32",
			"vec2f16" | "vec3f16" | "vec4f16" => "f16",
			"vec2f" | "vec3f" | "vec4f" | "packed_vec4f" => "f32",
			_ => type_name,
		}
	}

	/// Recovers matrix result types without mistaking matrix-vector products for matrices.
	pub(crate) fn operator_result_type_name(
		operator: &besl::Operators,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) -> Option<String> {
		let left_type = Self::node_type_name(left);
		let right_type = Self::node_type_name(right);
		match operator {
			besl::Operators::Plus
			| besl::Operators::Minus
			| besl::Operators::Multiply
			| besl::Operators::Divide
			| besl::Operators::Modulo => Self::matrix_arithmetic_result_type(left_type.as_deref(), right_type.as_deref()),
			_ => None,
		}
	}

	/// Mirrors the matrix and f32-broadcast result rules used by the BESL VM.
	pub(crate) fn matrix_arithmetic_result_type(left_type: Option<&str>, right_type: Option<&str>) -> Option<String> {
		match (left_type, right_type) {
			(Some(left), Some(right)) if left == right && Self::is_matrix_type(Some(left)) => Some(left.to_string()),
			(Some(matrix), Some("f32")) if Self::is_matrix_type(Some(matrix)) => Some(matrix.to_string()),
			(Some("f32"), Some(matrix)) if Self::is_matrix_type(Some(matrix)) => Some(matrix.to_string()),
			_ => None,
		}
	}

	pub(crate) fn is_matrix_type(type_name: Option<&str>) -> bool {
		type_name.is_some_and(|name| matches!(name, "mat2f" | "mat3f" | "mat4f" | "mat4x3f"))
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

	pub(crate) fn hlsl_name_likely_matrix_operand(name: &str) -> bool {
		name.contains("projection")
			|| name.contains("matrix")
			|| name == "model"
			|| name.ends_with(".model")
			|| name == "view"
			|| name.ends_with(".view")
	}

	pub(crate) fn emit_texture_2d_array_grad_sample(
		&mut self,
		string: &mut String,
		texture_array: &besl::NodeReference,
		texture_index: &besl::NodeReference,
		uv: &besl::NodeReference,
		uv_derivative_x: &besl::NodeReference,
		uv_derivative_y: &besl::NodeReference,
	) {
		self.emit_node_string(string, texture_array);
		string.push('[');
		self.emit_node_string(string, texture_index);
		string.push_str("].SampleGrad(");
		self.emit_node_string(string, texture_array);
		string.push_str("_sampler,");
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv);
		string.push(',');
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv_derivative_x);
		string.push(',');
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv_derivative_y);
		string.push(')');
	}

	pub(crate) fn hlsl_array_type(source: &str) -> Option<(&str, &str)> {
		let (element_type, count) = source.split_once('[')?;
		Some((element_type, count.trim_end_matches(']')))
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
	fn contains_hlsl_value_atomic(node: &besl::NodeReference) -> bool {
		if Self::hlsl_atomic_call(node).is_some() {
			return true;
		}
		let children = {
			let node = node.borrow();
			match node.node() {
				besl::Nodes::Function { statements, .. } => statements.clone(),
				besl::Nodes::Conditional { condition, statements } => {
					let mut children = vec![condition.clone()];
					children.extend(statements.iter().cloned());
					children
				}
				besl::Nodes::ForLoop {
					initializer,
					condition,
					update,
					statements,
				} => {
					let mut children = vec![initializer.clone(), condition.clone(), update.clone()];
					children.extend(statements.iter().cloned());
					children
				}
				besl::Nodes::Expression(expression) => match expression {
					besl::Expressions::Return { value } => value.iter().cloned().collect(),
					besl::Expressions::Expression { elements } => elements.clone(),
					besl::Expressions::FunctionCall { parameters, .. } => parameters.clone(),
					besl::Expressions::IntrinsicCall { arguments, .. } => arguments.clone(),
					besl::Expressions::Operator { left, right, .. } | besl::Expressions::Accessor { left, right } => {
						vec![left.clone(), right.clone()]
					}
					besl::Expressions::Macro { body, .. } => vec![body.clone()],
					besl::Expressions::Continue
					| besl::Expressions::Discard
					| besl::Expressions::Member { .. }
					| besl::Expressions::VariableDeclaration { .. }
					| besl::Expressions::Literal { .. } => Vec::new(),
				},
				_ => Vec::new(),
			}
		};
		children.iter().any(Self::contains_hlsl_value_atomic)
	}

	/// Rejects contexts where statement lifting would change when an atomic executes.
	pub(crate) fn has_unsupported_hlsl_atomic_context(node: &besl::NodeReference) -> bool {
		let node = node.borrow();
		match node.node() {
			besl::Nodes::Function { statements, .. } => statements.iter().any(Self::has_unsupported_hlsl_atomic_context),
			besl::Nodes::Conditional { condition, statements } => {
				Self::has_unsupported_hlsl_atomic_context(condition)
					|| statements.iter().any(Self::has_unsupported_hlsl_atomic_context)
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				// atomic_store lowers to a statement block, which is invalid in every
				// for-loop header field and cannot be moved without changing timing.
				Self::uses_intrinsic(initializer, "atomic_store")
					|| Self::uses_intrinsic(condition, "atomic_store")
					|| Self::uses_intrinsic(update, "atomic_store")
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
				| besl::Expressions::Discard
				| besl::Expressions::Member { .. }
				| besl::Expressions::VariableDeclaration { .. }
				| besl::Expressions::Literal { .. } => false,
			},
			_ => false,
		}
	}

	/// Returns the expression children that must be evaluated before `node`.
	fn hlsl_expression_children(node: &besl::NodeReference) -> Vec<besl::NodeReference> {
		let node = node.borrow();
		match node.node() {
			besl::Nodes::Conditional { condition, .. } => vec![condition.clone()],
			// A for-loop initializer runs once, so it can be lifted before the loop.
			// Atomics in the repeated condition or update are rejected by validation.
			besl::Nodes::ForLoop { initializer, .. } => vec![initializer.clone()],
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::Return { value } => value.iter().cloned().collect(),
				besl::Expressions::Expression { elements } => elements.clone(),
				besl::Expressions::FunctionCall { parameters, .. } => parameters.clone(),
				besl::Expressions::IntrinsicCall { arguments, .. } => arguments.clone(),
				besl::Expressions::Operator { left, right, .. } | besl::Expressions::Accessor { left, right } => {
					vec![left.clone(), right.clone()]
				}
				besl::Expressions::Macro { body, .. } => vec![body.clone()],
				besl::Expressions::Continue
				| besl::Expressions::Discard
				| besl::Expressions::Member { .. }
				| besl::Expressions::VariableDeclaration { .. }
				| besl::Expressions::Literal { .. } => Vec::new(),
			},
			_ => Vec::new(),
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
		for child in Self::hlsl_expression_children(node) {
			self.emit_hlsl_atomic_temporaries(string, &child, indent);
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
		Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
		string.push(' ');
		string.push_str(name);
		string.push(';');
		let array_texture = Self::node_type_name(&arguments[0]).as_deref() == Some("ArrayTexture2D");
		if array_texture {
			string.push_str("uint ");
			string.push_str(name);
			string.push_str("_layers;");
		}
		self.emit_node_string(string, &arguments[0]);
		string.push_str(".GetDimensions(");
		string.push_str(name);
		string.push_str(".x, ");
		string.push_str(name);
		string.push_str(".y");
		if array_texture {
			string.push_str(", ");
			string.push_str(name);
			string.push_str("_layers");
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
		emit_comma_separated_nodes(
			string,
			ShaderFormatting::new(self.minified),
			parameters,
			|string, parameter| self.emit_node_string(string, parameter),
		);
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
		if let Some(vector_type) = crate::shader::generator::scalar_array_vector_type(type_name) {
			string.push_str(Self::translate_type(vector_type));
			string.push(' ');
			string.push_str(name);
			string.push_str(" = ");
			self.emit_node_string(string, value);
		} else if let Some((element_type, count)) = Self::hlsl_array_type(type_name) {
			string.push_str(Self::translate_type(element_type));
			string.push(' ');
			string.push_str(name);
			string.push('[');
			string.push_str(count);
			string.push_str("] = ");
			if !self.emit_array_initializer(string, value) {
				self.emit_node_string(string, value);
			}
		} else {
			Self::emit_type_name(string, type_name);
			string.push(' ');
			string.push_str(name);
			string.push_str(" = ");
			self.emit_node_string(string, value);
		}
		string.push(';');
		if !self.minified {
			string.push('\n');
		}
	}
}
