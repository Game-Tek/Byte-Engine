use super::*;
impl<'p> Generator<'p> {
	pub(crate) fn hlsl_buffer_binding_source(&self, source: &besl::NodeId) -> Option<HlslBufferBindingSource> {
		let program = self.program;
		match program[source].node() {
			besl::Nodes::Binding {
				name,
				r#type: besl::BindingTypes::Buffer { .. },
				write,
				..
			} => Some(HlslBufferBindingSource {
				name: program.text(*name).to_string(),
				write: *write,
				wrapped_element: None,
			}),
			besl::Nodes::Binding {
				name,
				r#type: besl::BindingTypes::BufferArray { element, .. },
				write,
				..
			} => Some(HlslBufferBindingSource {
				name: program.text(*name).to_string(),
				write: *write,
				// DX12 stores narrow elements in shared 32-bit words and wraps `mat4x3f` elements in a struct; every other
				// element indexes directly.
				wrapped_element: program.name(*element).and_then(super::hlsl_wrapped_element),
			}),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => self.hlsl_buffer_binding_source(source),
			_ => None,
		}
	}

	/// Recovers a buffer member name and its source from either BESL member representation.
	pub(crate) fn hlsl_buffer_member_reference(&self, member: &besl::NodeId) -> Option<(String, besl::NodeId)> {
		let program = self.program;
		let member = &program[member];
		match member.node() {
			besl::Nodes::Expression(besl::Expressions::Member { name, source }) => {
				Some((program.text(*name).to_string(), *source))
			}
			besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
				Some((self.hlsl_member_name(right)?, *left))
			}
			_ => None,
		}
	}

	/// Recovers the underlying HLSL buffer, the accessed field, and narrow-element metadata for an indexed BESL
	/// member expression. An array buffer's field is its own binding name.
	pub(crate) fn hlsl_buffer_member_target(
		&self,
		member: &besl::NodeId,
	) -> Option<(String, String, bool, Option<&'static str>)> {
		// Lexed buffer-member access can retain its dot operation as an accessor,
		// so recover both sides before indexing it.
		let (name, source) = self.hlsl_buffer_member_reference(member)?;
		let binding = self.hlsl_buffer_binding_source(&source)?;
		Some((binding.name, name, binding.write, binding.wrapped_element))
	}

	/// Reports whether an accessor selects one element from a declared buffer-member array.
	pub(crate) fn hlsl_buffer_member_is_array(&self, member: &besl::NodeId) -> bool {
		let Some((name, source)) = self.hlsl_buffer_member_reference(member) else {
			return false;
		};
		self.hlsl_buffer_source_member_is_array(&source, &name)
	}

	/// Finds whether the named member is an array in the underlying buffer declaration.
	pub(crate) fn hlsl_buffer_source_member_is_array(&self, source: &besl::NodeId, member_name: &str) -> bool {
		let program = self.program;
		if runtime_buffer_element(program, source).is_some() {
			return true;
		}
		match program[source].node() {
			besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} => members.iter().any(|member| {
				matches!(
					program[member].node(),
					besl::Nodes::Member {
						name,
						count: Some(_),
						..
					} if program.text(*name) == member_name
				)
			}),
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => {
				self.hlsl_buffer_source_member_is_array(source, member_name)
			}
			_ => false,
		}
	}

	pub(crate) fn hlsl_member_name(&self, member: &besl::NodeId) -> Option<String> {
		let program = self.program;
		let member = &program[member];
		let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = member.node() else {
			return None;
		};
		Some(program.text(*name).to_string())
	}

	/// Returns the BESL name of the value type `node` produces, or `None` when linking cannot know it.
	///
	/// The HLSL emitters use it to pick type-dependent spellings, such as `mul()` for matrix products. It delegates to
	/// [`besl::infer_expression_type`], so every backend types expressions the same way.
	pub(crate) fn node_type_name(&self, node: &besl::NodeId) -> Option<String> {
		let program = self.program;
		besl::infer_expression_type(program, node).and_then(|r#type| program.name(r#type).map(str::to_string))
	}

	pub(crate) fn hlsl_square_matrix_column_type(type_name: &str) -> Option<(&'static str, usize)> {
		match type_name {
			"mat2f" => Some(("vec2f", 2)),
			"mat3f" => Some(("vec3f", 3)),
			"mat4f" => Some(("vec4f", 4)),
			_ => None,
		}
	}

	pub(crate) fn is_square_column_vector_matrix_constructor(&self, type_name: &str, parameters: &[besl::NodeId]) -> bool {
		let Some((column_type, column_count)) = Self::hlsl_square_matrix_column_type(type_name) else {
			return false;
		};

		parameters.len() == column_count
			&& parameters
				.iter()
				.all(|parameter| self.node_type_name(parameter).as_deref() == Some(column_type))
	}

	/// Splits a `let` with an initializer into its declaration and its initializer, or returns `None` for any other
	/// expression.
	pub(crate) fn declaration_with_initializer<'e>(
		&self,
		expression: &'e besl::Expressions,
	) -> Option<(&'e besl::NodeId, &'e besl::NodeId)> {
		let program = self.program;
		let besl::Expressions::Operator {
			operator: besl::Operators::Assignment,
			left,
			right,
		} = expression
		else {
			return None;
		};
		matches!(
			program[left].node(),
			besl::Nodes::Expression(besl::Expressions::VariableDeclaration { .. })
		)
		.then_some((left, right))
	}

	/// Emits an array built inside an expression, such as a function argument, as a call to a function that returns
	/// the array wrapped in a struct, then reads the array out of it.
	///
	/// HLSL can't build an array inside an expression, nor return one, but a function can return a struct that holds
	/// one. Each array type gets one such struct and function, in [`Generator::helpers`].
	pub(crate) fn emit_hlsl_array_constructor(&mut self, string: &mut String, node: &besl::NodeId, elements: &[besl::NodeId]) {
		let program = self.program;
		let type_name = {
			let node = &program[node];
			let besl::Nodes::Expression(besl::Expressions::FunctionCall { function, .. }) = node.node() else {
				unreachable!("An array constructor is a function call");
			};
			program.name(*function).unwrap_or_default().to_string()
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
		arguments: &[besl::NodeId],
		return_type: &str,
		previous_value: &str,
	) {
		let operation = match name {
			"atomic_load" => "InterlockedOr",
			"atomic_exchange" => "InterlockedExchange",
			"atomic_compare_exchange" => "InterlockedCompareExchange",
			"atomic_add" | "atomic_sub" => "InterlockedAdd",
			"atomic_min" => "InterlockedMin",
			"atomic_max" | "image_atomic_max" => "InterlockedMax",
			"atomic_and" => "InterlockedAnd",
			"atomic_or" | "image_atomic_or" => "InterlockedOr",
			"atomic_xor" => "InterlockedXor",
			_ => unreachable!("Only value-producing atomic intrinsics are lifted"),
		};
		string.push_str(operation);
		string.push('(');
		if is_image_atomic(name) {
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
		arguments: &[besl::NodeId],
		return_type: Option<&str>,
	) {
		let program = self.program;
		let id = self.atomic_helper_counter;
		self.atomic_helper_counter = self.atomic_helper_counter.checked_add(1).expect(
			"HLSL atomic helper count overflowed. The most likely cause is an invalid shader with billions of atomic calls.",
		);
		// The destination's indices, then every operand, each with the BESL type its parameter takes.
		let mut parameters = Vec::<(besl::NodeId, String)>::new();
		let destination = if is_image_atomic(name) { None } else { Some(&arguments[0]) };
		if let Some(destination) = destination {
			collect_destination_indices(program, destination, &mut parameters);
		}
		let operands = &arguments[1..];
		for (index, operand) in operands.iter().enumerate() {
			// An image atomic's first operand is its coordinate; every other operand has the atomic's value type.
			let r#type = if destination.is_none() && index == 0 {
				self.node_type_name(operand).unwrap_or_else(|| "vec2u".to_string())
			} else {
				return_type
					.map(str::to_string)
					.or_else(|| self.node_type_name(operand))
					.unwrap_or_else(|| "u32".to_string())
			};
			parameters.push((*operand, r#type));
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
			self.substitutions.insert(*node, format!("_besl_p{index}"));
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

	pub(crate) fn emit_array_initializer(&mut self, string: &mut String, value: &besl::NodeId) -> bool {
		let program = self.program;
		let value = &program[value];
		let besl::Nodes::Expression(besl::Expressions::FunctionCall { parameters, .. }) = value.node() else {
			return false;
		};

		// HLSL array constants use brace initializers rather than constructor syntax like float[3](...).
		string.push('{');
		self.emit_call_arguments(string, parameters);
		string.push('}');
		true
	}

	pub(crate) fn emit_const_node(&mut self, string: &mut String, name: &str, r#type: &besl::NodeId, value: &besl::NodeId) {
		let program = self.program;
		let type_node = &program[r#type];
		let type_name = program.text(type_node.get_name().unwrap());
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
fn collect_destination_indices(program: &besl::Program, node: &besl::NodeId, parameters: &mut Vec<(besl::NodeId, String)>) {
	match program[node].node() {
		besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
			collect_destination_indices(program, left, parameters);
			let indexed = program[left].node().is_indexable(program);
			let constant = matches!(
				program[right].node(),
				besl::Nodes::Expression(besl::Expressions::Literal { .. })
			);
			if indexed && !constant {
				parameters.push((*right, "u32".to_string()));
			}
		}
		besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => {
			collect_destination_indices(program, source, parameters)
		}
		besl::Nodes::Expression(besl::Expressions::Expression { elements }) if elements.len() == 1 => {
			collect_destination_indices(program, &elements[0], parameters)
		}
		_ => {}
	}
}

/// Returns whether `name` is an image atomic, whose destination is a storage-image texel instead of a buffer or
/// workgroup value.
fn is_image_atomic(name: &str) -> bool {
	matches!(name, "image_atomic_or" | "image_atomic_max")
}
