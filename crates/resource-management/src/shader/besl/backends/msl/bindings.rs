use super::*;

/// Reports whether Metal stores a member declared directly in a buffer binding as a packed type.
///
/// The MSL emitter and storage-layout reflection both call it, so reflected buffer strides always match the emitted
/// Metal struct layout. Array members, and 16-bit vectors in mixed structs, are packed; other members keep their
/// natural Metal alignment.
pub(crate) fn msl_packs_direct_binding_member(type_name: &str, is_array: bool) -> bool {
	is_array || matches!(type_name, "vec2f16" | "vec3f16" | "vec4f16" | "vec2u16" | "vec4u16")
}

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
			Self::type_identifier(format.borrow().get_name().unwrap()).push_to(string);
			let _ = write!(string, " {}[{count}]", Self::identifier(name));
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
				let _ = write!(string, "_{name}* {}", Self::identifier(name));
				emit_suffix(string, primary_id);
			}
			besl::BindingTypes::BufferArray { element, .. } => {
				let address_space = buffer_address_space(*memory_class, *write);
				string.push_str(address_space);
				string.push(' ');
				Self::emit_buffer_member_type(string, element.borrow().get_name().unwrap());
				string.push_str("* ");
				Self::identifier(name).push_to(string);
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
				let _ = write!(string, "texture2d<{element_type}, {access}> {}", Self::identifier(name));
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
				Self::identifier(name).push_to(string);
				emit_suffix(string, primary_id);

				self.emit_indentation(string, 1);
				let _ = write!(string, "sampler {}_sampler", Self::identifier(name));
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

		self.emit_named_struct_start(string, format_args!("_{name}"));

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

	/// Emits the storage-buffer spelling of a member or array-buffer element type. User struct names keep their
	/// backend-safe name.
	pub(crate) fn emit_buffer_member_type(string: &mut String, source: &str) {
		// Metal storage buffers need packed vectors when the CPU data is tightly packed. Array buffers use this for
		// their elements, such as 12-byte `vec3f` and 48-byte `mat4x3f`.
		// Float vectors retain the existing array-only policy, while 16-bit vectors stay packed inside mixed structs.
		let packed = match source {
			"vec2f16" => "packed_half2",
			"vec3f16" => "packed_half3",
			"vec4f16" => "packed_half4",
			"vec2f" => "packed_float2",
			"vec3f" => "packed_float3",
			"vec3u" => "packed_uint3",
			"mat4x3f" => "_besl_packed_float4x3",
			"vec2u16" => "packed_ushort2",
			"vec4u16" => "packed_ushort4",
			_ => return Self::type_identifier(source).push_to(string),
		};
		string.push_str(packed);
	}

	pub(crate) fn emit_compute_entry_point(
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
			string.push_str(MSL_ENTRY_POINT);
		} else {
			Self::identifier(name).push_to(string);
		}
		string.push('(');
		string.push_str("uint2 gid [[thread_position_in_grid]]");
		self.emit_separator(string);
		string.push_str("uint thread_index [[thread_index_in_threadgroup]]");
		self.emit_separator(string);
		string.push_str("uint2 threadgroup_position [[threadgroup_position_in_grid]]");
		if uses_simd_lane_id {
			self.emit_separator(string);
			string.push_str("uint simd_lane_id [[thread_index_in_simdgroup]]");
		}

		for param in params {
			self.emit_separator(string);
			self.emit_node_string(string, param);
		}

		if push_constant.is_some() {
			self.emit_separator(string);
			self.emit_push_constant_parameter(string);
		}

		match self.compute_binding_mode {
			ComputeBindingMode::ArgumentBuffers if !bindings.is_empty() => {
				self.emit_separator(string);
				self.emit_argument_buffer_parameter(string);
			}
			ComputeBindingMode::BareResources => {
				for binding in bindings {
					self.emit_compute_binding_parameter(string, binding);
				}
			}
			ComputeBindingMode::ArgumentBuffers => {}
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
			Self::type_identifier(format.borrow().get_name().unwrap()).push_to(string);
			string.push(' ');
			Self::identifier(name).push_to(string);
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
			Self::identifier(name).push_to(string);
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

		if push_constant.is_some() {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			self.emit_push_constant_parameter(string);
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
			Self::type_identifier(format.borrow().get_name().unwrap()).push_to(string);
			string.push(' ');
			Self::identifier(name).push_to(string);
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
			Self::identifier(name).push_to(string);
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

		if push_constant.is_some() {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			self.emit_push_constant_parameter(string);
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

	/// Writes the entry-point parameter that binds the push-constant block at its fixed Metal buffer slot.
	pub(crate) fn emit_push_constant_parameter(&self, string: &mut String) {
		let _ = write!(
			string,
			"constant PushConstant& push_constant [[buffer({PUSH_CONSTANT_BINDING_INDEX})]]"
		);
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
				let _ = write!(string, "_{name}* {} [[buffer({index})]]", Self::identifier(name));
			}
			besl::BindingTypes::BufferArray { element, .. } => {
				let address_space = buffer_address_space(*memory_class, *write);
				self.emit_separator(string);
				string.push_str(address_space);
				string.push(' ');
				Self::emit_buffer_member_type(string, element.borrow().get_name().unwrap());
				string.push_str("* ");
				let _ = write!(string, "{} [[buffer({index})]]", Self::identifier(name));
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
				let _ = write!(
					string,
					"texture2d<{element_type}, {access}> {} [[texture({index})]]",
					Self::identifier(name)
				);
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
				let name = Self::identifier(name);
				let _ = write!(string, "{texture_type} {name} [[texture({index})]]");
				self.emit_separator(string);
				let _ = write!(string, "sampler {name}_sampler [[sampler({index})]]");
			}
		}
	}

	pub(crate) fn emit_compute_binding_reference(&self, string: &mut String, name: &str) {
		if self.mesh_stage_context.is_some() {
			string.push_str("resources.");
			Self::identifier(name).push_to(string);
			return;
		}

		match self.compute_binding_mode {
			ComputeBindingMode::ArgumentBuffers => {
				string.push_str("resources.");
				Self::identifier(name).push_to(string);
			}
			ComputeBindingMode::BareResources => Self::identifier(name).push_to(string),
		}
	}

	/// Qualifies a raster resource through the argument buffer supplied to its entry point or helper.
	pub(crate) fn emit_raster_binding_reference(&self, string: &mut String, name: &str) {
		string.push_str("resources.");
		Self::identifier(name).push_to(string);
	}

	/// Writes the implicit stage context of a BESL function: its declaration after the function's own parameters when
	/// `declare` is true, or the matching forwarded arguments at a call to it otherwise.
	///
	/// Metal has no module-level resources, push constants, workgroup storage, or stage builtins, so every helper that
	/// reaches them receives them as extra parameters. Declarations and calls walk the same list, so they always agree.
	/// Stage entry points write their own parameter lists, so `main` receives a context here only in a mesh stage.
	/// Call it from [`crate::shader::generator::NodeEmitter::emit_function_extra_parameters`], from
	/// [`Self::emit_function_prototype`], and from
	/// [`crate::shader::generator::NodeEmitter::emit_function_call_extra_arguments`].
	pub(crate) fn emit_hidden_context(
		&self,
		string: &mut String,
		function: &besl::NodeReference,
		has_previous_parameter: bool,
		declare: bool,
	) {
		let is_main = match function.borrow().node() {
			besl::Nodes::Function { name, .. } => name == "main",
			// Struct constructors and intrinsics take no hidden context.
			_ => return,
		};

		// Writes one entry. Declarations get the type prefix; forwarded arguments are the parameter name alone.
		let mut has_previous_parameter = has_previous_parameter;
		let mut push = |string: &mut String, declaration: std::fmt::Arguments<'_>, name: &dyn std::fmt::Display| {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			has_previous_parameter = true;
			if declare {
				let _ = string.write_fmt(declaration);
			}
			let _ = write!(string, "{name}");
		};

		if is_main {
			let Some(mesh) = &self.mesh_stage_context else {
				return;
			};
			if mesh.has_push_constant {
				push(string, format_args!("constant PushConstant& "), &"push_constant");
			}
			if mesh.has_resources {
				push(string, format_args!("constant _resources& "), &"resources");
			}
			push(string, format_args!("uint "), &"threadgroup_position");
			push(string, format_args!("uint "), &"thread_index");
			if mesh.has_task_payload {
				push(string, format_args!("const object_data ObjectPayload& "), &"payload");
			}
			push(
				string,
				format_args!(
					"metal::mesh<VertexOutput, PrimitiveOutput, {}, {}, topology::triangle> ",
					mesh.maximum_vertices, mesh.maximum_primitives
				),
				&"out_mesh",
			);
		} else if let Some(task) = &self.task_stage_context {
			if task.has_push_constant {
				push(string, format_args!("constant PushConstant& "), &"push_constant");
			}
			if task.has_resources {
				push(string, format_args!("constant _resources& "), &"resources");
			}
			if task.has_task_payload {
				push(string, format_args!("object_data ObjectPayload& "), &"payload");
			}
			push(string, format_args!("uint "), &"thread_position");
			push(string, format_args!("uint "), &"thread_index");
			for workgroup in &task.workgroups {
				let pointer = if workgroup.count.is_some() { "*" } else { "&" };
				push(
					string,
					format_args!("threadgroup {}{pointer} ", workgroup.msl_type),
					&Self::identifier(&workgroup.name),
				);
			}
			push(string, format_args!("thread mesh_grid_properties& "), &"mesh_grid");
		} else if self.in_compute_body {
			let Some(compute) = &self.compute_stage_context else {
				return;
			};
			// The lane index is a kernel builtin, so every caller on the path to its use must forward it.
			let uses_simd_lane_id = any_code_node(function, true, &mut |node| is_intrinsic_call(node, "subgroup_lane_index"));
			if !uses_simd_lane_id && !self.function_requires_resource_context(function) {
				return;
			}
			push(string, format_args!("uint2 "), &"gid");
			push(string, format_args!("uint "), &"thread_index");
			push(string, format_args!("uint2 "), &"threadgroup_position");
			if uses_simd_lane_id {
				push(string, format_args!("uint "), &"simd_lane_id");
			}
			if compute.has_push_constant {
				push(string, format_args!("constant PushConstant& "), &"push_constant");
			}
			if compute.has_resources {
				push(string, format_args!("constant _resources& "), &"resources");
			}
			for workgroup in &compute.workgroups {
				let pointer = if workgroup.count.is_some() { "*" } else { "&" };
				push(
					string,
					format_args!("threadgroup {}{pointer} ", workgroup.msl_type),
					&Self::identifier(&workgroup.name),
				);
			}
		} else if let Some(raster) = &self.raster_stage_context {
			if !raster.has_hidden_inputs() && !self.function_requires_resource_context(function) {
				return;
			}
			if raster.has_push_constant {
				push(string, format_args!("constant PushConstant& "), &"push_constant");
			}
			if raster.has_resources {
				push(string, format_args!("constant _resources& "), &"resources");
			}
			if raster.has_vertex_index {
				push(string, format_args!("uint "), &besl::VERTEX_INDEX_BUILTIN);
			}
			if raster.has_instance_index {
				push(string, format_args!("uint "), &besl::INSTANCE_INDEX_BUILTIN);
			}
		}
	}
}
