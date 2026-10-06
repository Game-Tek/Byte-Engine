use super::*;
impl Generator {
	/// Generates an MSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The shader compilation settings.
	/// * `main_function_node` - The shader's main function node.
	///
	/// # Returns
	///
	/// The MSL shader as a string.
	///
	/// # Panics
	///
	/// Panics if the main function node is not a function node.
	pub fn generate(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<String, ()> {
		let order = ordered_shader_nodes(main_function_node, "MSL");
		self.generate_order(shader_compilation_settings, main_function_node, &order)
	}

	/// Generates an MSL shader whose resource ABI contains every binding declared by `program`.
	///
	/// Code reachable from `main` is emitted, while every program binding stays in the Metal resource ABI.
	pub fn generate_program(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<String, ()> {
		let main = program.get_main().ok_or(())?;
		let mut order = ordered_shader_nodes(&main, "MSL");
		Self::append_declared_bindings(program, &mut order);
		self.generate_order(shader_compilation_settings, &main, &order)
	}

	/// Appends authored binding declarations without traversing unreachable executable nodes.
	fn append_declared_bindings(program: &besl::NodeReference, order: &mut Vec<besl::NodeReference>) {
		let program_borrow = program.borrow();
		match program_borrow.node() {
			besl::Nodes::Binding { r#type, .. } => {
				match r#type {
					besl::BindingTypes::Buffer { members } => {
						for member in members {
							Self::append_storage_type_declarations(member, order);
						}
					}
					besl::BindingTypes::BufferArray { element, .. } => {
						Self::append_storage_type_declarations(element, order);
					}
					besl::BindingTypes::Image { .. } | besl::BindingTypes::CombinedImageSampler { .. } => {}
				}
				if !order.contains(program) {
					order.push(program.clone());
				}
			}
			besl::Nodes::Scope { children, .. } => {
				for child in children {
					Self::append_declared_bindings(child, order);
				}
			}
			_ => {}
		}
	}

	/// Retains user struct declarations required to represent an authored buffer binding.
	fn append_storage_type_declarations(node: &besl::NodeReference, order: &mut Vec<besl::NodeReference>) {
		let node_borrow = node.borrow();
		match node_borrow.node() {
			besl::Nodes::Member { r#type, .. } => Self::append_storage_type_declarations(r#type, order),
			besl::Nodes::Struct { fields, .. } if !fields.is_empty() => {
				for field in fields {
					Self::append_storage_type_declarations(field, order);
				}
				if !order.contains(node) {
					order.push(node.clone());
				}
			}
			_ => {}
		}
	}

	/// Emits one shader from the reachable node order and its complete resource declarations.
	fn generate_order(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
		order: &[besl::NodeReference],
	) -> Result<String, ()> {
		crate::shader::generator::validate_workgroup_storage_stage(&shader_compilation_settings.stage, order)?;
		crate::shader::generator::validate_vertex_builtin_inputs(&shader_compilation_settings.stage, order)?;
		let intrinsic_requirements = intrinsic_requirements(order);
		if intrinsic_requirements.uses_subgroup_intrinsics
			&& !matches!(shader_compilation_settings.stage, Stages::Compute { .. })
		{
			return Err(());
		}
		Self::validate_reachable_binding_layout(order)?;
		if matches!(shader_compilation_settings.stage, Stages::Vertex | Stages::Fragment)
			&& let Some(source) = Self::find_full_source_passthrough(main_function_node)
		{
			return Ok(source);
		}
		self.collect_packed_mat4x3_members(order);
		self.hidden_contexts = analyze_hidden_contexts(order);

		let downsample_helper_capacity =
			if intrinsic_requirements.uses_downsample_min || intrinsic_requirements.uses_downsample_max {
				4096
			} else {
				0
			};
		let mut string = String::with_capacity(2048 + downsample_helper_capacity);

		self.generate_msl_header_block(&mut string, shader_compilation_settings, &intrinsic_requirements);

		match shader_compilation_settings.stage {
			Stages::Vertex if Self::has_raster_interface(order) => {
				self.generate_raster_shader(&mut string, order, main_function_node, true)
			}
			Stages::Fragment if Self::has_raster_interface(order) || Self::has_non_void_return(main_function_node) => {
				self.generate_raster_shader(&mut string, order, main_function_node, false)
			}
			Stages::Compute { .. } => self.generate_compute_shader(
				&mut string,
				order,
				main_function_node,
				intrinsic_requirements.uses_simd_lane_id,
			),
			Stages::Task {
				maximum_mesh_threadgroups,
				..
			} => self.generate_task_shader(&mut string, order, main_function_node, maximum_mesh_threadgroups),
			Stages::Mesh {
				maximum_vertices,
				maximum_primitives,
				..
			} => self.generate_mesh_shader(
				&mut string,
				order,
				main_function_node,
				maximum_vertices,
				maximum_primitives,
				intrinsic_requirements.uses_render_target_array_index,
			),
			_ => {
				for node in order {
					self.emit_node_string(&mut string, node);
				}
			}
		}

		Ok(string)
	}

	/// Finds every logical affine matrix that needs a packed Metal storage representation.
	///
	/// Members are recorded by their declaration node. A `mat4x3f` array buffer is recorded by its binding node.
	pub(crate) fn collect_packed_mat4x3_members(&mut self, order: &[besl::NodeReference]) {
		self.packed_mat4x3_members.clear();
		let mut visited_structs = Vec::new();
		for node_reference in order {
			let node = node_reference.borrow();
			let besl::Nodes::Binding { r#type, .. } = node.node() else {
				continue;
			};
			match r#type {
				besl::BindingTypes::Buffer { members } => {
					for member in members {
						self.collect_packed_mat4x3_member(member, &mut visited_structs);
					}
				}
				besl::BindingTypes::BufferArray { element, .. } => {
					let element = element.borrow();
					if element.get_name() == Some("mat4x3f") {
						self.packed_mat4x3_members.push(node_reference.clone());
					} else if let besl::Nodes::Struct { fields, .. } = element.node() {
						for field in fields {
							self.collect_packed_mat4x3_member(field, &mut visited_structs);
						}
					}
				}
				besl::BindingTypes::Image { .. } | besl::BindingTypes::CombinedImageSampler { .. } => {}
			}
		}
	}

	/// Recurses through one buffer member without changing the logical BESL type graph.
	pub(crate) fn collect_packed_mat4x3_member(
		&mut self,
		member: &besl::NodeReference,
		visited_structs: &mut Vec<besl::NodeReference>,
	) {
		let member_reference = member.clone();
		let r#type = {
			let member = member.borrow();
			let besl::Nodes::Member { r#type, .. } = member.node() else {
				return;
			};
			if r#type.borrow().get_name() == Some("mat4x3f")
				&& !self
					.packed_mat4x3_members
					.iter()
					.any(|candidate| candidate == &member_reference)
			{
				self.packed_mat4x3_members.push(member_reference);
			}
			r#type.clone()
		};

		if visited_structs.iter().any(|candidate| candidate == &r#type) {
			return;
		}
		let fields = {
			let r#type = r#type.borrow();
			let besl::Nodes::Struct { fields, .. } = r#type.node() else {
				return;
			};
			fields.clone()
		};
		visited_structs.push(r#type);
		for field in fields {
			self.collect_packed_mat4x3_member(&field, visited_structs);
		}
	}

	pub(crate) fn is_packed_mat4x3_member(&self, member: &besl::NodeReference) -> bool {
		self.packed_mat4x3_members.iter().any(|candidate| candidate == member)
	}

	/// Returns whether `expression` names packed `mat4x3f` storage and, if so, whether that storage is an array.
	pub(crate) fn packed_mat4x3_storage_is_array(
		&self,
		expression: &besl::NodeReference,
		parent: Option<&besl::NodeReference>,
	) -> Option<bool> {
		let expression = expression.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { name, source }) = expression.node() else {
			return None;
		};
		let member = if self.is_packed_mat4x3_member(source) {
			source.clone()
		} else {
			let parent_type = parent.and_then(besl::infer_expression_type)?;
			let parent_type = parent_type.borrow();
			let besl::Nodes::Struct { fields, .. } = parent_type.node() else {
				return None;
			};
			fields
				.iter()
				.find(|field| {
					self.is_packed_mat4x3_member(field)
						&& matches!(field.borrow().node(), besl::Nodes::Member { name: field_name, .. } if field_name == name)
				})?
				.clone()
		};
		let member = member.borrow();
		match member.node() {
			besl::Nodes::Member { count, .. } => Some(count.is_some()),
			besl::Nodes::Binding { .. } => Some(true),
			_ => None,
		}
	}

	/// Reports whether one accessor evaluates to a native matrix loaded from packed storage.
	pub(crate) fn accessor_returns_packed_mat4x3(&self, left: &besl::NodeReference, right: &besl::NodeReference) -> bool {
		// Without packed matrix storage no accessor can return one, so skip the type inference below.
		if self.packed_mat4x3_members.is_empty() {
			return false;
		}
		if self.packed_mat4x3_storage_is_array(right, Some(left)) == Some(false) {
			return true;
		}

		if matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		) {
			return false;
		}

		if self.packed_mat4x3_storage_is_array(left, None) == Some(true) {
			return true;
		}

		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::Accessor { right, .. }) = left.node() else {
			return false;
		};
		self.packed_mat4x3_storage_is_array(right, None) == Some(true)
	}

	pub(crate) fn expression_is_packed_mat4x3_accessor(&self, node: &besl::NodeReference) -> bool {
		let node = node.borrow();
		let besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) = node.node() else {
			return false;
		};
		self.accessor_returns_packed_mat4x3(left, right)
	}

	/// Emits a matrix product whose vector operand may be a packed storage read, converting that operand to its native
	/// vector type.
	///
	/// Metal converts packed vectors implicitly for arithmetic and built-in functions, but not for matrix
	/// multiplication. Returns `false`, without writing, when the product needs no conversion.
	pub(crate) fn emit_packed_vector_matrix_product(
		&mut self,
		string: &mut String,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) -> bool {
		let unpack_left = is_matrix_expression(right).then(|| packed_vector_read(left)).flatten();
		let unpack_right = is_matrix_expression(left).then(|| packed_vector_read(right)).flatten();
		if unpack_left.is_none() && unpack_right.is_none() {
			return false;
		}

		let formatting = ShaderFormatting::new(self.minified);
		for (index, (operand, unpack)) in [(left, unpack_left), (right, unpack_right)].into_iter().enumerate() {
			if index > 0 {
				string.push_str(formatting.space_str());
				string.push('*');
				string.push_str(formatting.space_str());
			}
			if let Some(vector) = unpack {
				string.push_str(Self::translate_type(vector.borrow().get_name().unwrap_or_default()));
				string.push('(');
				self.emit_node_string(string, operand);
				string.push(')');
			} else {
				self.emit_wrapped_expression(string, operand);
			}
		}
		true
	}

	/// Emits an accessor path without converting its final packed matrix storage value.
	pub(crate) fn emit_accessor_expression_raw(
		&mut self,
		string: &mut String,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) {
		self.emit_node_string(string, left);
		// Array buffers are element pointers; struct buffers point at their wrapper.
		if runtime_buffer_element(left).is_some() {
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
		} else if left.borrow().node().is_buffer_binding() {
			string.push_str("->");
			self.emit_node_string(string, right);
		} else if !matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		) && left.borrow().node().is_indexable()
		{
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
		} else {
			string.push('.');
			self.emit_node_string(string, right);
		}
	}

	pub(crate) fn find_full_source_passthrough(main_function_node: &besl::NodeReference) -> Option<String> {
		// Raster-stage MSL entrypoint lowering is not implemented yet, so callers can carry a full
		// Metal source through a BESL raw node while the GLSL path keeps using normal BESL generation.
		const MARKER: &str = "// besl-full-source";

		let main_function_node = main_function_node.borrow();
		let besl::Nodes::Function { statements, .. } = main_function_node.node() else {
			return None;
		};

		statements.iter().find_map(|node| {
			let node = node.borrow();
			let besl::Nodes::Raw { msl: Some(source), .. } = node.node() else {
				return None;
			};

			source.strip_prefix(MARKER).map(|source| source.trim_start().to_string())
		})
	}

	pub(crate) fn has_raster_interface(order: &[besl::NodeReference]) -> bool {
		order
			.iter()
			.any(|node| matches!(node.borrow().node(), besl::Nodes::Input { .. } | besl::Nodes::Output { .. }))
	}

	/// Validates logical flat-slot intervals and fixed Metal argument-ID reservations before source emission.
	pub(crate) fn validate_reachable_binding_layout(order: &[besl::NodeReference]) -> Result<(), ()> {
		let mut ranges = Vec::new();

		for binding in order {
			let binding = binding.borrow();
			let besl::Nodes::Binding { slot, count, .. } = binding.node() else {
				continue;
			};
			let count = count.map_or(1, |count| count.get());
			let end = slot.checked_add(count).ok_or(())?;
			Self::fixed_argument_ids(*slot, count)?;
			ranges.push((*slot, end));
		}

		// After sorting, adjacent ranges are enough to detect every overlap.
		ranges.sort_unstable_by_key(|(start, _)| *start);
		if ranges.windows(2).any(|ranges| ranges[1].0 < ranges[0].1) {
			return Err(());
		}

		Ok(())
	}

	pub(crate) fn has_non_void_return(function_node: &besl::NodeReference) -> bool {
		matches!(
			function_node.borrow().node(),
			besl::Nodes::Function { return_type, .. } if return_type.borrow().get_name().is_some_and(|name| name != "void")
		)
	}

	pub(crate) fn emit_argument_buffer_parameter(&self, string: &mut String) {
		string.push_str("constant _resources& resources [[buffer(16)]]");
	}

	pub(crate) fn classify_nodes(order: &[besl::NodeReference]) -> ClassifiedNodes<'_> {
		let mut nodes = ClassifiedNodes::default();

		for node in order {
			match node.borrow().node() {
				besl::Nodes::Binding { .. } => nodes.bindings.push(node),
				besl::Nodes::Input { .. } => nodes.inputs.push(node),
				besl::Nodes::Output { .. } => nodes.outputs.push(node),
				besl::Nodes::TaskPayload { .. } => nodes.task_payloads.push(node),
				besl::Nodes::Workgroup { .. } => nodes.workgroups.push(node),
				besl::Nodes::PushConstant { .. } => {
					if nodes.push_constant.is_none() {
						nodes.push_constant = Some(node);
					}
				}
				besl::Nodes::Function { name, .. } if name == "main" => {}
				besl::Nodes::Function { .. } => nodes.functions.push(node),
				besl::Nodes::Struct { .. }
				| besl::Nodes::Raw { .. }
				| besl::Nodes::Intrinsic { .. }
				| besl::Nodes::Const { .. }
				| besl::Nodes::Specialization { .. } => nodes.declarations.push(node),
				_ => {}
			}
		}

		nodes
	}
}

/// Returns the vector type that `expression` reads when the read may come from packed storage: a struct member, a
/// buffer element, or an array element. Locals, parameters, and computed values are always native vectors.
fn packed_vector_read(expression: &besl::NodeReference) -> Option<besl::NodeReference> {
	match expression.borrow().node() {
		besl::Nodes::Expression(besl::Expressions::Expression { elements }) if elements.len() == 1 => {
			return packed_vector_read(&elements[0]);
		}
		besl::Nodes::Expression(besl::Expressions::Accessor { .. }) => {}
		besl::Nodes::Expression(besl::Expressions::Member { source, .. })
			if matches!(source.borrow().node(), besl::Nodes::Member { .. }) => {}
		_ => return None,
	}
	besl::infer_expression_type(expression)
		.filter(|r#type| r#type.borrow().get_name().is_some_and(|name| name.starts_with("vec")))
}

/// Reports whether `expression` produces a matrix value.
fn is_matrix_expression(expression: &besl::NodeReference) -> bool {
	besl::infer_expression_type(expression)
		.is_some_and(|r#type| r#type.borrow().get_name().is_some_and(|name| name.starts_with("mat")))
}
