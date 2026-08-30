use super::*;
impl<A: Allocator + Clone> Generator<A> {
	pub(crate) fn emit_declarations(&mut self, string: &mut String, nodes: &[&besl::NodeReference]) {
		for node in nodes {
			self.emit_node_string(string, node);
		}
	}

	pub(crate) fn emit_buffer_binding_structs(&mut self, string: &mut String, bindings: &[&besl::NodeReference]) {
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

	pub(crate) fn generate_vertex_shader(
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
			has_vertex_index: nodes.inputs.iter().any(
				|input| matches!(input.borrow().node(), besl::Nodes::Input { name, .. } if name == besl::VERTEX_INDEX_BUILTIN),
			),
			has_instance_index: nodes.inputs.iter().any(
				|input| matches!(input.borrow().node(), besl::Nodes::Input { name, .. } if name == besl::INSTANCE_INDEX_BUILTIN),
			),
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

	pub(crate) fn emit_vertex_input_struct(&mut self, string: &mut String, inputs: &[&besl::NodeReference]) {
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

	pub(crate) fn emit_fragment_input_struct(&mut self, string: &mut String, inputs: &[&besl::NodeReference]) {
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

	pub(crate) fn emit_fragment_output_struct(&mut self, string: &mut String, outputs: &[&besl::NodeReference]) {
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

	pub(crate) fn emit_vertex_output_struct(&mut self, string: &mut String, outputs: &[&besl::NodeReference]) {
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

	pub(crate) fn generate_fragment_shader(
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
			has_vertex_index: false,
			has_instance_index: false,
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

	pub(crate) fn emit_raster_input_locals(
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

	pub(crate) fn emit_raster_output_locals(&mut self, string: &mut String, outputs: &[&besl::NodeReference], indent: usize) {
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

	pub(crate) fn emit_raster_output_assignments(
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

	pub(crate) fn emit_vertex_entry_point(
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

		string.push_str("vertex VertexOutput ");
		string.push_str(MSL_ENTRY_POINT);
		string.push_str("(VertexInput in [[stage_in]]");
		if inputs
			.iter()
			.any(|input| matches!(input.borrow().node(), besl::Nodes::Input { name, .. } if name == besl::VERTEX_INDEX_BUILTIN))
		{
			self.emit_separator(string);
			string.push_str("uint vertex_index [[vertex_id]]");
		}
		if inputs.iter().any(
			|input| matches!(input.borrow().node(), besl::Nodes::Input { name, .. } if name == besl::INSTANCE_INDEX_BUILTIN),
		) {
			self.emit_separator(string);
			string.push_str("uint instance_index [[instance_id]]");
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
			&[
				(besl::VERTEX_INDEX_BUILTIN, besl::VERTEX_INDEX_BUILTIN),
				(besl::INSTANCE_INDEX_BUILTIN, besl::INSTANCE_INDEX_BUILTIN),
			],
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

	pub(crate) fn emit_fragment_entry_point(
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
		let has_outputs = !outputs.is_empty();
		let entry_return_type = if returns_explicit_output {
			Self::translate_type(&return_type_name).to_string()
		} else if has_outputs {
			"FragmentOutput".to_string()
		} else {
			"void".to_string()
		};

		string.push_str("fragment ");
		string.push_str(&entry_return_type);
		string.push(' ');
		string.push_str(MSL_ENTRY_POINT);
		string.push_str("(FragmentInput in [[stage_in]]");
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
		} else if has_outputs {
			formatting.push_indentation(string, 1);
			string.push_str("FragmentOutput out");
			formatting.push_statement_end(string);
			self.emit_raster_output_locals(string, outputs, 1);

			self.emit_statement_block(string, statements, 1);

			self.emit_raster_output_assignments(string, outputs, "out", 1);
			formatting.push_indentation(string, 1);
			string.push_str("return out");
			formatting.push_statement_end(string);
		} else {
			self.emit_statement_block(string, statements, 1);
		}

		self.emit_block_end(string);
	}

	pub(crate) fn is_vertex_builtin_input(name: &str) -> bool {
		crate::shader::generator::is_vertex_builtin_input(name)
	}

	pub(crate) fn is_fragment_builtin_input(name: &str) -> bool {
		matches!(name, "front_facing")
	}

	pub(crate) fn is_integer_type(name: &str) -> bool {
		matches!(
			name,
			"u8" | "u16" | "u32" | "i32" | "vec2u" | "vec2u16" | "vec4u16" | "vec2i" | "vec3u" | "vec4u"
		)
	}

	pub(crate) fn generate_compute_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
		uses_simd_lane_id: bool,
	) {
		let nodes = self.classify_nodes(order);
		self.emit_declarations(string, &nodes.declarations);
		self.emit_declarations(string, &nodes.inputs);
		self.emit_declarations(string, &nodes.outputs);

		if let Some(push_constant) = nodes.push_constant {
			self.emit_push_constant_struct(string, push_constant);
		}

		let bindings = self.sort_bindings_by_slot(nodes.bindings.as_slice());
		let workgroups = nodes
			.workgroups
			.iter()
			.filter_map(|workgroup| {
				let workgroup = workgroup.borrow();
				let besl::Nodes::Workgroup { name, format, count } = workgroup.node() else {
					return None;
				};
				let msl_type = Self::translate_type(format.borrow().get_name().unwrap()).to_string();
				Some(StageWorkgroup {
					name: name.clone(),
					msl_type,
					count: count.map(|count| count.get()),
				})
			})
			.collect();
		let previous_compute_stage_context = self.compute_stage_context.replace(ComputeStageContext {
			has_resources: !bindings.is_empty(),
			has_push_constant: nodes.push_constant.is_some(),
			workgroups,
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
					&nodes.workgroups,
					uses_simd_lane_id,
				);
			}
			ComputeBindingMode::BareResources => {
				self.emit_compute_entry_point_bare_resources(
					string,
					main_function_node,
					nodes.bindings.as_slice(),
					nodes.push_constant,
					&nodes.workgroups,
					uses_simd_lane_id,
				);
			}
		}

		self.in_compute_body = previous_in_compute_body;
		self.compute_stage_context = previous_compute_stage_context;
	}

	pub(crate) fn generate_task_shader(
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
				let besl::Nodes::Workgroup { name, format, count } = workgroup.node() else {
					return None;
				};
				let msl_type = Self::translate_type(format.borrow().get_name().unwrap()).to_string();
				Some(StageWorkgroup {
					name: name.clone(),
					msl_type,
					count: count.map(|count| count.get()),
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

	pub(crate) fn generate_mesh_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main_function_node: &besl::NodeReference,
		maximum_vertices: u32,
		maximum_primitives: u32,
		uses_render_target_array_index: bool,
	) {
		let nodes = self.classify_nodes(order);
		if let Some(push_constant) = nodes.push_constant {
			self.emit_push_constant_struct(string, push_constant);
		}

		let bindings = self.sort_bindings_by_slot(nodes.bindings.as_slice());
		let primitive_output_fields = nodes
			.outputs
			.iter()
			.filter_map(|output| {
				let output = output.borrow();
				let besl::Nodes::Output {
					name, count: Some(_), ..
				} = output.node()
				else {
					return None;
				};
				Some(Self::mesh_output_field_name(&name).to_string())
			})
			.collect();
		let previous_mesh_stage_context = self.mesh_stage_context.replace(MeshStageContext {
			has_resources: !bindings.is_empty(),
			has_push_constant: nodes.push_constant.is_some(),
			has_task_payload: !nodes.task_payloads.is_empty(),
			uses_render_target_array_index,
			primitive_output_fields,
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

	pub(crate) fn has_raw_mesh_output_structs(nodes: &[&besl::NodeReference]) -> bool {
		nodes.iter().any(|node| match node.borrow().node() {
			besl::Nodes::Raw { msl, hlsl, .. } => msl
				.as_ref()
				.or(hlsl.as_ref())
				.is_some_and(|source| source.contains("struct VertexOutput") || source.contains("struct PrimitiveOutput")),
			_ => false,
		})
	}

	pub(crate) fn mesh_output_field_name(name: &str) -> &str {
		name.strip_prefix("out_").unwrap_or(name)
	}

	pub(crate) fn emit_mesh_output_structs(&mut self, string: &mut String, outputs: &[&besl::NodeReference]) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "VertexOutput");
		formatting.push_indentation(string, 1);
		string.push_str("float4 position [[position]]");
		formatting.push_statement_end(string);
		self.emit_struct_declaration_end(string);

		self.emit_named_struct_start(string, "PrimitiveOutput");
		if self
			.mesh_stage_context
			.as_ref()
			.is_some_and(|context| context.uses_render_target_array_index)
		{
			formatting.push_indentation(string, 1);
			string.push_str("uint render_target_array_index [[render_target_array_index]]");
			formatting.push_statement_end(string);
		}
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
			string.push_str(Self::mesh_output_field_name(&name));
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

	/// Returns resources in logical-slot order so generated MSL remains deterministic.
	pub(crate) fn sort_bindings_by_slot<'a>(&self, bindings: &[&'a besl::NodeReference]) -> Vec<&'a besl::NodeReference, A> {
		let mut sorted = Vec::with_capacity_in(bindings.len(), self.allocator.clone());
		sorted.extend_from_slice(bindings);
		sorted.sort_unstable_by_key(|binding| match binding.borrow().node() {
			besl::Nodes::Binding { slot, .. } => *slot,
			_ => u32::MAX,
		});
		sorted
	}
}
