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

	/// Emits an array built inside an expression, such as a function argument, as a call to a function that returns
	/// the array wrapped in a struct, then reads the array out of it.
	///
	/// HLSL can't build an array inside an expression, nor return one, but a function can return a struct that holds
	/// one. Each array type gets one such struct and function, in [`Generator::helpers`].
	pub(crate) fn emit_hlsl_array_constructor(
		&mut self,
		string: &mut String,
		node: &besl::NodeReference,
		elements: &[besl::NodeReference],
	) {
		let type_name = {
			let node = node.borrow();
			let besl::Nodes::Expression(besl::Expressions::FunctionCall { function, .. }) = node.node() else {
				unreachable!("An array constructor is a function call");
			};
			function.get().borrow().get_name().unwrap_or_default().to_string()
		};
		let (element_type, count) =
			crate::shader::generator::value_array_parts(&type_name).expect("An array constructor builds an array type");
		// Array type names hold brackets, so the helpers are named after the element type and count.
		let helper = format!(
			"_besl_array_{}_{count}",
			element_type.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
		);
		if self.constructed_arrays.insert(type_name.clone()) {
			let formatting = ShaderFormatting::new(self.minified);
			let mut function = format!("struct {helper}{{");
			Self::emit_c_declaration(&mut function, "v", &type_name);
			let _ = write!(function, ";}};{helper} {helper}_make(");
			for index in 0..elements.len() {
				if index > 0 {
					function.push_str(formatting.comma_str());
				}
				Self::emit_c_declaration(&mut function, &format!("_besl_p{index}"), element_type);
			}
			let _ = write!(function, "){{{helper} array;");
			for index in 0..elements.len() {
				let _ = write!(function, "array.v[{index}]=_besl_p{index};");
			}
			function.push_str("return array;}");
			function.push_str(formatting.break_str());
			self.helpers.push_str(&function);
		}
		let _ = write!(string, "{helper}_make(");
		self.emit_call_arguments(string, elements);
		string.push_str(").v");
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

	/// Emits a call to a generated function that runs the atomic intrinsic `name` and returns the value it read, or
	/// nothing for `atomic_store`.
	///
	/// HLSL's `Interlocked` functions return the value they read through an out parameter, so they are statements,
	/// while BESL atomics are expressions that may run conditionally or repeatedly: in `&&`, `||`, an `if` value, or a
	/// `for` condition. A function call is an expression that runs exactly when BESL evaluates the atomic, so each
	/// atomic becomes one. Its destination is rooted in a buffer or workgroup global, which the function names
	/// directly; the destination's indices and the operands are the function's parameters, evaluated at the call. The
	/// function goes in [`Generator::helpers`], which the node being emitted is preceded by.
	pub(crate) fn emit_hlsl_atomic_helper_call(
		&mut self,
		string: &mut String,
		name: &str,
		arguments: &[besl::NodeReference],
		return_type: Option<&str>,
	) {
		let id = self.atomic_helper_counter;
		self.atomic_helper_counter = self.atomic_helper_counter.checked_add(1).expect(
			"HLSL atomic helper count overflowed. The most likely cause is an invalid shader with billions of atomic calls.",
		);
		// The destination's indices, then every operand, each with the BESL type its parameter takes.
		let mut parameters = Vec::<(besl::NodeReference, String)>::new();
		let destination = if name == "image_atomic_or" {
			None
		} else {
			Some(&arguments[0])
		};
		if let Some(destination) = destination {
			collect_destination_indices(destination, &mut parameters);
		}
		let operands = &arguments[1..];
		for (index, operand) in operands.iter().enumerate() {
			// An image atomic's first operand is its coordinate; every other operand has the atomic's value type.
			let r#type = if destination.is_none() && index == 0 {
				Self::node_type_name(operand).unwrap_or_else(|| "vec2u".to_string())
			} else {
				return_type
					.map(str::to_string)
					.or_else(|| Self::node_type_name(operand))
					.unwrap_or_else(|| "u32".to_string())
			};
			parameters.push((operand.clone(), r#type));
		}

		let formatting = ShaderFormatting::new(self.minified);
		let mut helper = String::new();
		match return_type {
			Some(return_type) => Self::emit_type_name(&mut helper, return_type),
			None => helper.push_str("void"),
		}
		let _ = write!(helper, " _besl_atomic_{id}(");
		for (index, (node, r#type)) in parameters.iter().enumerate() {
			if index > 0 {
				helper.push_str(formatting.comma_str());
			}
			Self::emit_type_name(&mut helper, r#type);
			let _ = write!(helper, " _besl_p{index}");
			self.substitutions.insert(node.clone(), format!("_besl_p{index}"));
		}
		helper.push_str("){");
		match return_type {
			Some(return_type) => {
				Self::emit_type_name(&mut helper, return_type);
				helper.push_str(" _besl_previous;");
				self.emit_hlsl_atomic_call(&mut helper, name, arguments, return_type, "_besl_previous");
				helper.push_str(";return _besl_previous;}");
			}
			None => {
				// A store keeps no value, but InterlockedExchange returns the replaced one.
				let value_type = &parameters.last().expect("`atomic_store` takes a value").1;
				Self::emit_type_name(&mut helper, value_type);
				helper.push_str(" _besl_replaced;InterlockedExchange(");
				self.emit_node_string(&mut helper, &arguments[0]);
				helper.push_str(formatting.comma_str());
				self.emit_node_string(&mut helper, &arguments[1]);
				helper.push_str(formatting.comma_str());
				helper.push_str("_besl_replaced);}");
			}
		}
		helper.push_str(formatting.break_str());
		for (node, _) in &parameters {
			self.substitutions.remove(node);
		}

		// The arguments may hold atomics of their own, whose helpers then precede this one.
		let _ = write!(string, "_besl_atomic_{id}(");
		for (index, (node, _)) in parameters.iter().enumerate() {
			if index > 0 {
				string.push_str(formatting.comma_str());
			}
			self.emit_node_string(string, node);
		}
		string.push(')');
		self.helpers.push_str(&helper);
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

/// Adds each index of the atomic destination `node` to `parameters`, as a `u32`. Constant indices stay in the
/// destination.
fn collect_destination_indices(node: &besl::NodeReference, parameters: &mut Vec<(besl::NodeReference, String)>) {
	match node.borrow().node() {
		besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
			collect_destination_indices(left, parameters);
			let indexed = left.borrow().node().is_indexable();
			let constant = matches!(
				right.borrow().node(),
				besl::Nodes::Expression(besl::Expressions::Literal { .. })
			);
			if indexed && !constant {
				parameters.push((right.clone(), "u32".to_string()));
			}
		}
		besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => collect_destination_indices(source, parameters),
		besl::Nodes::Expression(besl::Expressions::Expression { elements }) if elements.len() == 1 => {
			collect_destination_indices(&elements[0], parameters)
		}
		_ => {}
	}
}
