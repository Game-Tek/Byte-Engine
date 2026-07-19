/// The `Generator` struct exists to generate Metal Shading Language shaders from BESL ASTs.
///
/// Raster-stage IO uses conventional BESL names for Metal semantics. Vertex inputs named
/// `vertex_id` and `instance_id` are emitted as entry-point parameters with `[[vertex_id]]` and
/// `[[instance_id]]` instead of vertex-attribute struct fields. Fragment inputs named
/// `front_facing` are emitted as a `[[front_facing]]` entry-point parameter. Fragment outputs named
/// `depth`, `stencil`, and `sample_mask` are emitted with their matching Metal attributes; other
/// fragment outputs are emitted as color attachments by location. Fragment shaders may also return
/// an explicit output struct directly. Integer user varyings are emitted as `[[flat]]` user attributes.
///
/// # Parameters
///
/// - *minified*: Controls whether the shader string output is minified. Is `true` by default in release builds.
pub struct Generator<A: Allocator + Clone = Global> {
	allocator: A,
	minified: bool,
	compute_binding_mode: ComputeBindingMode,
	in_compute_body: bool,
	compute_stage_context: Option<ComputeStageContext>,
	raster_stage_context: Option<RasterStageContext>,
	task_stage_context: Option<TaskStageContext>,
	mesh_stage_context: Option<MeshStageContext>,
	in_buffer_binding_struct: bool,
}

const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeBindingMode {
	ArgumentBuffers,
	BareResources,
}

#[derive(Clone, Debug)]
struct MeshStageContext {
	has_resources: bool,
	has_push_constant: bool,
	has_task_payload: bool,
	maximum_vertices: u32,
	maximum_primitives: u32,
}

#[derive(Clone, Debug)]
struct TaskStageContext {
	has_resources: bool,
	has_push_constant: bool,
	has_task_payload: bool,
	workgroups: Vec<TaskWorkgroup>,
}

#[derive(Clone, Debug)]
struct TaskWorkgroup {
	name: String,
	msl_type: String,
}

#[derive(Clone, Debug)]
struct ComputeStageContext {
	has_resources: bool,
	has_push_constant: bool,
}

/// The `RasterStageContext` struct carries the flat argument buffer into binding-dependent raster helpers.
#[derive(Clone, Debug)]
struct RasterStageContext {
	has_resources: bool,
}

struct ClassifiedNodes<'a, A: Allocator + Clone> {
	bindings: Vec<&'a besl::NodeReference, A>,
	inputs: Vec<&'a besl::NodeReference, A>,
	outputs: Vec<&'a besl::NodeReference, A>,
	task_payloads: Vec<&'a besl::NodeReference, A>,
	workgroups: Vec<&'a besl::NodeReference, A>,
	declarations: Vec<&'a besl::NodeReference, A>,
	functions: Vec<&'a besl::NodeReference, A>,
	push_constant: Option<&'a besl::NodeReference>,
}

impl<A: Allocator + Clone> ShaderGenerator for Generator<A> {}

impl Generator<Global> {
	/// Creates a new Generator.
	pub fn new() -> Self {
		Self::new_in(Global)
	}
}

impl<A: Allocator + Clone> Generator<A> {
	/// Creates a new Generator with the allocator used for transient generation buffers.
	pub fn new_in(allocator: A) -> Self {
		Generator {
			allocator,
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			compute_binding_mode: ComputeBindingMode::ArgumentBuffers,
			in_compute_body: false,
			compute_stage_context: None,
			raster_stage_context: None,
			task_stage_context: None,
			mesh_stage_context: None,
			in_buffer_binding_struct: false,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	pub fn compute_binding_mode(mut self, compute_binding_mode: ComputeBindingMode) -> Self {
		self.compute_binding_mode = compute_binding_mode;
		self
	}

	pub fn allocator(&self) -> &A {
		&self.allocator
	}

	/// Detects whether a function's reachable AST needs backend resource parameters.
	fn function_requires_resource_context(&self, function_node: &besl::NodeReference, include_push_constant: bool) -> bool {
		fn node_requires_resource_context<A: Allocator + Clone>(
			node: &besl::NodeReference,
			visited: &mut Vec<besl::NodeReference, A>,
			include_push_constant: bool,
		) -> bool {
			if visited.iter().any(|visited_node| visited_node == node) {
				return false;
			}

			visited.push(node.clone());

			let result = match node.borrow().node() {
				besl::Nodes::Binding { .. } => true,
				besl::Nodes::TaskPayload { .. } => true,
				besl::Nodes::Workgroup { .. } => false,
				besl::Nodes::PushConstant { .. } => include_push_constant,
				besl::Nodes::Scope { children, .. } => children
					.iter()
					.any(|child| node_requires_resource_context(child, visited, include_push_constant)),
				besl::Nodes::Function {
					params,
					return_type,
					statements,
					..
				} => {
					params
						.iter()
						.any(|param| node_requires_resource_context(param, visited, include_push_constant))
						|| node_requires_resource_context(return_type, visited, include_push_constant)
						|| statements
							.iter()
							.any(|statement| node_requires_resource_context(statement, visited, include_push_constant))
				}
				besl::Nodes::Conditional { condition, statements } => {
					node_requires_resource_context(condition, visited, include_push_constant)
						|| statements
							.iter()
							.any(|statement| node_requires_resource_context(statement, visited, include_push_constant))
				}
				besl::Nodes::ForLoop {
					initializer,
					condition,
					update,
					statements,
				} => {
					node_requires_resource_context(initializer, visited, include_push_constant)
						|| node_requires_resource_context(condition, visited, include_push_constant)
						|| node_requires_resource_context(update, visited, include_push_constant)
						|| statements
							.iter()
							.any(|statement| node_requires_resource_context(statement, visited, include_push_constant))
				}
				besl::Nodes::Struct { fields, .. } => fields
					.iter()
					.any(|field| node_requires_resource_context(field, visited, include_push_constant)),
				besl::Nodes::Raw { input, output, .. } => {
					input
						.iter()
						.any(|input| node_requires_resource_context(input, visited, include_push_constant))
						|| output
							.iter()
							.any(|output| node_requires_resource_context(output, visited, include_push_constant))
				}
				besl::Nodes::Parameter { r#type, .. }
				| besl::Nodes::Member { r#type, .. }
				| besl::Nodes::Specialization { r#type, .. }
				| besl::Nodes::Input { format: r#type, .. }
				| besl::Nodes::Output { format: r#type, .. } => node_requires_resource_context(r#type, visited, include_push_constant),
				besl::Nodes::Expression(expression) => match expression {
					besl::Expressions::Operator { left, right, .. } => {
						node_requires_resource_context(left, visited, include_push_constant)
							|| node_requires_resource_context(right, visited, include_push_constant)
					}
					besl::Expressions::FunctionCall {
						function, parameters, ..
					} => {
						node_requires_resource_context(function, visited, include_push_constant)
							|| parameters
								.iter()
								.any(|parameter| node_requires_resource_context(parameter, visited, include_push_constant))
					}
					besl::Expressions::IntrinsicCall { elements, .. } | besl::Expressions::Expression { elements } => elements
						.iter()
						.any(|element| node_requires_resource_context(element, visited, include_push_constant)),
					besl::Expressions::Macro { body, .. } => {
						node_requires_resource_context(body, visited, include_push_constant)
					}
					besl::Expressions::Member { source, .. } => {
						node_requires_resource_context(source, visited, include_push_constant)
					}
					besl::Expressions::VariableDeclaration { r#type, .. } => {
						node_requires_resource_context(r#type, visited, include_push_constant)
					}
					besl::Expressions::Return { value } => value
						.as_ref()
						.is_some_and(|value| node_requires_resource_context(value, visited, include_push_constant)),
					besl::Expressions::Accessor { left, right } => {
						node_requires_resource_context(left, visited, include_push_constant)
							|| node_requires_resource_context(right, visited, include_push_constant)
					}
					besl::Expressions::Literal { .. } | besl::Expressions::Continue => false,
				},
				_ => false,
			};

			visited.pop();
			result
		}

		node_requires_resource_context(function_node, &mut Vec::new_in(self.allocator.clone()), include_push_constant)
	}
}

impl<A: Allocator + Clone> Generator<A> {
	/// Generates an MSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The settings for the shader compilation.
	/// * `main_function_node` - The main function node of the shader.
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

	/// Generates an MSL shader using `allocator` for temporary graph and classification storage.
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

	fn generate_with_current_allocator(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<String, ()> {
		let order = ordered_shader_nodes_in(main_function_node, "MSL", self.allocator.clone());
		Self::validate_reachable_binding_layout(&order)?;
		if matches!(shader_compilation_settings.stage, Stages::Vertex | Stages::Fragment) {
			if let Some(source) = Self::find_full_source_passthrough(main_function_node) {
				return Ok(source);
			}
		}

		let mut string = String::with_capacity(2048);

		self.generate_msl_header_block(&mut string, shader_compilation_settings);

		match shader_compilation_settings.stage {
			Stages::Vertex if Self::has_raster_interface(&order) => {
				self.generate_vertex_shader(&mut string, &order, main_function_node)
			}
			Stages::Fragment if Self::has_raster_interface(&order) || Self::has_non_void_return(main_function_node) => {
				self.generate_fragment_shader(&mut string, &order, main_function_node)
			}
			Stages::Compute { .. } => self.generate_compute_shader(&mut string, &order, main_function_node),
			Stages::Task {
				maximum_mesh_threadgroups,
				..
			} => self.generate_task_shader(&mut string, &order, main_function_node, maximum_mesh_threadgroups),
			Stages::Mesh {
				maximum_vertices,
				maximum_primitives,
				..
			} => self.generate_mesh_shader(&mut string, &order, main_function_node, maximum_vertices, maximum_primitives),
			_ => {
				for node in order {
					self.emit_node_string(&mut string, &node);
				}
			}
		}

		Ok(string)
	}

	fn find_full_source_passthrough(main_function_node: &besl::NodeReference) -> Option<String> {
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

	fn has_raster_interface(order: &[besl::NodeReference]) -> bool {
		order
			.iter()
			.any(|node| matches!(node.borrow().node(), besl::Nodes::Input { .. } | besl::Nodes::Output { .. }))
	}

	/// Validates logical flat-slot intervals and the packed Metal argument-ID space before source emission.
	fn validate_reachable_binding_layout(order: &[besl::NodeReference]) -> Result<(), ()> {
		let mut dense_argument_end = 0u32;

		for (index, binding) in order.iter().enumerate() {
			let Some((start, end, dense_count)) = Self::binding_layout(binding)? else {
				continue;
			};

			dense_argument_end = dense_argument_end.checked_add(dense_count).ok_or(())?;

			// Graph construction already removes repeated references, so any overlapping node here is a distinct declaration.
			for other in &order[index + 1..] {
				let Some((other_start, other_end, _)) = Self::binding_layout(other)? else {
					continue;
				};
				if start < other_end && other_start < end {
					return Err(());
				}
			}
		}

		Ok(())
	}

	fn binding_layout(binding: &besl::NodeReference) -> Result<Option<(u32, u32, u32)>, ()> {
		let binding = binding.borrow();
		let besl::Nodes::Binding { slot, count, r#type, .. } = binding.node() else {
			return Ok(None);
		};

		let count = count.map_or(1, |count| count.get());
		let end = slot.checked_add(count).ok_or(())?;
		let dense_count = if matches!(r#type, besl::BindingTypes::CombinedImageSampler { .. }) {
			count.checked_mul(2).ok_or(())?
		} else {
			count
		};

		Ok(Some((*slot, end, dense_count)))
	}

	fn function_return_type_name(function_node: &besl::NodeReference) -> Option<String> {
		let node = function_node.borrow();
		let besl::Nodes::Function { return_type, .. } = node.node() else {
			return None;
		};
		let return_type_name = return_type.borrow().get_name().map(str::to_string);
		return_type_name
	}

	fn has_non_void_return(function_node: &besl::NodeReference) -> bool {
		Self::function_return_type_name(function_node).is_some_and(|name| name != "void")
	}

	fn emit_argument_buffer_parameter(&self, string: &mut String) {
		string.push_str("constant _resources& resources [[buffer(16)]]");
	}

	fn classify_nodes<'a>(&self, order: &'a [besl::NodeReference]) -> ClassifiedNodes<'a, A> {
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

	fn emit_declarations(&mut self, string: &mut String, nodes: &[&besl::NodeReference]) {
		for node in nodes {
			self.emit_node_string(string, node);
		}
	}

	fn emit_buffer_binding_structs(&mut self, string: &mut String, bindings: &[&besl::NodeReference]) {
		for binding in bindings {
			if let besl::Nodes::Binding {
				r#type: besl::BindingTypes::Buffer { members },
				..
			} = binding.borrow().node()
			{
				self.emit_buffer_binding_struct(string, binding, members.as_slice());
			}
		}
	}

	fn generate_vertex_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
	) {
		let nodes = self.classify_nodes(order);
		self.emit_declarations(string, &nodes.declarations);
		self.emit_buffer_binding_structs(string, &nodes.bindings);

		let bindings = self.sort_bindings_by_slot(nodes.bindings.as_slice());
		if !bindings.is_empty() {
			self.emit_argument_buffer_struct(string, &bindings);
		}

		self.emit_vertex_input_struct(string, &nodes.inputs);
		self.emit_vertex_output_struct(string, &nodes.outputs);
		let previous_raster_stage_context = self.raster_stage_context.replace(RasterStageContext {
			has_resources: !bindings.is_empty(),
		});

		for node in nodes.functions.iter().rev() {
			self.emit_function_prototype(string, node);
		}

		for node in nodes.functions.iter().rev() {
			self.emit_node_string(string, node);
		}

		self.emit_vertex_entry_point(
			string,
			main_function_node,
			&nodes.inputs,
			&nodes.outputs,
			!bindings.is_empty(),
		);
		self.raster_stage_context = previous_raster_stage_context;
	}

	fn emit_vertex_input_struct(&mut self, string: &mut String, inputs: &[&besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "VertexInput");

		for input in inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, location, format } = input.node() else {
				continue;
			};
			if Self::is_vertex_builtin_input(name) {
				continue;
			}
			formatting.push_indentation(string, 1);
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			string.push_str(" [[attribute(");
			string.push_str(location.to_string().as_str());
			string.push_str(")]]");
			formatting.push_statement_end(string);
		}

		self.emit_struct_declaration_end(string);
	}

	fn emit_fragment_input_struct(&mut self, string: &mut String, inputs: &[&besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "FragmentInput");

		for input in inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, location, format } = input.node() else {
				continue;
			};
			if Self::is_fragment_builtin_input(name) {
				continue;
			}
			formatting.push_indentation(string, 1);
			let format = format.borrow();
			let type_name = format.get_name().unwrap();
			string.push_str(Self::translate_type(type_name));
			string.push(' ');
			string.push_str(name);
			if Self::is_integer_type(type_name) {
				string.push_str(" [[flat]]");
			}
			string.push_str(" [[user(locn");
			string.push_str(location.to_string().as_str());
			string.push_str(")]]");
			formatting.push_statement_end(string);
		}

		self.emit_struct_declaration_end(string);
	}

	fn emit_fragment_output_struct(&mut self, string: &mut String, outputs: &[&besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "FragmentOutput");

		for output in outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count,
			} = output.node()
			else {
				continue;
			};
			if count.is_some() {
				continue;
			}
			formatting.push_indentation(string, 1);
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			match name.as_str() {
				"depth" => string.push_str(" [[depth(any)]]"),
				"stencil" => string.push_str(" [[stencil]]"),
				"sample_mask" => string.push_str(" [[sample_mask]]"),
				_ => {
					string.push_str(" [[color(");
					string.push_str(location.to_string().as_str());
					string.push_str(")]]");
				}
			}
			formatting.push_statement_end(string);
		}

		self.emit_struct_declaration_end(string);
	}

	fn emit_vertex_output_struct(&mut self, string: &mut String, outputs: &[&besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "VertexOutput");

		formatting.push_indentation(string, 1);
		string.push_str("float4 position [[position]]");
		formatting.push_statement_end(string);

		for output in outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count,
			} = output.node()
			else {
				continue;
			};
			if count.is_some() || name == "position" {
				continue;
			}
			formatting.push_indentation(string, 1);
			let format = format.borrow();
			let type_name = format.get_name().unwrap();
			string.push_str(Self::translate_type(type_name));
			string.push(' ');
			string.push_str(name);
			if Self::is_integer_type(type_name) {
				string.push_str(" [[flat]]");
			}
			string.push_str(" [[user(locn");
			string.push_str(location.to_string().as_str());
			string.push_str(")]]");
			formatting.push_statement_end(string);
		}

		self.emit_struct_declaration_end(string);
	}

	fn generate_fragment_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
	) {
		let nodes = self.classify_nodes(order);
		self.emit_declarations(string, &nodes.declarations);
		self.emit_buffer_binding_structs(string, &nodes.bindings);

		let bindings = self.sort_bindings_by_slot(nodes.bindings.as_slice());
		if !bindings.is_empty() {
			self.emit_argument_buffer_struct(string, &bindings);
		}

		self.emit_fragment_input_struct(string, &nodes.inputs);
		if !nodes.outputs.is_empty() {
			self.emit_fragment_output_struct(string, &nodes.outputs);
		}
		let previous_raster_stage_context = self.raster_stage_context.replace(RasterStageContext {
			has_resources: !bindings.is_empty(),
		});

		for node in nodes.functions.iter().rev() {
			self.emit_function_prototype(string, node);
		}

		for node in nodes.functions.iter().rev() {
			self.emit_node_string(string, node);
		}

		self.emit_fragment_entry_point(
			string,
			main_function_node,
			&nodes.inputs,
			&nodes.outputs,
			!bindings.is_empty(),
		);
		self.raster_stage_context = previous_raster_stage_context;
	}

	fn emit_raster_input_locals(
		&mut self,
		string: &mut String,
		inputs: &[&besl::NodeReference],
		input_name: &str,
		builtin_values: &[(&str, &str)],
		indent: usize,
	) {
		let formatting = ShaderFormatting::new(self.minified);
		for input in inputs {
			let input = input.borrow();
			let besl::Nodes::Input { name, format, .. } = input.node() else {
				continue;
			};
			let builtin_value = builtin_values
				.iter()
				.find_map(|(builtin_name, value)| (builtin_name == name).then_some(*value));
			// Builtin entry-point parameters already use their BESL names and need no local mirror.
			if builtin_value == Some(name.as_str()) {
				continue;
			}
			formatting.push_indentation(string, indent);
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			string.push('=');
			if let Some(value) = builtin_value {
				string.push_str(value);
			} else {
				string.push_str(input_name);
				string.push('.');
				string.push_str(name);
			}
			formatting.push_statement_end(string);
		}
	}

	fn emit_raster_output_locals(&mut self, string: &mut String, outputs: &[&besl::NodeReference], indent: usize) {
		let formatting = ShaderFormatting::new(self.minified);
		for output in outputs {
			let output = output.borrow();
			let besl::Nodes::Output { name, format, count, .. } = output.node() else {
				continue;
			};
			if count.is_some() {
				continue;
			}
			formatting.push_indentation(string, indent);
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			formatting.push_statement_end(string);
		}
	}

	fn emit_raster_output_assignments(
		&mut self,
		string: &mut String,
		outputs: &[&besl::NodeReference],
		output_name: &str,
		indent: usize,
	) {
		let formatting = ShaderFormatting::new(self.minified);
		for output in outputs {
			let output = output.borrow();
			let besl::Nodes::Output { name, count, .. } = output.node() else {
				continue;
			};
			if count.is_some() {
				continue;
			}
			formatting.push_indentation(string, indent);
			string.push_str(output_name);
			string.push('.');
			string.push_str(name);
			string.push('=');
			string.push_str(name);
			formatting.push_statement_end(string);
		}
	}

	fn emit_vertex_entry_point(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		inputs: &[&besl::NodeReference],
		outputs: &[&besl::NodeReference],
		has_resources: bool,
	) {
		let node = RefCell::borrow(main_function_node);
		let besl::Nodes::Function { statements, .. } = node.node() else {
			return;
		};
		let formatting = ShaderFormatting::new(self.minified);

		string.push_str("vertex VertexOutput besl_main(VertexInput in [[stage_in]]");
		if inputs
			.iter()
			.any(|input| matches!(input.borrow().node(), besl::Nodes::Input { name, .. } if name == "vertex_id"))
		{
			self.emit_separator(string);
			string.push_str("uint vertex_id [[vertex_id]]");
		}
		if inputs
			.iter()
			.any(|input| matches!(input.borrow().node(), besl::Nodes::Input { name, .. } if name == "instance_id"))
		{
			self.emit_separator(string);
			string.push_str("uint instance_id [[instance_id]]");
		}
		if has_resources {
			self.emit_separator(string);
			self.emit_argument_buffer_parameter(string);
		}

		formatting.push_block_start(string);

		// Mirror BESL global stage inputs and outputs through local variables so both ordinary BESL
		// assignments and raw statement snippets lower to a valid Metal entry point.
		self.emit_raster_input_locals(
			string,
			inputs,
			"in",
			&[("vertex_id", "vertex_id"), ("instance_id", "instance_id")],
			1,
		);
		formatting.push_indentation(string, 1);
		string.push_str("VertexOutput out");
		formatting.push_statement_end(string);
		self.emit_raster_output_locals(string, outputs, 1);

		self.emit_statement_block(string, statements, 1);

		self.emit_raster_output_assignments(string, outputs, "out", 1);
		formatting.push_indentation(string, 1);
		string.push_str("return out");
		formatting.push_statement_end(string);

		self.emit_block_end(string);
	}

	fn emit_fragment_entry_point(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		inputs: &[&besl::NodeReference],
		outputs: &[&besl::NodeReference],
		has_resources: bool,
	) {
		let node = RefCell::borrow(main_function_node);
		let besl::Nodes::Function {
			statements, return_type, ..
		} = node.node()
		else {
			return;
		};
		let formatting = ShaderFormatting::new(self.minified);
		let return_type_name = return_type.borrow().get_name().unwrap_or("void").to_string();
		let returns_explicit_output = return_type_name != "void";
		let entry_return_type = if returns_explicit_output {
			Self::translate_type(&return_type_name).to_string()
		} else {
			"FragmentOutput".to_string()
		};

		string.push_str("fragment ");
		string.push_str(&entry_return_type);
		string.push_str(" besl_main(FragmentInput in [[stage_in]]");
		if inputs
			.iter()
			.any(|input| matches!(input.borrow().node(), besl::Nodes::Input { name, .. } if name == "front_facing"))
		{
			self.emit_separator(string);
			string.push_str("bool front_facing [[front_facing]]");
		}
		if has_resources {
			self.emit_separator(string);
			self.emit_argument_buffer_parameter(string);
		}

		formatting.push_block_start(string);

		// Mirror BESL global stage inputs through local variables so ordinary BESL can read
		// stage inputs while explicit output structs can be returned directly.
		self.emit_raster_input_locals(string, inputs, "in", &[("front_facing", "front_facing")], 1);

		if returns_explicit_output {
			self.emit_statement_block(string, statements, 1);
		} else {
			formatting.push_indentation(string, 1);
			string.push_str("FragmentOutput out");
			formatting.push_statement_end(string);
			self.emit_raster_output_locals(string, outputs, 1);

			self.emit_statement_block(string, statements, 1);

			self.emit_raster_output_assignments(string, outputs, "out", 1);
			formatting.push_indentation(string, 1);
			string.push_str("return out");
			formatting.push_statement_end(string);
		}

		self.emit_block_end(string);
	}

	fn is_vertex_builtin_input(name: &str) -> bool {
		matches!(name, "vertex_id" | "instance_id")
	}

	fn is_fragment_builtin_input(name: &str) -> bool {
		matches!(name, "front_facing")
	}

	fn is_integer_type(name: &str) -> bool {
		matches!(
			name,
			"u8" | "u16" | "u32" | "i32" | "vec2u" | "vec2u16" | "vec4u16" | "vec2i" | "vec3u" | "vec4u"
		)
	}

	fn generate_compute_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
	) {
		let nodes = self.classify_nodes(order);
		self.emit_declarations(string, &nodes.declarations);
		self.emit_declarations(string, &nodes.inputs);
		self.emit_declarations(string, &nodes.outputs);

		if let Some(push_constant) = nodes.push_constant {
			self.emit_push_constant_struct(string, push_constant);
		}

		let bindings = self.sort_bindings_by_slot(nodes.bindings.as_slice());
		let previous_compute_stage_context = self.compute_stage_context.replace(ComputeStageContext {
			has_resources: !bindings.is_empty(),
			has_push_constant: nodes.push_constant.is_some(),
		});
		let previous_in_compute_body = self.in_compute_body;
		self.in_compute_body = true;

		self.emit_buffer_binding_structs(string, &nodes.bindings);

		if matches!(self.compute_binding_mode, ComputeBindingMode::ArgumentBuffers) && !bindings.is_empty() {
			self.emit_argument_buffer_struct(string, &bindings);
		}

		for node in nodes.functions.iter().rev() {
			self.emit_function_prototype(string, node);
		}

		for node in nodes.functions.iter().rev() {
			self.emit_node_string(string, node);
		}

		match self.compute_binding_mode {
			ComputeBindingMode::ArgumentBuffers => {
				self.emit_compute_entry_point_argument_buffers(
					string,
					main_function_node,
					!bindings.is_empty(),
					nodes.push_constant,
				);
			}
			ComputeBindingMode::BareResources => {
				self.emit_compute_entry_point_bare_resources(
					string,
					main_function_node,
					nodes.bindings.as_slice(),
					nodes.push_constant,
				);
			}
		}

		self.in_compute_body = previous_in_compute_body;
		self.compute_stage_context = previous_compute_stage_context;
	}

	fn generate_task_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
		maximum_mesh_threadgroups: u32,
	) {
		let nodes = self.classify_nodes(order);
		if let Some(push_constant) = nodes.push_constant {
			self.emit_push_constant_struct(string, push_constant);
		}

		let bindings = self.sort_bindings_by_slot(nodes.bindings.as_slice());
		let workgroups = nodes
			.workgroups
			.iter()
			.filter_map(|workgroup| {
				let workgroup = workgroup.borrow();
				let besl::Nodes::Workgroup { name, format } = workgroup.node() else {
					return None;
				};
				let msl_type = Self::translate_type(format.borrow().get_name().unwrap()).to_string();
				Some(TaskWorkgroup {
					name: name.clone(),
					msl_type,
				})
			})
			.collect();
		let previous_task_stage_context = self.task_stage_context.replace(TaskStageContext {
			has_resources: !bindings.is_empty(),
			has_push_constant: nodes.push_constant.is_some(),
			has_task_payload: !nodes.task_payloads.is_empty(),
			workgroups,
		});
		let previous_in_compute_body = self.in_compute_body;
		self.in_compute_body = true;

		self.emit_declarations(string, &nodes.declarations);
		self.emit_buffer_binding_structs(string, &nodes.bindings);
		if !bindings.is_empty() {
			self.emit_argument_buffer_struct(string, &bindings);
		}
		self.emit_object_payload_struct(string, &nodes.task_payloads);

		for node in nodes.functions.iter().rev() {
			self.emit_function_prototype(string, node);
		}
		for node in nodes.functions.iter().rev() {
			self.emit_node_string(string, node);
		}

		self.emit_task_entry_point(
			string,
			main_function_node,
			!bindings.is_empty(),
			nodes.push_constant,
			&nodes.task_payloads,
			&nodes.workgroups,
			maximum_mesh_threadgroups,
		);

		self.in_compute_body = previous_in_compute_body;
		self.task_stage_context = previous_task_stage_context;
	}

	fn generate_mesh_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
		maximum_vertices: u32,
		maximum_primitives: u32,
	) {
		let nodes = self.classify_nodes(order);
		if let Some(push_constant) = nodes.push_constant {
			self.emit_push_constant_struct(string, push_constant);
		}

		let bindings = self.sort_bindings_by_slot(nodes.bindings.as_slice());
		let previous_mesh_stage_context = self.mesh_stage_context.replace(MeshStageContext {
			has_resources: !bindings.is_empty(),
			has_push_constant: nodes.push_constant.is_some(),
			has_task_payload: !nodes.task_payloads.is_empty(),
			maximum_vertices,
			maximum_primitives,
		});
		self.emit_declarations(string, &nodes.declarations);
		self.emit_declarations(string, &nodes.inputs);
		self.emit_buffer_binding_structs(string, &nodes.bindings);

		if !bindings.is_empty() {
			self.emit_argument_buffer_struct(string, &bindings);
		}
		self.emit_object_payload_struct(string, &nodes.task_payloads);

		if !Self::has_raw_mesh_output_structs(&nodes.declarations) {
			self.emit_mesh_output_structs(string, &nodes.outputs);
		}

		for node in nodes.functions.iter().rev() {
			self.emit_function_prototype(string, node);
		}

		for node in nodes.functions.iter().rev() {
			self.emit_node_string(string, node);
		}

		self.emit_mesh_entry_point_argument_buffers(
			string,
			main_function_node,
			!bindings.is_empty(),
			nodes.push_constant,
			!nodes.task_payloads.is_empty(),
			maximum_vertices,
			maximum_primitives,
		);

		self.mesh_stage_context = previous_mesh_stage_context;
	}

	fn has_raw_mesh_output_structs(nodes: &[&besl::NodeReference]) -> bool {
		nodes.iter().any(|node| match node.borrow().node() {
			besl::Nodes::Raw { msl, hlsl, .. } => msl
				.as_ref()
				.or(hlsl.as_ref())
				.is_some_and(|source| source.contains("struct VertexOutput") || source.contains("struct PrimitiveOutput")),
			_ => false,
		})
	}

	fn mesh_output_field_name(name: &str) -> &str {
		name.strip_prefix("out_").unwrap_or(name)
	}

	fn emit_mesh_output_structs(&mut self, string: &mut String, outputs: &[&besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "VertexOutput");
		formatting.push_indentation(string, 1);
		string.push_str("float4 position [[position]]");
		formatting.push_statement_end(string);
		self.emit_struct_declaration_end(string);

		self.emit_named_struct_start(string, "PrimitiveOutput");
		for output in outputs {
			let output = output.borrow();
			let besl::Nodes::Output {
				name,
				location,
				format,
				count,
			} = output.node()
			else {
				continue;
			};
			if count.is_none() {
				continue;
			}

			formatting.push_indentation(string, 1);
			let format = format.borrow();
			let type_name = format.get_name().unwrap();
			string.push_str(Self::translate_type(type_name));
			string.push(' ');
			string.push_str(Self::mesh_output_field_name(name));
			if Self::is_integer_type(type_name) {
				string.push_str(" [[flat]]");
			}
			string.push_str(" [[user(locn");
			string.push_str(location.to_string().as_str());
			string.push_str(")]]");
			formatting.push_statement_end(string);
		}
		self.emit_struct_declaration_end(string);
	}

	/// Returns the resources in logical-slot order so Metal argument IDs are packed deterministically.
	fn sort_bindings_by_slot<'a>(&self, bindings: &[&'a besl::NodeReference]) -> Vec<&'a besl::NodeReference, A> {
		let mut sorted = Vec::with_capacity_in(bindings.len(), self.allocator.clone());
		sorted.extend_from_slice(bindings);
		sorted.sort_by_key(|binding| match binding.borrow().node() {
			besl::Nodes::Binding { slot, .. } => *slot,
			_ => u32::MAX,
		});
		sorted
	}

	fn emit_push_constant_struct(&mut self, string: &mut String, push_constant: &besl::NodeReference) {
		let node = push_constant.borrow();
		let besl::Nodes::PushConstant { members } = node.node() else {
			return;
		};

		self.emit_named_struct_start(string, "PushConstant");

		for member in members {
			self.emit_indentation(string, 1);
			self.emit_node_string(string, member);
			self.emit_statement_end(string);
		}

		self.emit_struct_declaration_end(string);
	}

	fn emit_object_payload_struct(&mut self, string: &mut String, payloads: &[&besl::NodeReference]) {
		if payloads.is_empty() {
			return;
		}

		self.emit_named_struct_start(string, "ObjectPayload");
		for payload in payloads {
			let payload = payload.borrow();
			let besl::Nodes::TaskPayload { name, format, count } = payload.node() else {
				continue;
			};

			self.emit_indentation(string, 1);
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			string.push('[');
			string.push_str(count.get().to_string().as_str());
			string.push(']');
			self.emit_statement_end(string);
		}
		self.emit_struct_declaration_end(string);
	}

	fn emit_argument_buffer_struct(&mut self, string: &mut String, bindings: &[&besl::NodeReference]) {
		self.emit_named_struct_start(string, "_resources");

		let mut next_id = 0u32;
		for binding in bindings {
			self.emit_argument_buffer_field(string, binding, &mut next_id);
		}

		self.emit_struct_declaration_end(string);
	}

	fn emit_argument_buffer_field(&mut self, string: &mut String, binding_node: &besl::NodeReference, next_id: &mut u32) {
		let node = binding_node.borrow();
		let besl::Nodes::Binding {
			name,
			read,
			write,
			r#type,
			count,
			..
		} = node.node()
		else {
			return;
		};

		let emit_suffix = |string: &mut String, next_id: &mut u32| {
			string.push_str(" [[id(");
			let _ = write!(string, "{next_id}");
			string.push_str(")]]");
			let descriptor_count = count.map(|count| count.get()).unwrap_or(1);
			if let Some(count) = count {
				string.push('[');
				let _ = write!(string, "{count}");
				string.push(']');
			}
			self.emit_statement_end(string);
			*next_id = next_id.checked_add(descriptor_count).expect(
				"Invalid dense Metal argument ID range. The most likely cause is that binding validation was bypassed before source emission.",
			);
		};

		self.emit_indentation(string, 1);

		match r#type {
			besl::BindingTypes::Buffer { .. } => {
				let address_space = if *write { "device" } else { "constant" };
				string.push_str(address_space);
				string.push(' ');
				string.push_str(&format!("_{}* {}", name, name));
				emit_suffix(string, next_id);
			}
			besl::BindingTypes::Image { format } => {
				let element_type = match format.as_str() {
					"r8ui" | "r16ui" | "r32ui" => "uint",
					_ => "float",
				};
				let access = if *read && *write {
					"access::read_write"
				} else if *write {
					"access::write"
				} else {
					"access::read"
				};
				string.push_str(&format!("texture2d<{}, {}> {}", element_type, access, name));
				emit_suffix(string, next_id);
			}
			besl::BindingTypes::CombinedImageSampler { format } => {
				let texture_type = match format.as_str() {
					"Texture3D" => "texture3d<float>",
					"ArrayTexture2D" => "texture2d_array<float>",
					"r8ui" | "r16ui" | "r32ui" => "texture2d<uint>",
					_ => "texture2d<float>",
				};
				string.push_str(texture_type);
				string.push(' ');
				string.push_str(name);
				emit_suffix(string, next_id);

				self.emit_indentation(string, 1);
				string.push_str("sampler ");
				string.push_str(&format!("{}_sampler", name));
				emit_suffix(string, next_id);
			}
		}
	}

	fn emit_buffer_binding_struct(
		&mut self,
		string: &mut String,
		binding_node: &besl::NodeReference,
		members: &[besl::NodeReference],
	) {
		let binding = binding_node.borrow();
		let besl::Nodes::Binding { name, .. } = binding.node() else {
			return;
		};

		self.emit_named_struct_start(string, &format!("_{name}"));

		let previous_in_buffer_binding_struct = self.in_buffer_binding_struct;
		self.in_buffer_binding_struct = true;

		for member in members {
			self.emit_indentation(string, 1);
			self.emit_node_string(string, member);
			self.emit_statement_end(string);
		}

		self.in_buffer_binding_struct = previous_in_buffer_binding_struct;

		self.emit_struct_declaration_end(string);
	}

	fn translate_buffer_member_type(source: &str) -> &str {
		// Metal storage buffers need packed vectors when the CPU data is tightly packed.
		// Float vectors retain the existing array-only policy, while u16 vectors must also stay packed inside mixed structs.
		match source {
			"vec2f" => "packed_float2",
			"vec3f" => "packed_float3",
			"vec2u16" => "packed_ushort2",
			"vec4u16" => "packed_ushort4",
			_ => Self::translate_type(source),
		}
	}

	fn emit_compute_entry_point_bare_resources(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		bindings: &[&besl::NodeReference],
		push_constant: Option<&besl::NodeReference>,
	) {
		let node = RefCell::borrow(main_function_node);

		let besl::Nodes::Function {
			name,
			statements,
			params,
			..
		} = node.node()
		else {
			return;
		};

		string.push_str("kernel void ");
		if *name == "main" {
			string.push_str("besl_main");
		} else {
			string.push_str(name);
		}
		string.push('(');
		string.push_str("uint2 gid [[thread_position_in_grid]]");

		for param in params {
			self.emit_separator(string);
			self.emit_node_string(string, param);
		}

		if let Some(push_constant) = push_constant {
			self.emit_separator(string);
			self.emit_compute_push_constant_parameter(string, push_constant);
		}

		for binding in bindings {
			self.emit_compute_binding_parameter(string, binding);
		}

		ShaderFormatting::new(self.minified).push_block_start(string);

		self.emit_statement_block(string, statements, 1);

		self.emit_block_end(string);
	}

	fn emit_compute_entry_point_argument_buffers(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		has_resources: bool,
		push_constant: Option<&besl::NodeReference>,
	) {
		let node = RefCell::borrow(main_function_node);

		let besl::Nodes::Function {
			name,
			statements,
			params,
			..
		} = node.node()
		else {
			return;
		};

		string.push_str("kernel void ");
		if *name == "main" {
			string.push_str("besl_main");
		} else {
			string.push_str(name);
		}
		string.push('(');
		string.push_str("uint2 gid [[thread_position_in_grid]]");

		for param in params {
			self.emit_separator(string);
			self.emit_node_string(string, param);
		}

		if let Some(push_constant) = push_constant {
			self.emit_separator(string);
			self.emit_compute_push_constant_parameter(string, push_constant);
		}

		if has_resources {
			self.emit_separator(string);
			self.emit_argument_buffer_parameter(string);
		}

		ShaderFormatting::new(self.minified).push_block_start(string);

		self.emit_statement_block(string, statements, 1);

		self.emit_block_end(string);
	}

	fn emit_task_entry_point(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		has_resources: bool,
		push_constant: Option<&besl::NodeReference>,
		task_payloads: &[&besl::NodeReference],
		workgroups: &[&besl::NodeReference],
		maximum_mesh_threadgroups: u32,
	) {
		let node = RefCell::borrow(main_function_node);
		let besl::Nodes::Function {
			name,
			statements,
			params,
			..
		} = node.node()
		else {
			return;
		};

		string.push_str("[[object, max_total_threadgroups_per_mesh_grid(");
		string.push_str(maximum_mesh_threadgroups.to_string().as_str());
		string.push_str(")]] void ");
		if *name == "main" {
			string.push_str("besl_main");
		} else {
			string.push_str(name);
		}
		string.push('(');

		let mut has_previous_parameter = false;
		for param in params {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			self.emit_node_string(string, param);
			has_previous_parameter = true;
		}

		if let Some(push_constant) = push_constant {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			self.emit_mesh_push_constant_parameter(string, push_constant);
			has_previous_parameter = true;
		}
		if has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			self.emit_argument_buffer_parameter(string);
			has_previous_parameter = true;
		}
		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("uint thread_position [[thread_position_in_grid]]");
		self.emit_separator(string);
		string.push_str("uint thread_index [[thread_index_in_threadgroup]]");
		if !task_payloads.is_empty() {
			self.emit_separator(string);
			string.push_str("object_data ObjectPayload& payload [[payload]]");
		}
		self.emit_separator(string);
		string.push_str("mesh_grid_properties mesh_grid");

		ShaderFormatting::new(self.minified).push_block_start(string);
		for workgroup in workgroups {
			let workgroup = workgroup.borrow();
			let besl::Nodes::Workgroup { name, format } = workgroup.node() else {
				continue;
			};
			self.emit_indentation(string, 1);
			string.push_str("threadgroup ");
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			self.emit_statement_end(string);
		}
		self.emit_statement_block(string, statements, 1);
		self.emit_block_end(string);
	}

	fn emit_mesh_entry_point_argument_buffers(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		has_resources: bool,
		push_constant: Option<&besl::NodeReference>,
		has_task_payload: bool,
		maximum_vertices: u32,
		maximum_primitives: u32,
	) {
		let node = RefCell::borrow(main_function_node);

		let besl::Nodes::Function {
			name,
			statements,
			params,
			..
		} = node.node()
		else {
			return;
		};

		string.push_str("[[mesh]] void ");
		if *name == "main" {
			string.push_str("besl_main");
		} else {
			string.push_str(name);
		}
		string.push('(');

		let mut has_previous_parameter = false;
		for param in params {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			self.emit_node_string(string, param);
			has_previous_parameter = true;
		}

		if let Some(push_constant) = push_constant {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			self.emit_mesh_push_constant_parameter(string, push_constant);
			has_previous_parameter = true;
		}

		if has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			self.emit_argument_buffer_parameter(string);
			has_previous_parameter = true;
		}

		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("uint threadgroup_position [[threadgroup_position_in_grid]]");
		self.emit_separator(string);
		string.push_str("uint thread_index [[thread_index_in_threadgroup]]");
		if has_task_payload {
			self.emit_separator(string);
			string.push_str("const object_data ObjectPayload& payload [[payload]]");
		}
		self.emit_separator(string);
		string.push_str(&format!(
			"metal::mesh<VertexOutput, PrimitiveOutput, {}, {}, topology::triangle> out_mesh",
			maximum_vertices, maximum_primitives
		));

		ShaderFormatting::new(self.minified).push_block_start(string);

		self.emit_statement_block(string, statements, 1);

		self.emit_block_end(string);
	}

	fn emit_mesh_push_constant_parameter(&self, string: &mut String, _push_constant: &besl::NodeReference) {
		string.push_str(&format!(
			"constant PushConstant& push_constant [[buffer({})]]",
			PUSH_CONSTANT_BINDING_INDEX
		));
	}

	fn emit_compute_push_constant_parameter(&self, string: &mut String, _push_constant: &besl::NodeReference) {
		string.push_str(&format!(
			"constant PushConstant& push_constant [[buffer({})]]",
			PUSH_CONSTANT_BINDING_INDEX
		));
	}

	fn emit_compute_binding_parameter(&self, string: &mut String, binding_node: &besl::NodeReference) {
		let node = binding_node.borrow();
		let besl::Nodes::Binding {
			name,
			slot,
			read,
			write,
			r#type,
			..
		} = node.node()
		else {
			return;
		};

		let index = *slot;

		match r#type {
			besl::BindingTypes::Buffer { .. } => {
				let address_space = if *write { "device" } else { "constant" };
				self.emit_separator(string);
				string.push_str(address_space);
				string.push(' ');
				string.push_str(&format!("_{}* {} [[buffer({})]]", name, name, index));
			}
			besl::BindingTypes::Image { format } => {
				let element_type = match format.as_str() {
					"r8ui" | "r16ui" | "r32ui" => "uint",
					_ => "float",
				};
				let access = if *read && *write {
					"access::read_write"
				} else if *write {
					"access::write"
				} else {
					"access::read"
				};

				self.emit_separator(string);
				string.push_str(&format!(
					"texture2d<{}, {}> {} [[texture({})]]",
					element_type, access, name, index
				));
			}
			besl::BindingTypes::CombinedImageSampler { format } => {
				let texture_type = match format.as_str() {
					"Texture3D" => "texture3d<float>",
					"ArrayTexture2D" => "texture2d_array<float>",
					_ => "texture2d<float>",
				};

				self.emit_separator(string);
				string.push_str(&format!("{} {} [[texture({})]]", texture_type, name, index));
				self.emit_separator(string);
				string.push_str(&format!("sampler {}_sampler [[sampler({})]]", name, index));
			}
		}
	}

	fn emit_compute_binding_reference(&self, string: &mut String, name: &str) {
		if self.mesh_stage_context.is_some() {
			string.push_str("resources.");
			string.push_str(name);
			return;
		}

		match self.compute_binding_mode {
			ComputeBindingMode::ArgumentBuffers => {
				string.push_str("resources.");
				string.push_str(name);
			}
			ComputeBindingMode::BareResources => string.push_str(name),
		}
	}

	/// Qualifies a raster resource through the argument buffer supplied to its entry point or helper.
	fn emit_raster_binding_reference(&self, string: &mut String, name: &str) {
		string.push_str("resources.");
		string.push_str(name);
	}

	fn emit_task_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(task_stage_context) = &self.task_stage_context else {
			return;
		};

		let mut has_previous_parameter = has_previous_parameter;
		if task_stage_context.has_push_constant {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("constant PushConstant& push_constant");
			has_previous_parameter = true;
		}
		if task_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("constant _resources& resources");
			has_previous_parameter = true;
		}
		if task_stage_context.has_task_payload {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("object_data ObjectPayload& payload");
			has_previous_parameter = true;
		}
		for parameter in ["uint thread_position", "uint thread_index"] {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str(parameter);
			has_previous_parameter = true;
		}
		for workgroup in &task_stage_context.workgroups {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("threadgroup ");
			string.push_str(&workgroup.msl_type);
			string.push_str("& ");
			string.push_str(&workgroup.name);
			has_previous_parameter = true;
		}
		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("thread mesh_grid_properties& mesh_grid");
	}

	fn emit_task_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(task_stage_context) = &self.task_stage_context else {
			return;
		};

		let mut has_previous_parameter = has_previous_parameter;
		if task_stage_context.has_push_constant {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("push_constant");
			has_previous_parameter = true;
		}
		if task_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("resources");
			has_previous_parameter = true;
		}
		if task_stage_context.has_task_payload {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("payload");
			has_previous_parameter = true;
		}
		for argument in ["thread_position", "thread_index"] {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str(argument);
			has_previous_parameter = true;
		}
		for workgroup in &task_stage_context.workgroups {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str(&workgroup.name);
			has_previous_parameter = true;
		}
		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("mesh_grid");
	}

	fn emit_mesh_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(mesh_stage_context) = &self.mesh_stage_context else {
			return;
		};

		let mut has_previous_parameter = has_previous_parameter;

		if mesh_stage_context.has_push_constant {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("constant PushConstant& push_constant");
			has_previous_parameter = true;
		}

		if mesh_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("constant _resources& resources");
			has_previous_parameter = true;
		}

		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("uint threadgroup_position");
		self.emit_separator(string);
		string.push_str("uint thread_index");
		if mesh_stage_context.has_task_payload {
			self.emit_separator(string);
			string.push_str("const object_data ObjectPayload& payload");
		}
		self.emit_separator(string);
		string.push_str(&format!(
			"metal::mesh<VertexOutput, PrimitiveOutput, {}, {}, topology::triangle> out_mesh",
			mesh_stage_context.maximum_vertices, mesh_stage_context.maximum_primitives
		));
	}

	fn emit_mesh_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(mesh_stage_context) = &self.mesh_stage_context else {
			return;
		};

		let mut has_previous_parameter = has_previous_parameter;

		if mesh_stage_context.has_push_constant {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("push_constant");
			has_previous_parameter = true;
		}

		if mesh_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("resources");
			has_previous_parameter = true;
		}

		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("threadgroup_position");
		self.emit_separator(string);
		string.push_str("thread_index");
		if mesh_stage_context.has_task_payload {
			self.emit_separator(string);
			string.push_str("payload");
		}
		self.emit_separator(string);
		string.push_str("out_mesh");
	}

	fn emit_compute_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		if self.mesh_stage_context.is_some() {
			self.emit_mesh_hidden_parameters(string, has_previous_parameter);
			return;
		}

		let Some(compute_stage_context) = &self.compute_stage_context else {
			return;
		};

		if !self.in_compute_body {
			return;
		}

		let mut has_previous_parameter = has_previous_parameter;

		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("uint2 gid");
		has_previous_parameter = true;

		if compute_stage_context.has_push_constant {
			self.emit_separator(string);
			string.push_str("constant PushConstant& push_constant");
			has_previous_parameter = true;
		}

		if compute_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("constant _resources& resources");
		}
	}

	/// Adds argument-buffer parameters to raster helpers that access BESL bindings.
	fn emit_raster_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(raster_stage_context) = &self.raster_stage_context else {
			return;
		};

		if raster_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("constant _resources& resources");
		}
	}

	fn emit_compute_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
		if self.mesh_stage_context.is_some() {
			self.emit_mesh_hidden_call_arguments(string, has_previous_parameter);
			return;
		}

		let Some(compute_stage_context) = &self.compute_stage_context else {
			return;
		};

		if !self.in_compute_body {
			return;
		}

		let mut has_previous_parameter = has_previous_parameter;

		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("gid");
		has_previous_parameter = true;

		if compute_stage_context.has_push_constant {
			self.emit_separator(string);
			string.push_str("push_constant");
			has_previous_parameter = true;
		}

		if compute_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("resources");
		}
	}

	/// Forwards entry-point argument buffers to binding-dependent raster helpers.
	fn emit_raster_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(raster_stage_context) = &self.raster_stage_context else {
			return;
		};

		if raster_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("resources");
		}
	}

	fn emit_function_prototype(&mut self, string: &mut String, function_node: &besl::NodeReference) {
		let node = RefCell::borrow(function_node);
		let besl::Nodes::Function {
			name,
			return_type,
			params,
			..
		} = node.node()
		else {
			return;
		};

		string.push_str(Self::translate_type(return_type.borrow().get_name().unwrap()));
		string.push(' ');
		string.push_str(name);
		string.push('(');

		let formatting = ShaderFormatting::new(self.minified);
		emit_comma_separated_nodes(string, formatting, params, |string, param| {
			self.emit_node_string(string, param)
		});

		if self.task_stage_context.is_some() {
			self.emit_task_hidden_parameters(string, !params.is_empty());
		} else if self.in_compute_body && self.function_requires_resource_context(function_node, true) {
			self.emit_compute_hidden_parameters(string, !params.is_empty());
		} else if self.raster_stage_context.is_some() && self.function_requires_resource_context(function_node, false) {
			self.emit_raster_hidden_parameters(string, !params.is_empty());
		}

		string.push(')');
		self.emit_statement_end(string);
	}

	fn mesh_output_assignment_parts(
		&mut self,
		statement: &besl::NodeReference,
	) -> Option<(String, besl::NodeReference, besl::NodeReference)> {
		let node = statement.borrow();
		let besl::Nodes::Expression(besl::Expressions::Operator {
			operator: besl::Operators::Assignment,
			left,
			right,
		}) = node.node()
		else {
			return None;
		};

		let left_node = left.borrow();
		let besl::Nodes::Expression(besl::Expressions::Accessor {
			left: output,
			right: index,
		}) = left_node.node()
		else {
			return None;
		};

		let output_node = output.borrow();
		let besl::Nodes::Expression(besl::Expressions::Member { source, .. }) = output_node.node() else {
			return None;
		};

		let source = source.borrow();
		let besl::Nodes::Output { name, count, .. } = source.node() else {
			return None;
		};

		if count.is_none() {
			return None;
		}

		if name != "out_instance_index" && name != "out_primitive_index" {
			return None;
		}

		Some((name.clone(), index.clone(), right.clone()))
	}

	fn emit_statement_block(&mut self, string: &mut String, statements: &[besl::NodeReference], indent: usize) {
		let formatting = ShaderFormatting::new(self.minified);
		let mut i = 0;

		while i < statements.len() {
			if self.mesh_stage_context.is_some() && i + 1 < statements.len() {
				let current = self.mesh_output_assignment_parts(&statements[i]);
				let next = self.mesh_output_assignment_parts(&statements[i + 1]);

				if let (Some((current_name, current_index, current_value)), Some((next_name, next_index, next_value))) =
					(current, next)
				{
					let mut current_index_string = String::new();
					self.emit_node_string(&mut current_index_string, &current_index);
					let mut next_index_string = String::new();
					self.emit_node_string(&mut next_index_string, &next_index);

					if current_index_string == next_index_string
						&& current_name != next_name
						&& ((current_name == "out_instance_index" && next_name == "out_primitive_index")
							|| (current_name == "out_primitive_index" && next_name == "out_instance_index"))
					{
						let (instance_value, primitive_value) = if current_name == "out_instance_index" {
							(current_value, next_value)
						} else {
							(next_value, current_value)
						};

						formatting.push_indentation(string, indent);

						string.push_str("out_mesh.set_primitive(");
						self.emit_node_string(string, &current_index);
						string.push_str(", PrimitiveOutput{.instance_index = ");
						self.emit_node_string(string, &instance_value);
						string.push_str(", .primitive_index = ");
						self.emit_node_string(string, &primitive_value);
						string.push_str("})");
						formatting.push_statement_end(string);
						i += 2;
						continue;
					}
				}
			}

			emit_statement_block(string, formatting, &statements[i..i + 1], indent, |string, statement| {
				self.emit_node_string(string, statement)
			});
			i += 1;
		}
	}

	/// Translates BESL intrinsic type names to MSL type names.
	/// Example: `vec2f` -> `float2`
	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"bool" => "bool",
			"atomicu32" => "atomic_uint",
			"vec2f" => "float2",
			"vec2u" => "uint2",
			"vec2i" => "int2",
			"vec2u16" => "ushort2",
			"vec4u16" => "ushort4",
			"vec3u" => "uint3",
			"vec4u" => "uint4",
			"vec3f" => "float3",
			"vec4f" => "float4",
			"mat2f" => "float2x2",
			"mat3f" => "float3x3",
			"mat4f" => "float4x4",
			"mat4x3f" => "float4x3",
			"f32" => "float",
			"u8" => "uchar",
			"u16" => "ushort",
			"u32" => "uint",
			"i32" => "int",
			"Texture2D" => "texture2d<float>",
			"Texture3D" => "texture3d<float>",
			"ArrayTexture2D" => "texture2d_array<float>",
			_ => source,
		}
	}

	fn emit_visibility_texture_sample(&mut self, string: &mut String, slot: &besl::NodeReference, xy_only: bool) {
		string.push_str("resources.textures[material.textures[");
		self.emit_visibility_texture_slot(string, slot);
		string.push_str("]].sample(resources.textures_sampler[material.textures[");
		self.emit_visibility_texture_slot(string, slot);
		string.push_str("]], vertex_uv, level(0.0))");
		if xy_only {
			string.push_str(".xy");
		}
	}

	fn emit_visibility_texture_slot(&mut self, string: &mut String, slot: &besl::NodeReference) {
		let slot_borrow = slot.borrow();
		match slot_borrow.node() {
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => {
				let source = source.clone();
				drop(slot_borrow);
				if self.try_emit_visibility_texture_const_value(string, &source) {
					return;
				}
				self.emit_node_string(string, slot);
			}
			_ => {
				drop(slot_borrow);
				if self.try_emit_visibility_texture_const_value(string, slot) {
					return;
				}
				self.emit_node_string(string, slot);
			}
		}
	}

	fn try_emit_visibility_texture_const_value(&mut self, string: &mut String, slot: &besl::NodeReference) -> bool {
		let slot_borrow = slot.borrow();
		let besl::Nodes::Const { value, .. } = slot_borrow.node() else {
			return false;
		};
		let value = value.clone();
		drop(slot_borrow);
		self.emit_node_string(string, &value);
		true
	}

	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeReference,
		arguments: &[besl::NodeReference],
		elements: &[besl::NodeReference],
	) {
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic {
			name,
			elements: definition,
			..
		} = intrinsic.node()
		else {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		};

		match name.as_str() {
			"sample_material" => {
				self.emit_visibility_texture_sample(string, &arguments[0], false);
				return;
			}
			"sample_normal" => {
				string.push_str("unit_vector_from_xy(");
				self.emit_visibility_texture_sample(string, &arguments[0], true);
				string.push(')');
				return;
			}
			"texture_lod" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".sample(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", level(0.0))");
				return;
			}
			_ => {}
		}

		let has_body = definition
			.iter()
			.any(|element| !matches!(element.borrow().node(), besl::Nodes::Parameter { .. }));
		if has_body {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		}

		match name.as_str() {
			"min" | "max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "round" | "fract"
			| "fwidth" | "step" | "smoothstep" | "mix" => {
				string.push_str(name);
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"radians" => {
				string.push('(');
				self.emit_node_string(string, &arguments[0]);
				if self.minified {
					string.push_str("*(PI/180.0))");
				} else {
					string.push_str(" * (PI / 180.0))");
				}
			}
			"inversesqrt" => {
				string.push_str("rsqrt(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"f32" => {
				string.push_str("float(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"u32" => {
				string.push_str("uint(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"atomic_add" => {
				string.push_str("atomic_fetch_add_explicit(&");
				self.emit_node_string(string, &arguments[0]);
				self.emit_separator(string);
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", memory_order_relaxed)");
			}
			"atomic_load" => {
				string.push_str("atomic_load_explicit(&");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(", memory_order_relaxed)");
			}
			"atomic_store" => {
				string.push_str("atomic_store_explicit(&");
				self.emit_node_string(string, &arguments[0]);
				self.emit_separator(string);
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", memory_order_relaxed)");
			}
			"thread_position" => {
				string.push_str("thread_position");
			}
			"thread_id" => {
				string.push_str("gid");
			}
			"thread_idx" => {
				string.push_str("thread_index");
			}
			"threadgroup_position" => {
				string.push_str("threadgroup_position");
			}
			"workgroup_barrier" => {
				string.push_str("threadgroup_barrier(mem_flags::mem_threadgroup)");
			}
			"set_task_mesh_output_count" => {
				string.push_str("mesh_grid.set_threadgroups_per_grid(uint3(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(", 1, 1))");
			}
			"set_mesh_output_counts" => {
				string.push_str("if(thread_index==0){out_mesh.set_primitive_count(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(");}");
			}
			"set_mesh_vertex_position" => {
				string.push_str("out_mesh.set_vertex(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(", VertexOutput{.position = ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str("})");
			}
			"set_mesh_triangle" => {
				string.push_str("out_mesh.set_index(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" * 3 + 0, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".x);out_mesh.set_index(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" * 3 + 1, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".y);out_mesh.set_index(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(" * 3 + 2, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".z)");
			}
			"image_load" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"image_load_u32" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(").x");
			}
			"fetch" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"texture_size" | "image_size" => {
				string.push_str("uint2(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_width(),");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_height())");
			}
			"write" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".write(");
				self.emit_node_string(string, &arguments[2]);
				if self.minified {
					string.push(',');
				} else {
					string.push_str(", ");
				}
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"guard_image_bounds" => {
				string.push_str("if(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".x>=");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_width()||");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(".y>=");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_height()){return;}");
			}
			_ => {
				string.push_str(name);
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
		}
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	fn emit_node_string(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(this_node);
		let formatting = ShaderFormatting::new(self.minified);

		let break_char = formatting.break_str();

		match node.node() {
			besl::Nodes::Null => {}
			besl::Nodes::Scope { .. } => {}
			besl::Nodes::Function {
				name,
				statements,
				return_type,
				params,
				..
			} => self.emit_function_node(string, this_node, name, statements, return_type, params),
			besl::Nodes::Struct {
				name, fields, template, ..
			} => self.emit_struct_node(string, name, fields, template),
			besl::Nodes::PushConstant { members } => {
				self.emit_named_struct_start(string, "PushConstant");

				for member in members {
					formatting.push_indentation(string, 1);
					self.emit_node_string(string, member);
					formatting.push_statement_end(string);
				}

				self.emit_struct_declaration_end(string);

				// TODO: Confirm push constant mapping for Metal argument buffers.
				if self.minified {
					string.push_str(&format!(
						"constant PushConstant& push_constant [[buffer({})]];",
						PUSH_CONSTANT_BINDING_INDEX
					));
				} else {
					string.push_str(&format!(
						"constant PushConstant& push_constant [[buffer({})]];\n",
						PUSH_CONSTANT_BINDING_INDEX
					));
				}
			}
			besl::Nodes::TaskPayload { .. } | besl::Nodes::Workgroup { .. } => {}
			besl::Nodes::Specialization { name, r#type } => {
				let mut members = Vec::new();

				let r#type = r#type.borrow();

				let t = r#type.get_name().unwrap();
				let type_name = Self::translate_type(t);

				if let besl::Nodes::Struct { fields, .. } = r#type.node() {
					for (i, field) in fields.iter().enumerate() {
						if let besl::Nodes::Member {
							name: member_name,
							r#type,
							..
						} = field.borrow().node()
						{
							let member_name = format!("{}_{}", name, { member_name });
							string.push_str(&format!(
								"constant {} {} [[function_constant({})]];{}",
								Self::translate_type(r#type.borrow().get_name().unwrap()),
								member_name,
								i,
								if !self.minified { "\n" } else { "" }
							));
							members.push(member_name);
						}
					}
				}

				string.push_str(&format!(
					"constant {} {}={};{}",
					type_name,
					name,
					format!("{}({})", &type_name, members.join(",")),
					if !self.minified { "\n" } else { "" }
				));
			}
			besl::Nodes::Member { name, r#type, count } => {
				if let Some(type_name) = r#type.borrow().get_name() {
					if self.in_buffer_binding_struct && (count.is_some() || matches!(type_name, "vec2u16" | "vec4u16")) {
						string.push_str(Self::translate_buffer_member_type(type_name));
					} else if type_name.contains('[') {
						Self::emit_type_name(string, type_name);
					} else {
						string.push_str(Self::translate_type(type_name));
					}
					string.push(' ');
				}
				string.push_str(name.as_str());
				if let Some(count) = count {
					string.push('[');
					string.push_str(count.to_string().as_str());
					string.push(']');
				}
			}
			besl::Nodes::Raw { glsl, hlsl, msl, .. } => {
				if let Some(code) = msl.as_ref().or(hlsl.as_ref()).or(glsl.as_ref()) {
					string.push_str(code);
				}
			}
			besl::Nodes::Parameter { name, r#type } => self.emit_parameter_node(string, name, r#type),
			besl::Nodes::Input { name, location, format } => {
				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
				// TODO: Map interpolation qualifiers to Metal (flat/linear).
				string.push_str(&format!("{} {} [[attribute({})]];{break_char}", type_name, name, location));
			}
			besl::Nodes::Output {
				name,
				location,
				format,
				count,
			} => {
				if count.is_some() {
					return;
				}

				let format = format.borrow();
				let type_name = Self::translate_type(format.get_name().unwrap());
				string.push_str(&format!("{} {} [[color({})]];{break_char}", type_name, name, location));
			}
			besl::Nodes::Expression(expression) => self.emit_expression_node(string, expression),
			besl::Nodes::Conditional { condition, statements } => self.emit_conditional_node(string, condition, statements),
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => self.emit_for_loop_node(string, initializer, condition, update, statements),
			besl::Nodes::Binding {
				name,
				slot,
				read,
				write,
				r#type,
				count,
				..
			} => {
				if self.in_compute_body || self.mesh_stage_context.is_some() {
					self.emit_compute_binding_reference(string, name);
					return;
				}

				let index = *slot;

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						self.emit_named_struct_start(string, &format!("_{name}"));

						for member in members.iter() {
							self.emit_indentation(string, 1);
							self.emit_node_string(string, member);
							self.emit_statement_end(string);
						}

						self.emit_struct_declaration_end(string);

						let address_space = if *write { "device" } else { "constant" };

						string.push_str(address_space);
						string.push(' ');
						string.push_str(&format!("_{}* {}", name, name));

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[buffer({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::Image { format } => {
						let element_type = match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "uint",
							_ => "float",
						};

						let access = if *read && *write {
							"access::read_write"
						} else if *write {
							"access::write"
						} else {
							"access::read"
						};

						string.push_str(&format!("texture2d<{}, {}> {}", element_type, access, name));

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[texture({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
					besl::BindingTypes::CombinedImageSampler { format } => {
						let texture_type = match format.as_str() {
							"ArrayTexture2D" => "texture2d_array<float>",
							"r8ui" | "r16ui" | "r32ui" => "texture2d<uint>",
							_ => "texture2d<float>",
						};

						string.push_str(texture_type);
						string.push(' ');
						string.push_str(name);

						if let Some(count) = count {
							string.push('[');
							string.push_str(count.to_string().as_str());
							string.push(']');
						}

						string.push_str(&format!(" [[texture({})]];", index));
						if !self.minified {
							string.push('\n');
						}

						string.push_str("sampler ");
						string.push_str(&format!("{}_sampler", name));
						string.push_str(&format!(" [[sampler({})]];", index));
						if !self.minified {
							string.push('\n');
						}
					}
				}
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node_string(string, element);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				self.emit_node_string(string, value);
			}
			besl::Nodes::Const { name, r#type, value } => {
				string.push_str("constant ");
				let type_name = r#type.borrow().get_name().unwrap().to_string();
				if let Some((element_type, count)) = type_name.split_once('[') {
					string.push_str(Self::translate_type(element_type));
					string.push(' ');
					string.push_str(name);
					string.push('[');
					string.push_str(count.trim_end_matches(']'));
					string.push(']');
				} else {
					Self::emit_type_name(string, &type_name);
					string.push(' ');
					string.push_str(name);
				}
				string.push_str(" = ");
				if let besl::Nodes::Expression(besl::Expressions::FunctionCall {
					parameters, function, ..
				}) = value.borrow().node()
				{
					if function.borrow().get_name() == Some(type_name.as_str()) {
						string.push('{');
						self.emit_call_arguments(string, parameters);
						string.push('}');
					} else {
						self.emit_node_string(string, value);
					}
				} else {
					self.emit_node_string(string, value);
				}
				string.push_str(&format!(";{break_char}"));
			}
		}
	}

	fn generate_msl_header_block(&self, msl_block: &mut String, compilation_settings: &ShaderGenerationSettings) {
		msl_block.push_str("#include <metal_stdlib>\n");
		msl_block.push_str("using namespace metal;\n");

		match compilation_settings.stage {
			Stages::Vertex => msl_block.push_str("// #pragma shader_stage(vertex)\n"),
			Stages::Fragment => msl_block.push_str("// #pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => msl_block.push_str("// #pragma shader_stage(compute)\n"),
			Stages::Task { .. } => msl_block.push_str("// #pragma shader_stage(object)\n"),
			Stages::Mesh { .. } => msl_block.push_str("// #pragma shader_stage(mesh)\n"),
		}

		match compilation_settings.stage {
			Stages::Compute { local_size } => {
				msl_block.push_str(&format!(
					"// besl-threadgroup-size:{},{},{}\n",
					local_size.width().max(1),
					local_size.height().max(1),
					local_size.depth().max(1)
				));
				msl_block.push_str("// Note: Metal threadgroup sizes are set on the pipeline state.\n");
			}
			Stages::Task { local_size, .. } | Stages::Mesh { local_size, .. } => {
				msl_block.push_str(&format!(
					"// besl-threadgroup-size:{},{},{}\n",
					local_size.width().max(1),
					local_size.height().max(1),
					local_size.depth().max(1)
				));
			}
			_ => {}
		}

		match compilation_settings.matrix_layout {
			MatrixLayouts::RowMajor => msl_block.push_str("// Matrix layout: row major\n"),
			MatrixLayouts::ColumnMajor => msl_block.push_str("// Matrix layout: column major\n"),
		}

		msl_block.push_str("constant float PI = 3.14159265359;");

		if !self.minified {
			msl_block.push('\n');
		}
	}
}

impl<A: Allocator + Clone> crate::shader::generator::NodeEmitter for Generator<A> {
	fn type_from_besl(source: &str) -> &str {
		Generator::<A>::translate_type(source)
	}
	fn minified(&self) -> bool {
		self.minified
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
		} else if self.in_compute_body && self.function_requires_resource_context(node, true) {
			self.emit_compute_hidden_parameters(string, has_previous_parameter);
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
			} else if self.in_compute_body && self.function_requires_resource_context(function, true) {
				self.emit_compute_hidden_call_arguments(string, has_previous_argument);
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
			name, template: None, ..
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
		self.emit_call_arguments(string, parameters);
		string.push('}');
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
	fn emit_node(&mut self, string: &mut String, node: &besl::NodeReference) {
		self.emit_node_string(string, node)
	}
}
#[cfg(test)]
mod tests {
	use std::cell::RefCell;

	use super::*;
	use crate::shader::generator::{self, ShaderGenerationSettings};

	macro_rules! assert_string_contains {
		($haystack:expr, $needle:expr) => {
			assert!(
				$haystack.contains($needle),
				"Expected string to contain '{}', but it did not. String: '{}'",
				$needle,
				$haystack
			);
		};
	}

	fn sampled_binding(name: &str, slot: u32, read: bool, write: bool) -> besl::NodeReference {
		besl::Node::binding(
			name,
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			slot,
			read,
			write,
		)
		.into()
	}

	fn main_with(statements: Vec<besl::NodeReference>) -> besl::NodeReference {
		let root = besl::Node::root();
		let void = root.get_child("void").expect("Expected the built-in void type");
		besl::Node::function("main", Vec::new(), void, statements).into()
	}

	#[test]
	fn intrinsic_definition_only_bindings_do_not_shift_dense_argument_ids() {
		let root = besl::Node::root();
		let void = root.get_child("void").expect("Expected the built-in void type");
		let intrinsic: besl::NodeReference = besl::Node::intrinsic(
			"instantiated_binding_fixture",
			vec![sampled_binding("definition_only", 0, true, false)],
			void.clone(),
		)
		.into();
		let call = besl::Node::expression(besl::Expressions::IntrinsicCall {
			intrinsic,
			arguments: Vec::new(),
			elements: vec![sampled_binding("instantiated", 100, true, false)],
		})
		.into();
		let main: besl::NodeReference = besl::Node::function("main", Vec::new(), void, vec![call]).into();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected instantiated intrinsic binding generation");

		assert_string_contains!(shader, "texture2d<float> instantiated [[id(0)]];");
		assert_string_contains!(shader, "sampler instantiated_sampler [[id(1)]];");
		assert!(!shader.contains("definition_only"));
	}

	#[test]
	fn distinct_reachable_declarations_cannot_reuse_a_flat_slot() {
		let main = main_with(vec![
			sampled_binding("first", 4, true, false),
			sampled_binding("second", 4, false, true),
		]);

		assert!(
			Generator::new()
				.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
				.is_err(),
			"Distinct declarations at one flat slot must be rejected before MSL emission"
		);
	}

	#[test]
	fn distinct_reachable_declaration_ranges_cannot_overlap() {
		let array: besl::NodeReference = besl::Node::binding_array(
			"array",
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			4,
			true,
			false,
			2,
		)
		.into();
		let main = main_with(vec![array, sampled_binding("interior", 5, true, false)]);

		assert!(
			Generator::new()
				.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
				.is_err(),
			"Intersecting flat slot intervals must be rejected before MSL emission"
		);
	}

	#[test]
	fn dense_metal_argument_id_ranges_cannot_overflow() {
		let binding: besl::NodeReference = besl::Node::binding_array(
			"textures",
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			0,
			true,
			false,
			u32::MAX as usize,
		)
		.into();
		let main = main_with(vec![binding]);

		assert!(
			Generator::new()
				.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
				.is_err(),
			"Packed Metal argument IDs must not wrap"
		);
	}

	#[test]
	fn bindings() {
		let main = generator::tests::bindings();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct _buff{float member;};");
		assert_string_contains!(shader, "device _buff* buff [[buffer(0)]];");
		assert_string_contains!(shader, "texture2d<float, access::write> image [[texture(1)]];");
		assert_string_contains!(shader, "texture2d<float> texture [[texture(2)]];");
		assert_string_contains!(shader, "sampler texture_sampler [[sampler(2)]];");
		assert_string_contains!(shader, "void main(){buff;image;texture;}");
	}

	#[test]
	fn vec4u16_uses_the_native_msl_packed_storage_vector_type() {
		let main = generator::tests::vec4u16_binding();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected vec4u16 MSL generation");

		assert_string_contains!(shader, "struct _buff{packed_ushort4 value;};");
		assert!(!shader.contains("struct vec4u16"));
	}

	#[test]
	fn packed_u16_storage_vectors_preserve_tight_array_and_mixed_struct_layouts() {
		let vec2_array = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::vec2u16_array_binding(),
			)
			.expect("Expected vec2u16 MSL generation");
		let mixed_vec4 = Generator::new()
			.minified(true)
			.generate(
				&ShaderGenerationSettings::compute(utils::Extent::line(1)),
				&generator::tests::mixed_vec4u16_binding(),
			)
			.expect("Expected mixed vec4u16 MSL generation");

		assert_string_contains!(vec2_array, "struct _buff{packed_ushort2 values[2];};");
		assert_string_contains!(mixed_vec4, "struct _buff{packed_ushort4 value;ushort tail;};");
	}

	#[test]
	fn generator_accepts_custom_allocator() {
		let main = generator::tests::bindings();

		let shader = Generator::new_in(std::alloc::System)
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader with custom allocator");

		assert_string_contains!(shader, "struct _buff{float member;};");
	}

	#[test]
	fn generate_accepts_call_scoped_allocator() {
		let main = generator::tests::bindings();
		let mut generator = Generator::new_in(std::alloc::System).minified(true);

		let shader = generator
			.generate_in(&ShaderGenerationSettings::vertex(), &main, std::alloc::System)
			.expect("Failed to generate shader with call-scoped allocator");

		assert_string_contains!(shader, "struct _buff{float member;};");
	}

	#[test]
	fn compute_bindings_use_argument_buffers_by_default() {
		let main = generator::tests::bindings();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct _resources{device _buff* buff [[id(0)]];texture2d<float, access::write> image [[id(1)]];texture2d<float> texture [[id(2)]];sampler texture_sampler [[id(3)]];};"
		);
		assert_string_contains!(
			shader,
			"kernel void besl_main(uint2 gid [[thread_position_in_grid]],constant _resources& resources [[buffer(16)]])"
		);
		assert_string_contains!(shader, "resources.buff;resources.image;resources.texture;");
	}

	#[test]
	fn compute_bindings_can_use_bare_resources() {
		let main = generator::tests::bindings();

		let shader = Generator::new()
			.minified(true)
			.compute_binding_mode(ComputeBindingMode::BareResources)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "kernel void besl_main(uint2 gid [[thread_position_in_grid]],");
		assert_string_contains!(shader, "device _buff* buff [[buffer(0)]]");
		assert_string_contains!(shader, "texture2d<float, access::write> image [[texture(1)]]");
		assert_string_contains!(shader, "texture2d<float> texture [[texture(2)]]");
		assert_string_contains!(shader, "sampler texture_sampler [[sampler(2)]]");
		assert_string_contains!(shader, "buff;image;texture;");
	}

	#[test]
	fn same_named_buffer_members_lower_to_msl() {
		let main = generator::tests::same_named_buffer_member_access();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "resources.pixel_mapping->pixel_mapping[0]");
		assert_string_contains!(shader, "resources.meshes->meshes[1]");
	}

	#[test]
	fn buffer_vector_arrays_use_packed_msl_types() {
		let script = r#"
		main: fn () -> void {
			let position: vec3f = positions.values[0];
			let uv: vec2f = uvs.values[0];
			position;
			uv;
		}
		"#;

		let mut root = besl::parse(script).expect("Expected packed buffer array test shader source to parse");
		root.add(vec![
			besl::parser::Node::binding(
				"positions",
				besl::parser::Node::buffer("Positions", vec![besl::parser::Node::member("values", "vec3f[8]")]),
				0,
				true,
				false,
			),
			besl::parser::Node::binding(
				"uvs",
				besl::parser::Node::buffer("Uvs", vec![besl::parser::Node::member("values", "vec2f[8]")]),
				1,
				true,
				false,
			),
		]);
		let root = besl::lex(root).expect(
			"Expected packed buffer array test shader source to lex. The most likely cause is invalid BESL syntax in the test shader.",
		);
		let main = root.get_main().expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct _positions{packed_float3 values[8];};");
		assert_string_contains!(shader, "struct _uvs{packed_float2 values[8];};");
	}

	#[test]
	fn non_buffer_vector_arrays_keep_standard_msl_types() {
		let script = r#"
		VertexBlock: struct {
			positions: vec3f[4],
		}

		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(script, None).expect(
			"Expected non-buffer vector array test shader source to compile. The most likely cause is invalid BESL syntax in the test shader.",
		);
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");
		let vertex_block = RefCell::borrow(&root)
			.get_child("VertexBlock")
			.expect("Expected VertexBlock struct");

		{
			let mut main = main.borrow_mut();
			main.add_child(
				besl::Node::raw(
					Some("VertexBlock;".to_string()),
					Some("VertexBlock;".to_string()),
					Some("VertexBlock;".to_string()),
					vec![vertex_block],
					vec![],
				)
				.into(),
			);
		}

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct VertexBlock{float3 positions[4];};");
		assert!(
			!shader.contains("packed_float3 positions[4]"),
			"Expected non-buffer vector arrays to keep standard MSL vector types"
		);
	}

	#[test]
	fn intrinsics_lower_to_valid_msl_names() {
		let source = r#"
		main: fn () -> void {
			let angle: f32 = radians(180.0);
			let inverse: f32 = inversesqrt(4.0);
			angle;
			inverse;
		}
		"#;

		let root = besl::compile_to_besl(source, None).expect(
			"Expected intrinsic test shader source to compile. The most likely cause is invalid BESL syntax in the test shader.",
		);
		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float angle=(180.0*(PI/180.0));");
		assert_string_contains!(shader, "rsqrt(4.0)");
	}

	#[test]
	fn user_struct_constructors_lower_to_aggregate_initialization() {
		let mut root = besl::Node::root();
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		root.add_child(
			besl::Node::r#struct(
				"Pair",
				vec![
					besl::Node::member("left", vec4f.clone()).into(),
					besl::Node::member("right", vec4f).into(),
				],
			)
			.into(),
		);
		let root = besl::compile_to_besl(
			"main: fn () -> void { let pair: Pair = Pair(vec4f(1.0, 1.0, 1.0, 1.0), vec4f(2.0, 2.0, 2.0, 2.0)); pair; }",
			Some(root),
		)
		.expect("Expected user struct constructor shader to compile");
		let main = root.get_main().expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "Pair pair=Pair{float4(1.0,1.0,1.0,1.0),float4(2.0,2.0,2.0,2.0)};");
	}

	const TASK_PAYLOAD_FIXTURE_SOURCE: &str = r#"
		Meshlets: struct {
			values: u32[32],
		}
		meshlets: descriptor<Meshlets, 8, read>;
		visible_meshlets: task_payload<u32, 32>;
		visible_count: workgroup<atomicu32>;
		push_constant: push_constant {
			base_meshlet: u32,
		}

		dispatch_visible_meshlets: fn () -> void {
			let position: u32 = thread_position();
			let lane: u32 = thread_idx();
			if (lane == 0) {
				atomic_store(visible_count, 0);
			}
			workgroup_barrier();
			if (position < 32) {
				let payload_index: u32 = atomic_add(visible_count, 1);
				visible_meshlets[payload_index] = meshlets.values[push_constant.base_meshlet + position];
			}
			workgroup_barrier();
			if (lane == 0) {
				set_task_mesh_output_count(atomic_load(visible_count));
			}
		}

		main: fn () -> void {
			dispatch_visible_meshlets();
		}
	"#;

	const MESH_PAYLOAD_FIXTURE_SOURCE: &str = r#"
		visible_meshlets: task_payload<u32, 32>;
		out_instance_index: output<u32, 0, 126>;
		out_primitive_index: output<u32, 1, 126>;

		main: fn () -> void {
			let lane: u32 = thread_idx();
			let meshlet_index: u32 = visible_meshlets[threadgroup_position()];
			set_mesh_output_counts(3, 1);
			if (lane < 3) {
				set_mesh_vertex_position(lane, vec4f(f32(lane), 0.0, 0.0, 1.0));
			}
			if (lane < 1) {
				set_mesh_triangle(0, vec3u(0, 1, 2));
				out_instance_index[0] = meshlet_index;
				out_primitive_index[0] = meshlet_index;
			}
		}
	"#;

	fn lower_fixture(source: &str, settings: &ShaderGenerationSettings) -> String {
		let root = besl::compile_to_besl(source, None).expect("Expected stage fixture source to link");
		let main = root.get_main().expect("Expected stage fixture main function");
		Generator::new()
			.minified(true)
			.generate(settings, &main)
			.expect("Expected stage fixture to lower to MSL")
	}

	#[test]
	fn task_stage_lowers_workgroup_storage_payload_and_mesh_dispatch() {
		let shader = lower_fixture(
			TASK_PAYLOAD_FIXTURE_SOURCE,
			&ShaderGenerationSettings::task(utils::Extent::line(32), 32),
		);

		assert_string_contains!(shader, "// #pragma shader_stage(object)");
		assert_string_contains!(shader, "// besl-threadgroup-size:32,1,1");
		assert_string_contains!(shader, "struct ObjectPayload{uint visible_meshlets[32];};");
		assert_string_contains!(shader, "[[object, max_total_threadgroups_per_mesh_grid(32)]] void besl_main(");
		assert_string_contains!(shader, "uint thread_position [[thread_position_in_grid]]");
		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(shader, "object_data ObjectPayload& payload [[payload]]");
		assert_string_contains!(shader, "mesh_grid_properties mesh_grid");
		assert_string_contains!(shader, "threadgroup atomic_uint visible_count;");
		assert_string_contains!(shader, "threadgroup_barrier(mem_flags::mem_threadgroup)");
		assert_string_contains!(shader, "payload.visible_meshlets[payload_index]");
		assert_string_contains!(shader, "mesh_grid.set_threadgroups_per_grid(uint3(");
	}

	#[test]
	fn mesh_stage_consumes_the_same_authored_task_payload() {
		let shader = lower_fixture(
			MESH_PAYLOAD_FIXTURE_SOURCE,
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
		);

		assert_string_contains!(shader, "struct ObjectPayload{uint visible_meshlets[32];};");
		assert_string_contains!(shader, "const object_data ObjectPayload& payload [[payload]]");
		assert_string_contains!(shader, "uint meshlet_index=payload.visible_meshlets[threadgroup_position];");
		assert_string_contains!(shader, "out_mesh.set_vertex(");
		assert_string_contains!(shader, "out_mesh.set_index(");
		assert_string_contains!(shader, "out_mesh.set_primitive(");
	}

	#[test]
	fn matrix_and_vector_index_access_uses_msl_subscripts() {
		let shader = lower_fixture(
			r#"
			main: fn() -> void {
				let matrix: mat4f = mat4f(
					vec4f(1.0, 0.0, 0.0, 0.0),
					vec4f(0.0, 1.0, 0.0, 0.0),
					vec4f(0.0, 0.0, 1.0, 0.0),
					vec4f(0.0, 0.0, 0.0, 1.0)
				);
				let column: vec4f = matrix[0];
				let element: f32 = column[1];
				element;
			}
			"#,
			&ShaderGenerationSettings::vertex(),
		);

		assert_string_contains!(shader, "matrix[0]");
		assert_string_contains!(shader, "column[1]");
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn generated_task_and_mesh_payload_stages_compile_with_metal() {
		let task = lower_fixture(
			TASK_PAYLOAD_FIXTURE_SOURCE,
			&ShaderGenerationSettings::task(utils::Extent::line(32), 32),
		);
		let mesh = lower_fixture(
			MESH_PAYLOAD_FIXTURE_SOURCE,
			&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)),
		);

		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&task, "besl-task-payload-fixture")
			.expect("Expected generated task MSL to compile natively");
		crate::shader::msl_shader_compiler::compile_msl_source_to_metallib(&mesh, "besl-mesh-payload-fixture")
			.expect("Expected generated mesh MSL to compile natively");
	}

	#[test]
	fn mesh_stage_uses_mesh_entry_point_and_mesh_push_constants() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let main = besl::parser::Node::function(
			"main",
			Vec::new(),
			"void",
			vec![besl::parser::Node::raw_code(
				Some("".into()),
				Some("push_constant;threadgroup_position;thread_index;out_mesh;".into()),
				Some("push_constant;threadgroup_position;thread_index;out_mesh;".into()),
				&["push_constant", "VertexOutput", "PrimitiveOutput"],
				&[],
			)],
		);
		let shader = besl::parser::Node::scope("Shader", vec![push_constant, mesh_output_types, main]);
		let mut root = besl::parser::Node::root();
		root.add(vec![shader]);

		let root_node = besl::lex(root).unwrap();
		let main_node = root_node.get_main().unwrap();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main_node)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "// besl-threadgroup-size:128,1,1");
		assert_string_contains!(shader, "[[mesh]] void besl_main(");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]]");
		assert_string_contains!(shader, "uint threadgroup_position [[threadgroup_position_in_grid]]");
		assert_string_contains!(shader, "uint thread_index [[thread_index_in_threadgroup]]");
		assert_string_contains!(
			shader,
			"metal::mesh<VertexOutput, PrimitiveOutput, 64, 126, topology::triangle> out_mesh"
		);
	}

	#[test]
	fn compute_shaders_emit_threadgroup_metadata() {
		let source = "main: fn () -> void { let coord: vec3u = thread_id(); }";
		let root = besl::parse(source).unwrap();
		let root = besl::lex(root).unwrap();
		let main_node = root.get_main().unwrap();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(128)), &main_node)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "// besl-threadgroup-size:128,1,1");
	}

	#[test]
	fn specializtions() {
		let main = generator::tests::specializations();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "constant float color_x [[function_constant(0)]];");
		assert_string_contains!(shader, "constant float color_y [[function_constant(1)]];");
		assert_string_contains!(shader, "constant float color_z [[function_constant(2)]];");
		assert_string_contains!(shader, "constant float3 color=float3(color_x,color_y,color_z);");
		assert_string_contains!(shader, "void main(){color;}");
	}

	#[test]
	fn input() {
		let main = generator::tests::input();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct VertexInput{float3 color [[attribute(0)]];};");
		assert_string_contains!(shader, "vertex VertexOutput besl_main(VertexInput in [[stage_in]])");
		assert_string_contains!(shader, "float3 color=in.color;");
		assert_string_contains!(shader, "color;return out;");
	}

	#[test]
	fn output() {
		let main = generator::tests::output();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct VertexOutput{float4 position [[position]];float3 color [[user(locn0)]];};"
		);
		assert_string_contains!(shader, "vertex VertexOutput besl_main(VertexInput in [[stage_in]])");
		assert_string_contains!(shader, "float3 color;color;out.color=color;return out;");
	}

	#[test]
	fn vertex_builtin_stage_inputs_lower_to_msl_semantics() {
		let mut root = besl::Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		root.add_child(besl::Node::input("vertex_id", u32_type.clone(), 0).into());
		root.add_child(besl::Node::input("instance_id", u32_type, 1).into());

		let root = besl::compile_to_besl("main: fn () -> void { vertex_id; instance_id; }", Some(root)).unwrap();
		let main = root.borrow().get_child("main").unwrap();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct VertexInput{};");
		assert_string_contains!(shader, "uint vertex_id [[vertex_id]],uint instance_id [[instance_id]]");
		assert!(!shader.contains("uint vertex_id=vertex_id;"));
		assert!(!shader.contains("uint instance_id=instance_id;"));
	}

	#[test]
	fn fragment_explicit_output_struct_return_lowers_to_msl_entry_return() {
		let script = r#"
		FragmentOutput: struct {
			color: vec4f,
		}

		main: fn () -> FragmentOutput {
			return FragmentOutput(vec4f(1.0, 0.0, 0.0, 1.0));
		}
		"#;
		let root = besl::compile_to_besl(script, None).expect("Expected explicit fragment output shader to lex");
		let main = root.borrow().get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct FragmentInput{};");
		assert_string_contains!(shader, "struct FragmentOutput{float4 color;};");
		assert_string_contains!(shader, "fragment FragmentOutput besl_main(FragmentInput in [[stage_in]])");
		assert_string_contains!(shader, "return FragmentOutput{float4(1.0,0.0,0.0,1.0)};");
	}

	#[test]
	fn fwidth_intrinsic_lowers_to_msl() {
		let program = besl::compile_to_besl("main: fn() -> void { let edge_width: f32 = fwidth(1.0); edge_width; }", None)
			.expect("Failed to compile fwidth BESL shader");
		let main = program.get_main().expect("Expected fwidth BESL shader main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate fwidth MSL shader");

		assert_string_contains!(shader, "fwidth(1.0)");
	}

	#[test]
	fn fragment_builtin_stage_io_lowers_to_msl_semantics() {
		let mut root = besl::Node::root();
		let bool_type = root.get_child("bool").expect("Expected bool type");
		let f32_type = root.get_child("f32").expect("Expected f32 type");
		root.add_child(besl::Node::input("front_facing", bool_type, 0).into());
		let u32_type = root.get_child("u32").expect("Expected u32 type");
		root.add_child(besl::Node::output("depth", f32_type, 0).into());
		root.add_child(besl::Node::output("stencil", u32_type.clone(), 1).into());
		root.add_child(besl::Node::output("sample_mask", u32_type, 2).into());

		let root = besl::compile_to_besl(
			"main: fn () -> void { front_facing; depth; stencil; sample_mask; }",
			Some(root),
		)
		.unwrap();
		let main = root.borrow().get_child("main").unwrap();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct FragmentInput{};");
		assert_string_contains!(shader, "float depth [[depth(any)]];");
		assert_string_contains!(shader, "uint stencil [[stencil]];");
		assert_string_contains!(shader, "uint sample_mask [[sample_mask]];");
		assert_string_contains!(shader, "bool front_facing [[front_facing]]");
		assert!(!shader.contains("bool front_facing=front_facing;"));
	}

	#[test]
	fn fragment_shader() {
		let main = generator::tests::fragment_shader();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::fragment(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){float3 albedo=float3(1.0,0.0,0.0);}");
	}

	#[test]
	fn raster_full_source_passthrough_uses_raw_msl_source() {
		let source = "// besl-full-source\n#include <metal_stdlib>\nvertex void besl_main() {}";
		let mut root = besl::parser::Node::root();
		let main = besl::parser::Node::main_function(vec![besl::parser::Node::raw_code(
			Some("".into()),
			None,
			Some(source.into()),
			&[],
			&[],
		)]);
		root.add(vec![besl::parser::Node::scope("Shader", vec![main])]);

		let main = besl::lex(root).unwrap().get_main().unwrap();
		let shader = Generator::new()
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_eq!(shader, "#include <metal_stdlib>\nvertex void besl_main() {}");
	}

	#[test]
	fn vertex_shader_generates_msl_entry_point() {
		let mut root = besl::parser::Node::root();
		let camera = besl::parser::Node::r#struct("Camera", vec![besl::parser::Node::member("view_projection", "mat4f")]);
		let cameras = besl::parser::Node::binding(
			"cameras",
			besl::parser::Node::buffer("CamerasBuffer", vec![besl::parser::Node::member("cameras", "Camera[8]")]),
			0,
			true,
			false,
		);
		let main = besl::parser::Node::main_function(vec![besl::parser::Node::raw_code(
			Some("".into()),
			None,
			Some(
				"position = resources.cameras->cameras[0].view_projection * float4(in_position, 1.0); out_instance_index = 0u;"
					.into(),
			),
			&["cameras", "in_position", "out_instance_index"],
			&[],
		)]);
		root.add(vec![besl::parser::Node::scope(
			"Shader",
			vec![
				camera,
				cameras,
				besl::parser::Node::input("in_position", "vec3f", 0),
				besl::parser::Node::output("out_instance_index", "u32", 0),
				main,
			],
		)]);

		let main = besl::lex(root).unwrap().get_main().unwrap();
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct _cameras{Camera cameras[8];};");
		assert_string_contains!(shader, "struct _resources{constant _cameras* cameras [[id(0)]];};");
		assert_string_contains!(shader, "struct VertexInput{float3 in_position [[attribute(0)]];};");
		assert_string_contains!(
			shader,
			"struct VertexOutput{float4 position [[position]];uint out_instance_index [[flat]] [[user(locn0)]];};"
		);
		assert_string_contains!(
			shader,
			"vertex VertexOutput besl_main(VertexInput in [[stage_in]],constant _resources& resources [[buffer(16)]])"
		);
		assert_string_contains!(shader, "position = resources.cameras->cameras[0].view_projection");
		assert_string_contains!(shader, "return out;");
	}

	/// Verifies raster helpers retain binding access when lowered outside the Metal entry point.
	#[test]
	fn raster_helpers_receive_argument_buffer_context() {
		let mut root = besl::Node::root();
		let mat4f = root.get_child("mat4f").expect("Expected mat4f type");
		let vec3f = root.get_child("vec3f").expect("Expected vec3f type");
		let vec4f = root.get_child("vec4f").expect("Expected vec4f type");
		let camera =
			root.add_child(besl::Node::r#struct("Camera", vec![besl::Node::member("view_projection", mat4f).into()]).into());
		root.add_children(vec![
			besl::Node::binding(
				"cameras",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("cameras", camera, 1)],
				},
				0,
				true,
				false,
			)
			.into(),
			besl::Node::input("in_position", vec3f, 0).into(),
			besl::Node::output("position", vec4f, 0).into(),
		]);

		let program = besl::compile_to_besl(
			r#"
			camera_matrix: fn () -> mat4f {
				return cameras.cameras[0].view_projection;
			}
			main: fn () -> void {
				position = camera_matrix() * vec4f(in_position.x, in_position.y, in_position.z, 1.0);
			}
			"#,
			Some(root),
		)
		.expect("Failed to compile the raster helper fixture. The most likely cause is invalid BESL syntax.");
		let main = program.get_main().expect("Expected raster helper fixture main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate raster helper MSL. The most likely cause is missing raster resource context.");

		assert_string_contains!(shader, "float4x4 camera_matrix(constant _resources& resources);");
		assert_string_contains!(
			shader,
			"float4x4 camera_matrix(constant _resources& resources){return resources.cameras->cameras[0].view_projection;}"
		);
		assert_string_contains!(
			shader,
			"position=(camera_matrix(resources)*float4(in_position.x,in_position.y,in_position.z,1.0));"
		);
	}

	#[test]
	fn fetch_intrinsic_lowers_to_msl() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 2);
			let texel: vec4f = fetch(texture, coord);
		}
		"#;

		let mut root = besl::Node::root();
		root.add_child(
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				true,
				false,
			)
			.into(),
		);

		let root = besl::compile_to_besl(script, Some(root)).expect("Expected fetch shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "float4 texel=resources.texture.read(coord);");
	}

	#[test]
	fn cull_unused_functions() {
		let main = generator::tests::cull_unused_functions();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"void used_by_used(){}void used(){used_by_used();}void main(){used();}"
		);
	}

	#[test]
	fn structure() {
		let main = generator::tests::structure();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"struct Vertex{float3 position;float3 normal;};Vertex use_vertex(){}void main(){use_vertex();}"
		);
	}

	#[test]
	fn push_constant() {
		let main = generator::tests::push_constant();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct PushConstant{uint material_id;};");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]];");
		assert_string_contains!(shader, "void main(){push_constant;}");
	}

	#[test]
	fn test_msl() {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		used: fn() -> void {}

		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let vertex_struct = RefCell::borrow(&root).get_child("Vertex").unwrap();
		let used_function = RefCell::borrow(&root).get_child("used").unwrap();

		{
			let mut main = main.borrow_mut();
			main.add_child(
				besl::Node::hlsl(
					"output.position = float4(0, 0, 0, 1)".to_string(),
					vec![vertex_struct, used_function],
					vec![],
				)
				.into(),
			);
		}

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void used(){}");
		assert_string_contains!(shader, "void main(){output.position = float4(0, 0, 0, 1);}");
	}

	#[test]
	fn test_instrinsic() {
		let main = generator::tests::intrinsic();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void main(){0 + 1.0 * 2;}");
	}

	#[test]
	fn matrix_multiplication_preserves_operand_order_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, model: mat4f, position: vec4f) -> vec4f {
			return projection * model * position;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected matrix multiply shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4x4 model,float4 position){return (projection*model)*position;}"
		);
	}

	#[test]
	fn matrix_on_both_sides_preserves_operand_order_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, model: mat4f) -> mat4f {
			return projection * model;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected matrix-matrix shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4x4 main(float4x4 projection,float4x4 model){return projection*model;}"
		);
	}

	#[test]
	fn matrix_and_vector_multiplication_preserves_operand_order_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, position: vec4f) -> vec4f {
			return projection * position;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected matrix-vector shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4 position){return projection*position;}"
		);
	}

	#[test]
	fn chained_matrix_vector_scalar_multiplication_preserves_operand_order_for_msl() {
		let script = r#"
		main: fn (projection: mat4f, position: vec4f, scale: f32) -> vec4f {
			return projection * position * scale;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected chained multiply shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"float4 main(float4x4 projection,float4 position,float scale){return (projection*position)*scale;}"
		);
	}

	#[test]
	fn test_multi_language_raw_code() {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		main: fn () -> void {}
		"#;

		let root = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		let vertex_struct = RefCell::borrow(&root).get_child("Vertex").unwrap();

		{
			let mut main = main.borrow_mut();
			// Create a RawCode node with both GLSL and HLSL variants
			main.add_child(
				besl::Node::raw(
					Some("gl_Position = vec4(0)".to_string()),
					Some("output.position = float4(0, 0, 0, 1)".to_string()),
					Some("out.position = float4(0, 0, 0, 1)".to_string()),
					vec![vertex_struct],
					vec![],
				)
				.into(),
			);
		}

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		// MSL generator should use the explicit MSL code
		assert_string_contains!(shader, "struct Vertex{float3 position;float3 normal;};");
		assert_string_contains!(shader, "void main(){out.position = float4(0, 0, 0, 1);}");
		// Should NOT contain GLSL code
		assert!(!shader.contains("gl_Position"), "MSL shader should not contain GLSL code");
	}

	#[test]
	fn test_const_variable() {
		let main = generator::tests::const_variable();

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "constant float PI = 3.14;");
		assert_string_contains!(shader, "void main(){PI;}");
	}

	#[test]
	fn mesh_intrinsics_emit_msl_mesh_commands() {
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let script = r#"
		main: fn () -> void {
			set_mesh_output_counts(4, 2);
			set_mesh_vertex_position(0, vec4f(1.0, 2.0, 3.0, 1.0));
			set_mesh_triangle(0, vec3u(0, 1, 2));
		}
		"#;

		let mut root = besl::parse(script).expect("Expected mesh shader source to parse");
		root.add(vec![mesh_output_types]);
		let root = besl::lex(root).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "if(thread_index==0){out_mesh.set_primitive_count(2);}");
		assert_string_contains!(
			shader,
			"out_mesh.set_vertex(0, VertexOutput{.position = float4(1.0,2.0,3.0,1.0)})"
		);
		assert_string_contains!(
			shader,
			"out_mesh.set_index(0 * 3 + 0, uint3(0,1,2).x);out_mesh.set_index(0 * 3 + 1, uint3(0,1,2).y);out_mesh.set_index(0 * 3 + 2, uint3(0,1,2).z)"
		);
	}

	#[test]
	fn mesh_output_assignments_lower_to_msl_primitive_outputs() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let out_instance_index = besl::parser::Node::output_array("out_instance_index", "u32", 0, 126);
		let out_primitive_index = besl::parser::Node::output_array("out_primitive_index", "u32", 1, 126);
		let script = r#"
		main: fn () -> void {
			out_instance_index[0] = 7;
			out_primitive_index[0] = 9;
		}
		"#;

		let mut root = besl::parse(script).expect("Expected mesh shader source to parse");
		root.add(vec![
			push_constant,
			mesh_output_types,
			out_instance_index,
			out_primitive_index,
		]);
		let root = besl::lex(root).expect("Expected mesh shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(
			shader,
			"out_mesh.set_primitive(0, PrimitiveOutput{.instance_index = 7, .primitive_index = 9});"
		);
	}

	#[test]
	fn mesh_stage_user_functions_do_not_receive_hidden_context_parameters() {
		let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
		let meshlets = besl::parser::Node::binding(
			"meshlets",
			besl::parser::Node::buffer("MeshletBuffer", vec![besl::parser::Node::member("count", "u32")]),
			0,
			true,
			false,
		);
		let mesh_output_types = besl::parser::Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint primitive_index [[flat]] [[user(locn0)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let mut parsed_shader = besl::parse(
			r#"
			helper: fn () -> void {
				meshlets.count;
				threadgroup_position();
				thread_idx();
				set_mesh_output_counts(3, 1);
			}

			main: fn () -> void {
				helper();
			}
			"#,
		)
		.expect("Expected mesh helper shader to parse");
		let parsed_children = match parsed_shader.node_mut() {
			besl::parser::Nodes::Scope { children, .. } => std::mem::take(children),
			_ => panic!(
				"Expected mesh helper shader to parse into a scope. The most likely cause is invalid BESL syntax in the mesh helper shader test."
			),
		};
		let mut shader = besl::parser::Node::root();
		shader.add(vec![meshlets, push_constant, mesh_output_types]);
		shader.add(parsed_children);
		let root = besl::lex(shader).expect("Expected mesh helper shader to lex");
		let main = root.get_main().expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::mesh(64, 126, utils::Extent::line(128)), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "void helper()");
		assert_string_contains!(shader, "helper();");
		assert!(!shader.contains("void helper(constant _resources& resources"));
		assert!(!shader.contains("helper(resources,threadgroup_position,thread_index,out_mesh);"));
	}

	#[test]
	fn conditional_blocks_lower_to_msl() {
		let script = r#"
		main: fn () -> void {
			let n: u32 = 0;
			if (n < 1) {
				n = 2;
			}
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected conditional shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "if(n<1){n=2;}");
	}

	#[test]
	fn bitwise_operators_lower_to_msl() {
		let script = r#"
		main: fn () -> void {
			let packed: u32 = 1 << 8 | 2 & 255;
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected bitwise shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "uint packed=((1<<8)|(2&255));");
	}

	#[test]
	fn comparison_and_continue_lower_to_msl() {
		let script = r#"
		main: fn () -> void {
			for (let i: u32 = 0; i <= 4; i = i + 1) {
				if (i >= 2) {
					continue;
				}
			}
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "for(uint i=0;i<=4;i=(i+1)){if(i>=2){continue;};};");
	}

	#[test]
	fn scalar_max_and_clamp_lower_to_msl() {
		let script = r#"
		main: fn () -> void {
			let maximum: f32 = max(1.0, 2.0);
			let clamped: f32 = clamp(1.5, 0.0, 1.0);
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "max(1.0,2.0)");
		assert_string_contains!(shader, "clamp(1.5,0.0,1.0)");
	}

	#[test]
	fn const_array_variable_lowers_to_msl() {
		let script = r#"
		WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);

		main: fn () -> void {
			let value: f32 = WEIGHTS[1];
		}
		"#;

		let root = besl::compile_to_besl(script, None).expect("Expected const-array shader source to lex");
		let main = RefCell::borrow(&root).get_child("main").expect("Expected main function");

		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(shader, "constant float WEIGHTS[3] = {0.5,0.25,0.125};");
		assert_string_contains!(shader, "float value=WEIGHTS[1];");
	}

	#[test]
	fn source_declared_atomic_images_and_push_constants_lower_to_msl() {
		let source = r#"
			Counters: struct {
				values: atomicu32[8],
			}
			counters: descriptor<Counters, 2, read_write>;
			index_image: descriptor<StorageImage<r32ui>, 4, read>;
			push_constant: push_constant {
				base: u32,
			}
			main: fn () -> void {
				let coord: vec2u = thread_id();
				let index: u32 = image_load_u32(index_image, coord) + push_constant.base;
				let old: u32 = atomic_add(counters.values[index], 1);
				atomic_store(counters.values[index], atomic_load(counters.values[old]));
			}
		"#;

		let root = besl::compile_to_besl(source, None).expect("Expected standalone atomic source to lex");
		let main = root.get_main().expect("Expected standalone atomic source main function");
		let shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::compute(utils::Extent::line(1)), &main)
			.expect("Expected standalone atomic source to lower to MSL");

		assert_string_contains!(shader, "atomic_uint values[8]");
		assert_string_contains!(shader, "texture2d<uint, access::read> index_image");
		assert_string_contains!(shader, "constant PushConstant& push_constant [[buffer(15)]]");
		assert_string_contains!(shader, ".read(coord).x");
		assert_string_contains!(shader, "atomic_fetch_add_explicit(&");
		assert_string_contains!(shader, "atomic_load_explicit(&");
		assert_string_contains!(shader, "atomic_store_explicit(&");
	}

	#[test]
	fn return_values_and_pretty_spacing_lower_to_msl() {
		let main = generator::tests::return_value();

		let minified_shader = Generator::new()
			.minified(true)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(minified_shader, "float main(){return 1.0;}");

		let pretty_shader = Generator::new()
			.minified(false)
			.generate(&ShaderGenerationSettings::vertex(), &main)
			.expect("Failed to generate shader");

		assert_string_contains!(pretty_shader, "float main() {\n\treturn 1.0;\n}\n");
	}
}

use std::{
	alloc::{Allocator, Global},
	cell::RefCell,
	fmt::Write as _,
	vec::Vec,
};

pub use Generator as MSLShaderGenerator;

use crate::shader::generator::{
	emit_comma_separated_nodes, emit_statement_block, ordered_shader_nodes_in, MatrixLayouts, NodeEmitter, ShaderFormatting,
	ShaderGenerationSettings, ShaderGenerator, Stages,
};
