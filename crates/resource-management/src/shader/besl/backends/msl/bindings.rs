use super::*;

/// The Metal type of an `AccelerationStructure` descriptor: a top-level structure whose instances hold the scene's
/// geometry. Metal binds it through a buffer index or an argument buffer member.
pub(crate) const ACCELERATION_STRUCTURE_TYPE: &str = "raytracing::instance_acceleration_structure";

/// Selects the Metal texture element type and access for a storage image.
pub(crate) fn storage_image_type(format: &str, read: bool, write: bool) -> (&'static str, &'static str) {
	let element_type = match format {
		"r8ui" | "r16ui" | "r32ui" => "uint",
		_ => "float",
	};
	let access = if read && write {
		"access::read_write"
	} else if write {
		"access::write"
	} else {
		"access::read"
	};
	(element_type, access)
}

impl Generator {
	pub(crate) fn emit_push_constant_struct(&mut self, string: &mut String, push_constant: &besl::NodeReference) {
		let node = push_constant.borrow();
		let besl::Nodes::PushConstant { members } = node.node() else {
			return;
		};

		self.emit_named_struct_start(string, "PushConstant");

		let formatting = ShaderFormatting::new(self.minified);
		emit_statement_block(string, formatting, members, 1, |string, member| {
			self.emit_node_string(string, member)
		});

		self.emit_struct_declaration_end(string);
	}

	pub(crate) fn emit_object_payload_struct(&mut self, string: &mut String, payloads: &[&besl::NodeReference]) {
		if !payloads.is_empty() {
			self.emit_task_payload_struct(string, "ObjectPayload", payloads.iter().copied());
		}
	}

	/// Emits one argument-buffer struct per descriptor set, named `_set<n>`.
	///
	/// Argument IDs are dense and follow slot order: a binding takes one ID per array element, and a combined image
	/// sampler takes its textures first and then as many samplers. An ID is also the entry's index in the encoded
	/// buffer, which the Metal GHI backend lays out by the same rule, so every shader that shares a layout reads one
	/// buffer.
	pub(crate) fn emit_argument_buffer_structs(&mut self, string: &mut String) {
		for set in 0..self.argument_sets.len() {
			self.emit_named_struct_start(string, format_args!("_set{set}"));

			let bindings = self.argument_sets[set].clone();
			let mut next_id = 0;
			for binding in &bindings {
				self.emit_argument_buffer_field(string, binding, &mut next_id);
			}

			self.emit_struct_declaration_end(string);
		}
	}

	/// Emits one field at `next_id` and advances `next_id` past the IDs the field takes.
	pub(crate) fn emit_argument_buffer_field(
		&mut self,
		string: &mut String,
		binding_node: &besl::NodeReference,
		next_id: &mut u32,
	) {
		let node = binding_node.borrow();
		let besl::Nodes::Binding {
			name,
			read,
			write,
			memory_class,
			r#type,
			count,
			..
		} = node.node()
		else {
			return;
		};

		let descriptor_count = count.map(|count| count.get()).unwrap_or(1);
		let (primary_id, secondary_id) = (*next_id, *next_id + descriptor_count);
		*next_id += match r#type {
			besl::BindingTypes::CombinedImageSampler { .. } => 2 * descriptor_count,
			_ => descriptor_count,
		};
		let emit_suffix = |string: &mut String, argument_id: u32| {
			let _ = write!(string, " [[id({argument_id})]]");
			if let Some(count) = count {
				let _ = write!(string, "[{count}]");
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
				let (element_type, access) = storage_image_type(format, *read, *write);
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
			besl::BindingTypes::AccelerationStructure => {
				let _ = write!(string, "{ACCELERATION_STRUCTURE_TYPE} {}", Self::identifier(name));
				emit_suffix(string, primary_id);
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

		let formatting = ShaderFormatting::new(self.minified);
		emit_statement_block(string, formatting, members, 1, |string, member| {
			self.emit_node_string(string, member)
		});

		self.emit_struct_declaration_end(string);
	}

	/// Emits the storage spelling of a struct member or array-buffer element type. User struct names keep their
	/// backend-safe name.
	///
	/// Every vector is packed, so it keeps the size and scalar alignment that HLSL, GLSL scalar layout, and the
	/// CPU-side `ghi::pod` vectors use: a native `float3` takes 16 bytes, and a native `float2` or `float4` needs an 8-
	/// or 16-byte offset. On Apple GPUs packed loads and stores at native offsets compile to the same instructions and
	/// run at the same speed as native ones. A shader that writes a lone `vec4` member into a buffer record measured
	/// about 10 % slower when the member crosses a 16-byte boundary, so keep written `vec4` members on 16-byte offsets.
	pub(crate) fn emit_buffer_member_type(string: &mut String, source: &str) {
		if source == "mat4x3f" {
			string.push_str("_besl_packed_float4x3");
		} else if source.starts_with("vec") {
			string.push_str("packed_");
			string.push_str(Self::translate_type(source));
		} else {
			Self::type_identifier(source).push_to(string);
		}
	}

	pub(crate) fn emit_compute_entry_point(
		&mut self,
		string: &mut String,
		main_function_node: &besl::NodeReference,
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

		if self.has_argument_sets() {
			self.emit_separator(string);
			self.emit_argument_buffer_parameter(string);
		}

		ShaderFormatting::new(self.minified).push_block_start(string);

		self.emit_compute_workgroup_declarations(string, workgroups);
		self.emit_statement_block(string, statements, 1);

		self.emit_block_end(string);
	}

	/// Emits function-scope threadgroup variables shared by every invocation in one compute or object workgroup.
	pub(crate) fn emit_compute_workgroup_declarations(&mut self, string: &mut String, workgroups: &[&besl::NodeReference]) {
		for workgroup in workgroups {
			let workgroup = workgroup.borrow();
			let besl::Nodes::Workgroup { name, format, count } = workgroup.node() else {
				continue;
			};
			self.emit_indentation(string, 1);
			let _ = write!(
				string,
				"threadgroup {} {}",
				Self::type_identifier(format.borrow().get_name().unwrap()),
				Self::identifier(name)
			);
			if let Some(count) = count {
				let _ = write!(string, "[{count}]");
			}
			self.emit_statement_end(string);
		}
	}

	/// Writes an object or mesh entry point's authored, push-constant, and argument-buffer parameters, then the
	/// separator before its stage builtins.
	fn emit_kernel_entry_parameters(
		&mut self,
		string: &mut String,
		params: &[besl::NodeReference],
		has_push_constant: bool,
		has_resources: bool,
	) {
		self.emit_call_arguments(string, params);
		let mut has_previous_parameter = !params.is_empty();
		if has_push_constant {
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

		let _ = write!(
			string,
			"[[object, max_total_threadgroups_per_mesh_grid({maximum_mesh_threadgroups})]] void "
		);
		if *name == "main" {
			string.push_str(MSL_ENTRY_POINT);
		} else {
			Self::identifier(name).push_to(string);
		}
		string.push('(');
		self.emit_kernel_entry_parameters(string, params, push_constant.is_some(), has_resources);
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
		self.emit_compute_workgroup_declarations(string, workgroups);
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
		self.emit_kernel_entry_parameters(string, params, push_constant.is_some(), has_resources);
		string.push_str("uint threadgroup_position [[threadgroup_position_in_grid]]");
		self.emit_separator(string);
		string.push_str("uint thread_index [[thread_index_in_threadgroup]]");
		if has_task_payload {
			self.emit_separator(string);
			string.push_str("const object_data ObjectPayload& payload [[payload]]");
		}
		self.emit_separator(string);
		let _ = write!(
			string,
			"metal::mesh<VertexOutput, PrimitiveOutput, {maximum_vertices}, {maximum_primitives}, topology::triangle> out_mesh"
		);

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

	/// Qualifies a resource through the argument buffer of its descriptor set.
	pub(crate) fn emit_binding_reference(&self, string: &mut String, name: &str) {
		if let Some(set) = self.binding_sets.get(name) {
			let _ = write!(string, "{}.", SetArgument(*set));
		}
		Self::identifier(name).push_to(string);
	}

	/// Writes the implicit stage context of a BESL function: its declaration after the function's own parameters when
	/// `declare` is true, or the matching forwarded arguments at a call to it otherwise.
	///
	/// Metal has no module-level resources, push constants, workgroup storage, or stage builtins, so every helper that
	/// reaches them receives them as extra parameters. Declarations and calls walk the same list, so they always agree.
	/// Stage entry points write their own parameter lists, so `main` receives a context here only in a mesh stage and in
	/// a closest hit, whose `main` runs inside the visible entry point.
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

		if let Some(ray_tracing) = self.ray_tracing_stage_context {
			// A helper that reads no stage value and no resource takes no context.
			let context = self.hidden_context(function);
			if !is_main && !context.uses_ray_tracing_inputs && !context.requires_resources {
				return;
			}
			if ray_tracing.closest_hit {
				push(string, format_args!("thread const {}& ", besl::HIT_STRUCT), &HIT_ARGUMENT);
			} else {
				push(string, format_args!("uint3 "), &LAUNCH_INDEX_ARGUMENT);
				push(string, format_args!("uint3 "), &LAUNCH_SIZE_ARGUMENT);
				if ray_tracing.invokes {
					push(
						string,
						format_args!("visible_function_table<{CLOSEST_HIT_TYPE}> "),
						&HITS_ARGUMENT,
					);
					push(string, format_args!("thread const {CONTEXT_STRUCT}& "), &CONTEXT_ARGUMENT);
				}
			}
			if ray_tracing.has_push_constant {
				push(string, format_args!("constant PushConstant& "), &"push_constant");
			}
			for set in 0..self.argument_sets.len() {
				push(string, format_args!("constant _set{set}& "), &SetArgument(set));
			}
		} else if is_main {
			let Some(mesh) = &self.mesh_stage_context else {
				return;
			};
			if mesh.has_push_constant {
				push(string, format_args!("constant PushConstant& "), &"push_constant");
			}
			if mesh.has_resources {
				for set in 0..self.argument_sets.len() {
					push(string, format_args!("constant _set{set}& "), &SetArgument(set));
				}
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
				for set in 0..self.argument_sets.len() {
					push(string, format_args!("constant _set{set}& "), &SetArgument(set));
				}
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
			let context = self.hidden_context(function);
			let uses_simd_lane_id = context.uses_simd_lane_id;
			if !uses_simd_lane_id && !context.requires_resources {
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
				for set in 0..self.argument_sets.len() {
					push(string, format_args!("constant _set{set}& "), &SetArgument(set));
				}
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
			if !raster.has_hidden_inputs() && !self.hidden_context(function).requires_resources {
				return;
			}
			if raster.has_push_constant {
				push(string, format_args!("constant PushConstant& "), &"push_constant");
			}
			if raster.has_resources {
				for set in 0..self.argument_sets.len() {
					push(string, format_args!("constant _set{set}& "), &SetArgument(set));
				}
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
