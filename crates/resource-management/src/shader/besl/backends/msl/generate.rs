use super::*;
impl<A: Allocator + Clone> Generator<A> {
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
		self.generate_in(shader_compilation_settings, main_function_node, self.allocator.clone())
	}

	/// Generates an MSL shader whose resource ABI contains every binding declared by `program`.
	pub fn generate_program(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<String, ()> {
		self.generate_program_in(shader_compilation_settings, program, self.allocator.clone())
	}

	/// Generates a full-program MSL shader using `allocator` for temporary graph and classification storage.
	pub fn generate_program_in(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
		allocator: A,
	) -> Result<String, ()> {
		let previous_allocator = std::mem::replace(&mut self.allocator, allocator);
		let result = self.generate_program_with_current_allocator(shader_compilation_settings, program);
		self.allocator = previous_allocator;
		result
	}

	/// Generates an entry-point MSL shader using `allocator` for temporary graph and classification storage.
	pub fn generate_in(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
		allocator: A,
	) -> Result<String, ()> {
		let previous_allocator = std::mem::replace(&mut self.allocator, allocator);
		let result = self.generate_with_current_allocator(shader_compilation_settings, main_function_node);
		self.allocator = previous_allocator;
		result
	}

	/// Generates code reachable from `main` while retaining every program binding in the Metal resource ABI.
	pub(crate) fn generate_program_with_current_allocator(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		program: &besl::NodeReference,
	) -> Result<String, ()> {
		let main = program.get_main().ok_or(())?;
		let mut order = ordered_shader_nodes_in(&main, "MSL", self.allocator.clone());
		Self::append_declared_bindings(program, &mut order);
		self.generate_order(shader_compilation_settings, &main, &order)
	}

	pub(crate) fn generate_with_current_allocator(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<String, ()> {
		let order = ordered_shader_nodes_in(main_function_node, "MSL", self.allocator.clone());
		self.generate_order(shader_compilation_settings, main_function_node, &order)
	}

	/// Appends authored binding declarations without traversing unreachable executable nodes.
	fn append_declared_bindings(program: &besl::NodeReference, order: &mut Vec<besl::NodeReference, A>) {
		let program_borrow = program.borrow();
		match program_borrow.node() {
			besl::Nodes::Binding { r#type, .. } => {
				if let besl::BindingTypes::Buffer { members } = r#type {
					for member in members {
						Self::append_storage_type_declarations(member, order);
					}
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
	fn append_storage_type_declarations(node: &besl::NodeReference, order: &mut Vec<besl::NodeReference, A>) {
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
		let intrinsic_requirements = Self::collect_intrinsic_requirements(order);
		if intrinsic_requirements.uses_subgroup_intrinsics
			&& !matches!(shader_compilation_settings.stage, Stages::Compute { .. })
		{
			return Err(());
		}
		Self::validate_reachable_binding_layout(order, self.allocator.clone())?;
		self.collect_packed_mat4x3_members(order);
		if matches!(shader_compilation_settings.stage, Stages::Vertex | Stages::Fragment) {
			if let Some(source) = Self::find_full_source_passthrough(main_function_node) {
				return Ok(source);
			}
		}

		let fallback_helper_capacity = if self.downsample_strategy == DownsampleStrategy::ShaderGather
			&& (intrinsic_requirements.uses_downsample_min || intrinsic_requirements.uses_downsample_max)
		{
			4096
		} else {
			0
		};
		let mut string = String::with_capacity(2048 + fallback_helper_capacity);

		self.generate_msl_header_block(&mut string, shader_compilation_settings, &intrinsic_requirements);

		match shader_compilation_settings.stage {
			Stages::Vertex if Self::has_raster_interface(order) => {
				self.generate_vertex_shader(&mut string, order, main_function_node)
			}
			Stages::Fragment if Self::has_raster_interface(order) || Self::has_non_void_return(main_function_node) => {
				self.generate_fragment_shader(&mut string, order, main_function_node)
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
	pub(crate) fn collect_packed_mat4x3_members(&mut self, order: &[besl::NodeReference]) {
		self.packed_mat4x3_members.clear();
		let mut visited_structs = Vec::new();
		for node in order {
			let node = node.borrow();
			let besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} = node.node()
			else {
				continue;
			};
			for member in members {
				self.collect_packed_mat4x3_member(member, &mut visited_structs);
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

	pub(crate) fn packed_mat4x3_member_count(
		&self,
		expression: &besl::NodeReference,
		parent: Option<&besl::NodeReference>,
	) -> Option<Option<usize>> {
		let expression = expression.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { name, source }) = expression.node() else {
			return None;
		};
		let member = if self.is_packed_mat4x3_member(source) {
			source.clone()
		} else {
			let parent_type = parent.and_then(Self::logical_node_type)?;
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
		let besl::Nodes::Member { count, .. } = member.node() else {
			return None;
		};
		Some(count.map(|count| count.get()))
	}

	/// Resolves enough expression types to identify fields of packed storage structs.
	pub(crate) fn logical_node_type(node: &besl::NodeReference) -> Option<besl::NodeReference> {
		let node = node.borrow();
		match node.node() {
			besl::Nodes::Member { r#type, .. } | besl::Nodes::Parameter { r#type, .. } => Some(r#type.clone()),
			besl::Nodes::Expression(besl::Expressions::VariableDeclaration { r#type, .. }) => Some(r#type.clone()),
			besl::Nodes::Expression(besl::Expressions::Member { name, source }) => {
				match source.borrow().node() {
					besl::Nodes::Member { r#type, .. } => return Some(r#type.clone()),
					besl::Nodes::Parameter { .. } | besl::Nodes::Expression(besl::Expressions::VariableDeclaration { .. }) => {
						return Self::logical_node_type(source);
					}
					_ => {}
				}
				let source_type = Self::logical_node_type(source)?;
				let source_type = source_type.borrow();
				let besl::Nodes::Struct { fields, .. } = source_type.node() else {
					return None;
				};
				fields.iter().find_map(|field| match field.borrow().node() {
					besl::Nodes::Member {
						name: field_name,
						r#type,
						..
					} if field_name == name => Some(r#type.clone()),
					_ => None,
				})
			}
			besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
				if matches!(
					right.borrow().node(),
					besl::Nodes::Expression(besl::Expressions::Member { .. })
				) {
					Self::logical_node_type(right)
				} else {
					Self::logical_node_type(left)
				}
			}
			besl::Nodes::Expression(besl::Expressions::FunctionCall { function, .. }) => match function.borrow().node() {
				besl::Nodes::Function { return_type, .. } => Some(return_type.clone()),
				besl::Nodes::Struct { .. } => Some(function.clone()),
				_ => None,
			},
			_ => None,
		}
	}

	/// Reports whether one accessor evaluates to a native matrix loaded from packed storage.
	pub(crate) fn accessor_returns_packed_mat4x3(&self, left: &besl::NodeReference, right: &besl::NodeReference) -> bool {
		if self.packed_mat4x3_member_count(right, Some(left)) == Some(None) {
			return true;
		}

		if matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		) {
			return false;
		}

		if self
			.packed_mat4x3_member_count(left, None)
			.is_some_and(|count| count.is_some())
		{
			return true;
		}

		let left = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::Accessor { right, .. }) = left.node() else {
			return false;
		};
		self.packed_mat4x3_member_count(right, None)
			.is_some_and(|count| count.is_some())
	}

	pub(crate) fn expression_is_packed_mat4x3_accessor(&self, node: &besl::NodeReference) -> bool {
		let node = node.borrow();
		let besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) = node.node() else {
			return false;
		};
		self.accessor_returns_packed_mat4x3(left, right)
	}

	/// Emits an accessor path without converting its final packed matrix storage value.
	pub(crate) fn emit_accessor_expression_raw(
		&mut self,
		string: &mut String,
		left: &besl::NodeReference,
		right: &besl::NodeReference,
	) {
		self.emit_node_string(string, left);
		if left.borrow().node().is_buffer_binding() {
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
	pub(crate) fn validate_reachable_binding_layout(order: &[besl::NodeReference], allocator: A) -> Result<(), ()> {
		let binding_count = order
			.iter()
			.filter(|node| matches!(node.borrow().node(), besl::Nodes::Binding { .. }))
			.count();
		let mut ranges = Vec::with_capacity_in(binding_count, allocator);

		for binding in order {
			let Some((start, end)) = Self::binding_layout(binding)? else {
				continue;
			};

			ranges.push((start, end));
		}

		// After sorting, adjacent ranges are enough to detect every overlap.
		ranges.sort_unstable_by_key(|(start, _)| *start);
		if ranges.windows(2).any(|ranges| ranges[1].0 < ranges[0].1) {
			return Err(());
		}

		Ok(())
	}

	pub(crate) fn binding_layout(binding: &besl::NodeReference) -> Result<Option<(u32, u32)>, ()> {
		let binding = binding.borrow();
		let besl::Nodes::Binding { slot, count, .. } = binding.node() else {
			return Ok(None);
		};

		let count = count.map_or(1, |count| count.get());
		let end = slot.checked_add(count).ok_or(())?;
		Self::fixed_argument_ids(*slot, count)?;

		Ok(Some((*slot, end)))
	}

	pub(crate) fn function_return_type_name(function_node: &besl::NodeReference) -> Option<String> {
		let node = function_node.borrow();
		let besl::Nodes::Function { return_type, .. } = node.node() else {
			return None;
		};
		let return_type_name = return_type.borrow().get_name().map(str::to_string);
		return_type_name
	}

	pub(crate) fn has_non_void_return(function_node: &besl::NodeReference) -> bool {
		Self::function_return_type_name(function_node).is_some_and(|name| name != "void")
	}

	pub(crate) fn emit_argument_buffer_parameter(&self, string: &mut String) {
		string.push_str("constant _resources& resources [[buffer(16)]]");
	}

	pub(crate) fn classify_nodes<'a>(&self, order: &'a [besl::NodeReference]) -> ClassifiedNodes<'a, A> {
		let mut nodes = ClassifiedNodes {
			bindings: Vec::new_in(self.allocator.clone()),
			inputs: Vec::new_in(self.allocator.clone()),
			outputs: Vec::new_in(self.allocator.clone()),
			task_payloads: Vec::new_in(self.allocator.clone()),
			workgroups: Vec::new_in(self.allocator.clone()),
			declarations: Vec::new_in(self.allocator.clone()),
			functions: Vec::new_in(self.allocator.clone()),
			push_constant: None,
		};

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
