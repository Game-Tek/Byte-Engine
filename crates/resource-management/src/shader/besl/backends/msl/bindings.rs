use super::*;
impl<A: Allocator + Clone> Generator<A> {
	pub(crate) fn emit_push_constant_struct(&mut self, string: &mut String, push_constant: &besl::NodeReference) {
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

	pub(crate) fn emit_object_payload_struct(&mut self, string: &mut String, payloads: &[&besl::NodeReference]) {
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

	/// Maps one logical flat-slot interval to a stable Metal argument-ID reservation.
	pub(crate) fn fixed_argument_ids(slot: u32, count: u32) -> Result<(u32, u32), ()> {
		let primary = slot.checked_mul(2).ok_or(())?;
		let secondary = primary.checked_add(count).ok_or(())?;
		secondary.checked_add(count).ok_or(())?;
		Ok((primary, secondary))
	}

	pub(crate) fn emit_argument_buffer_struct(&mut self, string: &mut String, bindings: &[&besl::NodeReference]) {
		self.emit_named_struct_start(string, "_resources");

		for binding in bindings {
			self.emit_argument_buffer_field(string, binding);
		}

		self.emit_struct_declaration_end(string);
	}

	/// Emits one field using IDs derived only from its logical flat-slot interval.
	pub(crate) fn emit_argument_buffer_field(&mut self, string: &mut String, binding_node: &besl::NodeReference) {
		let node = binding_node.borrow();
		let besl::Nodes::Binding {
			name,
			read,
			write,
			memory_class,
			r#type,
			count,
			slot,
			..
		} = node.node()
		else {
			return;
		};

		let descriptor_count = count.map(|count| count.get()).unwrap_or(1);
		let (primary_id, secondary_id) = Self::fixed_argument_ids(*slot, descriptor_count).expect(
			"Invalid fixed Metal argument ID range. The most likely cause is that binding validation was bypassed before source emission.",
		);
		let emit_suffix = |string: &mut String, argument_id: u32| {
			string.push_str(" [[id(");
			let _ = write!(string, "{argument_id}");
			string.push_str(")]]");
			if let Some(count) = count {
				string.push('[');
				let _ = write!(string, "{count}");
				string.push(']');
			}
			self.emit_statement_end(string);
		};

		self.emit_indentation(string, 1);

		match r#type {
			besl::BindingTypes::Buffer { .. } => {
				let address_space = buffer_address_space(*memory_class, *write);
				string.push_str(address_space);
				string.push(' ');
				string.push_str(&format!("_{}* {}", name, name));
				emit_suffix(string, primary_id);
			}
			besl::BindingTypes::BufferArray { element } => {
				let address_space = buffer_address_space(*memory_class, *write);
				string.push_str(address_space);
				string.push(' ');
				string.push_str(Self::translate_type(element.borrow().get_name().unwrap()));
				string.push_str("* ");
				string.push_str(name);
				emit_suffix(string, primary_id);
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
				emit_suffix(string, primary_id);
			}
			besl::BindingTypes::CombinedImageSampler { format } => {
				let texture_type = match format.as_str() {
					"Texture3D" => "texture3d<float>",
					"TextureCube" => "texturecube<float>",
					"TextureCubeArray" => "texturecube_array<float>",
					"ArrayTexture2D" => "texture2d_array<float>",
					"r8ui" | "r16ui" | "r32ui" => "texture2d<uint>",
					_ => "texture2d<float>",
				};
				string.push_str(texture_type);
				string.push(' ');
				string.push_str(name);
				emit_suffix(string, primary_id);

				self.emit_indentation(string, 1);
				string.push_str("sampler ");
				string.push_str(&format!("{}_sampler", name));
				emit_suffix(string, secondary_id);
			}
		}
	}

	pub(crate) fn emit_buffer_binding_struct(
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

	pub(crate) fn translate_buffer_member_type(source: &str) -> &str {
		// Metal storage buffers need packed vectors when the CPU data is tightly packed.
		// Float vectors retain the existing array-only policy, while 16-bit vectors stay packed inside mixed structs.
		match source {
			"vec2f16" => "packed_half2",
			"vec3f16" => "packed_half3",
			"vec4f16" => "packed_half4",
			"vec2f" => "packed_float2",
			"vec3f" => "packed_float3",
			"vec3u" => "packed_uint3",
			"mat4x3f" => "_besl_packed_float4x3",
			"vec2u16" => "packed_ushort2",
			"vec4u16" => "packed_ushort4",
			_ => Self::translate_type(source),
		}
	}

	pub(crate) fn emit_compute_entry_point_bare_resources(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		bindings: &[&besl::NodeReference],
		push_constant: Option<&besl::NodeReference>,
		workgroups: &[&besl::NodeReference],
		uses_simd_lane_id: bool,
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
		if uses_simd_lane_id {
			self.emit_separator(string);
			string.push_str("uint simd_lane_id [[thread_index_in_simdgroup]]");
		}
		if !workgroups.is_empty() {
			self.emit_separator(string);
			string.push_str("uint thread_index [[thread_index_in_threadgroup]]");
		}

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

		self.emit_compute_workgroup_declarations(string, workgroups);
		self.emit_statement_block(string, statements, 1);

		self.emit_block_end(string);
	}

	pub(crate) fn emit_compute_entry_point_argument_buffers(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
		has_resources: bool,
		push_constant: Option<&besl::NodeReference>,
		workgroups: &[&besl::NodeReference],
		uses_simd_lane_id: bool,
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
			string.push_str(MSL_ENTRY_POINT);
		} else {
			string.push_str(name);
		}
		string.push('(');
		string.push_str("uint2 gid [[thread_position_in_grid]]");
		if uses_simd_lane_id {
			self.emit_separator(string);
			string.push_str("uint simd_lane_id [[thread_index_in_simdgroup]]");
		}
		if !workgroups.is_empty() {
			self.emit_separator(string);
			string.push_str("uint thread_index [[thread_index_in_threadgroup]]");
		}

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

		self.emit_compute_workgroup_declarations(string, workgroups);
		self.emit_statement_block(string, statements, 1);

		self.emit_block_end(string);
	}

	/// Emits function-scope threadgroup variables shared by every invocation in one compute workgroup.
	pub(crate) fn emit_compute_workgroup_declarations(&mut self, string: &mut String, workgroups: &[&besl::NodeReference]) {
		for workgroup in workgroups {
			let workgroup = workgroup.borrow();
			let besl::Nodes::Workgroup { name, format, count } = workgroup.node() else {
				continue;
			};
			self.emit_indentation(string, 1);
			string.push_str("threadgroup ");
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			if let Some(count) = count {
				string.push('[');
				string.push_str(&count.to_string());
				string.push(']');
			}
			self.emit_statement_end(string);
		}
	}

	pub(crate) fn emit_task_entry_point(
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
			string.push_str(MSL_ENTRY_POINT);
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
			let besl::Nodes::Workgroup { name, format, count } = workgroup.node() else {
				continue;
			};
			self.emit_indentation(string, 1);
			string.push_str("threadgroup ");
			string.push_str(Self::translate_type(format.borrow().get_name().unwrap()));
			string.push(' ');
			string.push_str(name);
			if let Some(count) = count {
				string.push('[');
				string.push_str(&count.to_string());
				string.push(']');
			}
			self.emit_statement_end(string);
		}
		self.emit_statement_block(string, statements, 1);
		self.emit_block_end(string);
	}

	pub(crate) fn emit_mesh_entry_point_argument_buffers(
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
			string.push_str(MSL_ENTRY_POINT);
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

	pub(crate) fn emit_mesh_push_constant_parameter(&self, string: &mut String, _push_constant: &besl::NodeReference) {
		string.push_str(&format!(
			"constant PushConstant& push_constant [[buffer({})]]",
			PUSH_CONSTANT_BINDING_INDEX
		));
	}

	pub(crate) fn emit_compute_push_constant_parameter(&self, string: &mut String, _push_constant: &besl::NodeReference) {
		string.push_str(&format!(
			"constant PushConstant& push_constant [[buffer({})]]",
			PUSH_CONSTANT_BINDING_INDEX
		));
	}

	pub(crate) fn emit_compute_binding_parameter(&self, string: &mut String, binding_node: &besl::NodeReference) {
		let node = binding_node.borrow();
		let besl::Nodes::Binding {
			name,
			slot,
			read,
			write,
			memory_class,
			r#type,
			..
		} = node.node()
		else {
			return;
		};

		let index = *slot;

		match r#type {
			besl::BindingTypes::Buffer { .. } => {
				let address_space = buffer_address_space(*memory_class, *write);
				self.emit_separator(string);
				string.push_str(address_space);
				string.push(' ');
				string.push_str(&format!("_{}* {} [[buffer({})]]", name, name, index));
			}
			besl::BindingTypes::BufferArray { element } => {
				let address_space = buffer_address_space(*memory_class, *write);
				self.emit_separator(string);
				string.push_str(address_space);
				string.push(' ');
				string.push_str(Self::translate_type(element.borrow().get_name().unwrap()));
				string.push_str("* ");
				string.push_str(name);
				string.push_str(&format!(" [[buffer({index})]]"));
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
					"TextureCube" => "texturecube<float>",
					"TextureCubeArray" => "texturecube_array<float>",
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

	pub(crate) fn emit_compute_binding_reference(&self, string: &mut String, name: &str) {
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
	pub(crate) fn emit_raster_binding_reference(&self, string: &mut String, name: &str) {
		string.push_str("resources.");
		string.push_str(name);
	}

	pub(crate) fn emit_task_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
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
			if workgroup.count.is_some() {
				string.push_str("* ");
			} else {
				string.push_str("& ");
			}
			string.push_str(&workgroup.name);
			has_previous_parameter = true;
		}
		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("thread mesh_grid_properties& mesh_grid");
	}

	pub(crate) fn emit_task_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
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

	pub(crate) fn emit_mesh_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
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

	pub(crate) fn emit_mesh_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
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

	pub(crate) fn emit_compute_hidden_parameters(
		&self,
		string: &mut String,
		has_previous_parameter: bool,
		uses_simd_lane_id: bool,
	) {
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

		if uses_simd_lane_id {
			self.emit_separator(string);
			string.push_str("uint simd_lane_id");
		}

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
			has_previous_parameter = true;
		}

		if !compute_stage_context.workgroups.is_empty() {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("uint thread_index");
			has_previous_parameter = true;
		}

		for workgroup in &compute_stage_context.workgroups {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("threadgroup ");
			string.push_str(&workgroup.msl_type);
			if workgroup.count.is_some() {
				string.push_str("* ");
			} else {
				string.push_str("& ");
			}
			string.push_str(&workgroup.name);
			has_previous_parameter = true;
		}
	}

	/// Adds argument-buffer parameters to raster helpers that access BESL bindings.
	pub(crate) fn emit_raster_hidden_parameters(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(raster_stage_context) = &self.raster_stage_context else {
			return;
		};

		let mut has_previous_parameter = has_previous_parameter;
		if raster_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("constant _resources& resources");
			has_previous_parameter = true;
		}
		for (used, name) in [
			(raster_stage_context.has_vertex_index, besl::VERTEX_INDEX_BUILTIN),
			(raster_stage_context.has_instance_index, besl::INSTANCE_INDEX_BUILTIN),
		] {
			if !used {
				continue;
			}
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("uint ");
			string.push_str(name);
			has_previous_parameter = true;
		}
	}

	pub(crate) fn emit_compute_hidden_call_arguments(
		&self,
		string: &mut String,
		has_previous_parameter: bool,
		uses_simd_lane_id: bool,
	) {
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

		if uses_simd_lane_id {
			self.emit_separator(string);
			string.push_str("simd_lane_id");
		}

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
			has_previous_parameter = true;
		}

		if !compute_stage_context.workgroups.is_empty() {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("thread_index");
			has_previous_parameter = true;
		}

		for workgroup in &compute_stage_context.workgroups {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str(&workgroup.name);
			has_previous_parameter = true;
		}
	}

	/// Forwards entry-point argument buffers to binding-dependent raster helpers.
	pub(crate) fn emit_raster_hidden_call_arguments(&self, string: &mut String, has_previous_parameter: bool) {
		let Some(raster_stage_context) = &self.raster_stage_context else {
			return;
		};

		let mut has_previous_parameter = has_previous_parameter;
		if raster_stage_context.has_resources {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("resources");
			has_previous_parameter = true;
		}
		for (used, name) in [
			(raster_stage_context.has_vertex_index, besl::VERTEX_INDEX_BUILTIN),
			(raster_stage_context.has_instance_index, besl::INSTANCE_INDEX_BUILTIN),
		] {
			if !used {
				continue;
			}
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str(name);
			has_previous_parameter = true;
		}
	}
}
