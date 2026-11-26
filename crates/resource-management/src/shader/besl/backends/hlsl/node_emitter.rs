use super::*;
use crate::resources::material::{SpecializationConstant, SpecializationValue};

/// Returns the `#define` lines that give each of `constants` its value in HLSL the BESL HLSL backend generated, to put
/// before the source when compiling one specialization of it.
///
/// The backend declares each scalar specialization constant as `SPEC_CONSTANT_<id>`, with its default only when that
/// name is undefined, and each vector as one such constant per component, at consecutive IDs. Floats are written as
/// their bits, so every value, including one that isn't finite, compiles to exactly the value given.
pub(crate) fn specialization_defines(constants: &[SpecializationConstant]) -> String {
	let mut defines = String::new();
	for constant in constants {
		let components: &[u32] = match &constant.value {
			SpecializationValue::Bool(value) => {
				let _ = writeln!(defines, "#define SPEC_CONSTANT_{} {value}", constant.id);
				continue;
			}
			SpecializationValue::F32(bits) => std::slice::from_ref(bits),
			SpecializationValue::Vec3f(bits) => bits,
			SpecializationValue::Vec4f(bits) => bits,
		};
		for (component, bits) in (0u32..).zip(components) {
			let _ = writeln!(
				defines,
				"#define SPEC_CONSTANT_{} asfloat({bits:#010x}u)",
				constant.id + component
			);
		}
	}
	defines
}

impl<'p> crate::shader::generator::NodeEmitter<'p> for Generator<'p> {
	fn program(&self) -> &'p besl::Program<'p> {
		self.program
	}

	fn type_from_besl(source: &str) -> &str {
		Generator::translate_type(source)
	}
	const SPECIALIZATION_QUALIFIER: &'static str = "static const";
	fn emit_specialization_constant(
		&self,
		string: &mut String,
		type_name: &str,
		name: std::fmt::Arguments<'_>,
		id: u32,
		default: &str,
	) {
		// HLSL has no pipeline specialization. A DX12 bake compiles each variant with the `SPEC_CONSTANT_<id>` defines
		// `specialization_defines` writes, and the define below supplies the default otherwise. Directives need their
		// own lines, even in minified output.
		let _ = write!(
			string,
			"\n#ifndef SPEC_CONSTANT_{id}\n#define SPEC_CONSTANT_{id} {default}\n#endif\nstatic const {type_name} {name}=SPEC_CONSTANT_{id};"
		);
	}
	fn minified(&self) -> bool {
		self.minified
	}
	fn match_break_depth(&mut self) -> &mut Option<usize> {
		&mut self.match_break_depth
	}
	fn is_reserved_identifier(name: &str) -> bool {
		super::reserved::is_reserved(name)
	}
	fn emit_function_attributes(&mut self, string: &mut String, _node: &besl::NodeId, name: &str) {
		if name != "main" {
			return;
		}

		let break_char = ShaderFormatting::new(self.minified).break_str();
		if matches!(self.stage, Stages::Mesh { .. }) {
			let _ = write!(string, "[outputtopology(\"triangle\")]{break_char}");
		}

		let Some(local_size) = self.stage.local_size() else {
			return;
		};
		// HLSL attaches compute-like stage thread-group sizes directly to their entry functions.
		let _ = write!(
			string,
			"[numthreads({}, {}, {})]{break_char}",
			local_size.width(),
			local_size.height(),
			local_size.depth()
		);
	}
	fn emit_function_extra_parameters(
		&mut self,
		string: &mut String,
		_node: &besl::NodeId,
		name: &str,
		has_previous_parameter: bool,
	) {
		if name != "main" {
			return;
		}
		if matches!(self.stage, Stages::Vertex | Stages::Fragment) {
			self.emit_raster_entry_parameters(string, has_previous_parameter);
			return;
		}

		// Compute, task, and mesh entry points receive the dispatch builtins.
		if has_previous_parameter {
			self.emit_separator(string);
		}
		string.push_str("uint3 dispatch_thread_id : SV_DispatchThreadID");
		self.emit_separator(string);
		string.push_str("uint3 group_thread_id : SV_GroupThreadID");
		self.emit_separator(string);
		string.push_str("uint3 group_id : SV_GroupID");
		self.emit_separator(string);
		string.push_str("uint group_thread_index : SV_GroupIndex");

		if let Stages::Mesh {
			maximum_vertices,
			maximum_primitives,
			..
		} = self.stage
		{
			if !self.task_payloads.is_empty() {
				self.emit_separator(string);
				string.push_str("in payload ObjectPayload payload");
			}
			self.emit_separator(string);
			let _ = write!(string, "out vertices VertexOutput besl_vertices[{maximum_vertices}]");
			self.emit_separator(string);
			let _ = write!(string, "out primitives PrimitiveOutput besl_primitives[{maximum_primitives}]");
			self.emit_separator(string);
			let _ = write!(string, "out indices uint3 besl_triangles[{maximum_primitives}]");
		}
	}
	// HLSL textures carry no sampler, so a texture parameter is paired with one named after it, as bindings are.
	fn emit_texture_parameter_sampler(&mut self, string: &mut String, name: &str) {
		self.emit_separator(string);
		let _ = write!(string, "SamplerState {}_sampler", Self::identifier(name));
	}
	fn emit_texture_argument_sampler(&mut self, string: &mut String, argument: &besl::NodeId) {
		self.emit_separator(string);
		self.emit_sampler(string, argument);
	}
	fn emit_function_call(&mut self, string: &mut String, function: &besl::NodeId, parameters: &[besl::NodeId]) -> bool {
		let program = self.program;
		let function_node = &program[function];
		let besl::Nodes::Struct {
			name, template: None, ..
		} = function_node.node()
		else {
			return false;
		};
		if self.is_square_column_vector_matrix_constructor(program.text(*name), parameters) {
			// Square BESL matrix constructors take columns, while their HLSL equivalents take rows.
			string.push_str("transpose(");
			string.push_str(Self::translate_type(program.text(*name)));
			string.push('(');
			self.emit_call_arguments(string, parameters);
			string.push_str("))");
			return true;
		}
		if crate::shader::generator::is_builtin_struct_type(program.text(*name)) {
			return false;
		}
		if !self.user_struct_constructors.contains(function) {
			self.user_struct_constructors.push(*function);
		}

		// Route portable BESL construction through the field-by-field factory emitted with the struct.
		// The factory name is derived from the raw BESL name, so it cannot collide with a reserved word.
		string.push_str("besl_construct_");
		string.push_str(program.text(*name));
		string.push('(');
		self.emit_call_arguments(string, parameters);
		string.push(')');
		true
	}
	fn emit_expression_member(&mut self, string: &mut String, name: besl::Name, source: &besl::NodeId) -> bool {
		let program = self.program;
		match program[source].node() {
			besl::Nodes::TaskPayload { .. } => {
				// A task shader writes its own array of the payload, which the entry point hands on; a mesh shader reads
				// the payload it received.
				if matches!(self.stage, Stages::Task { .. }) {
					string.push_str(super::generate::TASK_PAYLOAD_STAGING_PREFIX);
				} else {
					string.push_str("payload.");
				}
				Self::identifier(program.text(name)).push_to(string);
				return true;
			}
			besl::Nodes::Workgroup { .. } => {
				Self::identifier(program.text(name)).push_to(string);
				return true;
			}
			_ => {}
		}

		let Some(binding) = self.hlsl_buffer_binding_source(source) else {
			// Textures, images, and acceleration structures are globals named after their binding.
			if let besl::Nodes::Binding { name, .. } = program[source].node() {
				let _ = write!(string, "{}", crate::shader::generator::binding_global(program.text(*name)));
				return true;
			}
			return false;
		};
		let _ = write!(string, "{}", crate::shader::generator::binding_global(&binding.name));
		if program.text(name) != binding.name {
			// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
			string.push_str("[0].");
			Self::identifier(program.text(name)).push_to(string);
		}
		true
	}
	fn emit_variable_declaration(&mut self, string: &mut String, name: &str, type_name: &str) {
		// HLSL declares arrays in C position, so the count follows the variable name.
		Self::emit_c_declaration(string, name, type_name);
	}
	fn emit_expression_override(&mut self, string: &mut String, expression: &besl::Expressions) -> bool {
		let program = self.program;
		if self.emit_windowed_byte(string, expression) {
			return true;
		}
		if let Some((declaration, initializer)) = self.declaration_with_initializer(expression)
			&& let Some(elements) = crate::shader::generator::array_constructor_elements(program, initializer)
		{
			// HLSL has no array expressions, so the constructor becomes the declaration's brace initializer.
			// Validation rejects array constructors anywhere else.
			self.emit_node_string(string, declaration);
			string.push_str(if self.minified { "={" } else { " = {" });
			self.emit_call_arguments(string, &elements);
			string.push('}');
			return true;
		}
		if let besl::Expressions::Operator { operator, left, right } = expression {
			if *operator == besl::Operators::Assignment {
				let indexed_target = {
					let left = &program[left];
					let besl::Nodes::Expression(besl::Expressions::Accessor {
						left: member,
						right: index,
					}) = left.node()
					else {
						return false;
					};
					Some((*member, *index))
				};
				if let Some((member, index)) = indexed_target
					&& let Some((binding_name, _, true, Some(element_type))) = self.hlsl_buffer_member_target(&member)
					// Only narrow elements share words; a wrapped `mat4x3f` element stores through its `.value` member.
					&& super::hlsl_narrow_element(element_type).is_some()
				{
					let (elements_per_word, bits_per_element, element_mask) = if element_type == "u8" {
						(4u32, 8u32, "0xffu")
					} else {
						(2u32, 16u32, "0xffffu")
					};
					let id = self.packed_write_counter;
					self.packed_write_counter = self.packed_write_counter.checked_add(1).expect(
								"Packed narrow-buffer write count overflowed. The most likely cause is an invalid shader with billions of assignment nodes.",
							);
					let binding = crate::shader::generator::binding_global(&binding_name);

					// Adjacent logical narrow elements share one DX12 word. Replace the
					// selected lane with one compare-exchange loop so another lane cannot
					// change between separate clear and set operations.
					let _ = write!(string, "{{uint besl_packed_index_{id}=");
					self.emit_node_string(string, &index);
					let _ = write!(string, ";uint besl_packed_value_{id}=(uint(");
					self.emit_node_string(string, right);
					let _ = write!(
						string,
						")&{element_mask});uint besl_packed_shift_{id}=(besl_packed_index_{id}%{elements_per_word}u)*{bits_per_element}u;\
						 uint besl_packed_mask_{id}={element_mask}<<besl_packed_shift_{id};uint besl_packed_expected_{id};\
						 InterlockedOr({binding}[besl_packed_index_{id}/{elements_per_word}u],0u,besl_packed_expected_{id});\
						 for(;;){{uint besl_packed_desired_{id}=(besl_packed_expected_{id}&~besl_packed_mask_{id})|(besl_packed_value_{id}<<besl_packed_shift_{id});\
						 uint besl_packed_observed_{id};\
						 InterlockedCompareExchange({binding}[besl_packed_index_{id}/{elements_per_word}u],besl_packed_expected_{id},besl_packed_desired_{id},besl_packed_observed_{id});\
						 if(besl_packed_observed_{id}==besl_packed_expected_{id}){{break;}}besl_packed_expected_{id}=besl_packed_observed_{id};}}}}"
					);
					return true;
				}
			}

			// Only multiplication depends on the operand types, so other operators skip type inference.
			if *operator != besl::Operators::Multiply {
				return false;
			}
			let left_type = self.node_type_name(left);
			let right_type = self.node_type_name(right);
			if left_type.as_deref() == Some("mat4x3f") && right_type.as_deref() == Some("vec4f") {
				// HLSL float4x3 stores the four BESL columns as rows, so the vector must be the left mul operand.
				string.push_str("mul(");
				self.emit_node_string(string, right);
				string.push_str(", ");
				self.emit_node_string(string, left);
				string.push(')');
				return true;
			}
			if matches!(
				(left_type.as_deref(), right_type.as_deref()),
				(Some("mat4f"), Some("mat4f" | "vec4f"))
					| (Some("mat2f" | "mat3f" | "mat4f" | "mat4x3f"), Some("f32"))
					| (Some("f32"), Some("mat2f" | "mat3f" | "mat4f" | "mat4x3f"))
			) {
				// BESL reserves algebraic multiplication for these matrix
				// shapes. Same-shaped mat4x3 values use component-wise `*`.
				string.push_str("mul(");
				self.emit_node_string(string, left);
				string.push_str(", ");
				self.emit_node_string(string, right);
				string.push(')');
				return true;
			}
		}

		false
	}

	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeId, right: &besl::NodeId) {
		let program = self.program;
		if resource_reference_kind(program, left) == Some(ResourceAccessorKind::DescriptorArray) {
			// Every texture intrinsic other than `sample` reaches its descriptor-array element through here.
			self.emit_node_string(string, left);
			self.emit_descriptor_array_index(string, right);
			return;
		}

		let right_is_member = matches!(
			program[right].node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		);
		// Resolved at most once: the swizzle path needs it only for member accessors, and the indexing path always.
		let buffer_target = right_is_member.then(|| self.hlsl_buffer_member_target(left));
		if let Some(Some((binding_name, field_name, ..))) = &buffer_target
			&& field_name != binding_name
		{
			// A component selected from a buffer field remains an HLSL swizzle after the buffer access itself is lowered.
			let _ = write!(string, "{}", crate::shader::generator::binding_global(&binding_name));
			string.push_str("[0].");
			Self::identifier(&field_name).push_to(string);
			string.push('.');
			self.emit_node_string(string, right);
			return;
		}

		if let Some((field_name, per_vertex)) =
			crate::shader::generator::mesh_output_target(program, left, |name, per_vertex| (name.to_string(), per_vertex))
		{
			// Mesh attributes live in the native per-vertex or per-primitive output array rather than module globals.
			string.push_str(if per_vertex { "besl_vertices[" } else { "besl_primitives[" });
			self.emit_node_string(string, right);
			string.push_str("].");
			Self::identifier(&field_name).push_to(string);
			return;
		}

		if let Some(binding) = self.hlsl_buffer_binding_source(left)
			&& let Some(field_name) = self.hlsl_member_name(right)
		{
			// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
			let _ = write!(string, "{}", crate::shader::generator::binding_global(&binding.name));
			string.push_str("[0].");
			Self::identifier(&field_name).push_to(string);
			return;
		}

		if !right_is_member
			&& !self.hlsl_buffer_member_is_array(left)
			&& self
				.node_type_name(left)
				.as_deref()
				.and_then(Self::hlsl_square_matrix_column_type)
				.is_some()
		{
			// BESL indexes square matrices by column. HLSL indexes them by row,
			// regardless of the storage packing selected for the shader.
			string.push_str("transpose(");
			self.emit_node_string(string, left);
			string.push_str(")[");
			self.emit_node_string(string, right);
			string.push(']');
			return;
		}

		if let Some((binding_name, field_name, _, wrapped_element)) =
			buffer_target.unwrap_or_else(|| self.hlsl_buffer_member_target(left))
		{
			if wrapped_element == Some("mat4x3f") {
				// The element struct's one member holds the matrix with row-major packing; see `hlsl_wrapped_element`.
				let _ = write!(string, "{}", crate::shader::generator::binding_global(&binding_name));
				string.push('[');
				self.emit_node_string(string, right);
				string.push_str("].value");
				return;
			}
			if let Some(element_type) = wrapped_element {
				let (word_index, bit_offset, element_mask) = if element_type == "u8" {
					(") / 4u] >> (((", ") % 4u) * 8u)) & ", "0xffu")
				} else {
					(") / 2u] >> (((", ") % 2u) * 16u)) & ", "0xffffu")
				};

				// DX12 exposes packed narrow-index buffers as 32-bit structured words, so recover the logical element here.
				string.push_str("((");
				let _ = write!(string, "{}", crate::shader::generator::binding_global(&binding_name));
				string.push_str("[(");
				self.emit_node_string(string, right);
				string.push_str(word_index);
				self.emit_node_string(string, right);
				string.push_str(bit_offset);
				string.push_str(element_mask);
				string.push(')');
				return;
			}

			let _ = write!(string, "{}", crate::shader::generator::binding_global(&binding_name));
			if field_name != binding_name {
				// BESL buffers are engine storage buffers, so HLSL always reads fields through element zero.
				string.push_str("[0].");
				Self::identifier(&field_name).push_to(string);
			}
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
			return;
		}

		self.emit_wrapped_expression(string, left);
		// BESL numeric access always remains an HLSL subscript, including when
		// its left side is itself an array-element expression.
		if !right_is_member {
			string.push('[');
			self.emit_node_string(string, right);
			string.push(']');
		} else {
			string.push('.');
			self.emit_node_string(string, right);
		}
	}
	// Keep the intrinsic table contiguous because each arm defines one exact HLSL lowering contract.
	#[allow(clippy::too_many_lines)]
	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeId,
		arguments: &[besl::NodeId],
		elements: &[besl::NodeId],
	) {
		let program = self.program;
		let besl::Nodes::Intrinsic {
			name,
			elements: definition,
			r#return,
		} = program[intrinsic].node()
		else {
			unreachable!("An intrinsic call always calls an intrinsic.");
		};
		let name = program.text(*name);

		let template = match name {
			// Texture samples bypass intrinsic bodies.
			"sample" => {
				let accessor = resource_accessor(program, &arguments[0]);
				self.emit_node_string(string, accessor.as_ref().map_or(&arguments[0], |(_, resource, _)| resource));
				if let Some((ResourceAccessorKind::DescriptorArray, _, index)) = &accessor {
					self.emit_descriptor_array_index(string, index);
				}
				string.push_str(".Sample(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				if let Some((ResourceAccessorKind::Texture2DArrayLayer, _, index)) = &accessor {
					string.push_str("float3(");
					self.emit_node_string(string, &arguments[1]);
					string.push_str(", float(");
					self.emit_node_string(string, index);
					string.push_str("))");
				} else {
					self.emit_node_string(string, &arguments[1]);
				}
				string.push(')');
				return;
			}
			"sample_texture_2d_array_grad" => "$0$d1.SampleGrad($0_sampler$d1$,$2$,$3$,$4)",
			// Ray tracing stage values are system values, or members of the hit a closest hit shades. See
			// `ray_tracing.rs`.
			"launch_index" => "DispatchRaysIndex()",
			"launch_size" => "DispatchRaysDimensions()",
			"hit_distance"
			| "hit_custom_index"
			| "hit_geometry_index"
			| "hit_primitive_index"
			| "hit_barycentrics"
			| "ray_origin"
			| "ray_direction" => return Self::emit_hit_value(string, name),
			// These call the helpers the ray tracing prelude and the call struct declare. An acceleration structure
			// binding is a global, so it passes like any other argument.
			"intersect" => "_besl_intersect($*)",
			"trace_occlusion" => "_besl_trace_occlusion($*)",
			"invoke" => "_besl_invoke($*)",
			// Every other intrinsic with a body emits its expansion.
			_ if definition
				.iter()
				.any(|element| !matches!(program[element].node(), besl::Nodes::Parameter { .. })) =>
			{
				for element in elements {
					self.emit_node_string(string, element);
				}
				return;
			}
			"pow" if arguments.len() == 2 && super::super::is_two(program, &arguments[0]) => "exp2($1)",
			"fetch" if arguments.len() == 3 => "$0.Load(int4($1, int($2), 0))",
			"fetch" => "$0.Load(int3($1, 0))",
			"fetch_u32" => "$0.Load(int3($1, 0)).x",
			"image_load" | "image_load_u32" => "$c0",
			"gather" if arguments.len() == 3 => "$0.Gather($s0, float3($1, float($2)))",
			"gather" => "$0.Gather($s0, $1)",
			"texture_lod" | "downsample_min" | "downsample_max" => {
				self.emit_template(
					string,
					match arguments.len() {
						4 => "$0.SampleLevel($s0, float3($1, float($2)), $3)",
						3 => "$0.SampleLevel($s0, $1, $2)",
						_ => "$0.SampleLevel($s0, $1, 0.0)",
					},
					arguments,
				);
				if name != "texture_lod" {
					string.push_str(".x");
				}
				return;
			}
			"texture_cube_array_lod" => "$0.SampleLevel($s0, float4($1, float($2)), $3)",
			"guard_image_bounds" => {
				// HLSL has no portable image bounds guard intrinsic, so emit the guard inline at the call site. A layered
				// image also writes its layer count, which the guard ignores.
				if let Some((image, _)) = besl::layered_image_access(program, &arguments[0]) {
					string.push_str("uint3 _besl_image_size; ");
					self.emit_node_string(string, &image);
					string.push_str(".GetDimensions(_besl_image_size.x, _besl_image_size.y, _besl_image_size.z); if (any(");
					self.emit_node_string(string, &arguments[1]);
					string.push_str(" >= _besl_image_size.xy)) { return; }");
					return;
				}
				"uint2 _besl_image_size; $0.GetDimensions(_besl_image_size.x, _besl_image_size.y); if (any($1 >= _besl_image_size)) { return; }"
			}
			"image_size" if let Some((image, _)) = besl::layered_image_access(program, &arguments[0]) => {
				// Every layer has the same size, so the layered image answers for the layer it selects.
				string.push_str("_besl_array_texture_size(");
				self.emit_node_string(string, &image);
				string.push(')');
				return;
			}
			"image_size" | "texture_size" => {
				// `GetDimensions` also writes an array texture's layer count. Both texture kinds have the type `Texture2D`,
				// so the binding's format tells them apart, or the overload the call resolved to for a texture parameter.
				let array_texture = match besl::referenced_binding(program, &arguments[0]) {
					Some(binding) => matches!(
						program[binding].node(),
						besl::Nodes::Binding {
							r#type: besl::BindingTypes::CombinedImageSampler { format, .. },
							..
						} if format == "ArrayTexture2D"
					),
					None => definition.iter().any(|element| {
						matches!(
							program[element].node(),
							besl::Nodes::Parameter { r#type, .. } if program.name(*r#type) == Some("ArrayTexture2D")
						)
					}),
				};
				if array_texture {
					"_besl_array_texture_size($0)"
				} else {
					"_besl_texture_size($0)"
				}
			}
			"write" | "write_u32" => "$c0 = $2",
			"image_atomic_or"
			| "image_atomic_max"
			| "atomic_load"
			| "atomic_store"
			| "atomic_exchange"
			| "atomic_compare_exchange"
			| "atomic_add"
			| "atomic_sub"
			| "atomic_min"
			| "atomic_max"
			| "atomic_and"
			| "atomic_or"
			| "atomic_xor" => {
				// A store returns nothing, so its helper returns nothing.
				let return_type = (name != "atomic_store").then(|| program.name(*r#return).unwrap_or("u32"));
				return self.emit_hlsl_atomic_helper_call(string, name, arguments, return_type);
			}
			// A fragment's invocation coordinate is the pixel it covers.
			"thread_id" if matches!(self.stage, Stages::Fragment) => "uint2(_besl_fragment_position.xy)",
			"thread_id" => "dispatch_thread_id.xy",
			"thread_layer" => "dispatch_thread_id.z",
			"thread_position" => "dispatch_thread_id.x",
			"thread_idx" => "group_thread_index",
			"subgroup_lane_index" => "WaveGetLaneIndex()",
			"threadgroup_position" => "group_id.x",
			"sincos" => "float2(sin($0), cos($0))",
			"round_to_i32" => "int2(round($0))",
			// HLSL stores a BESL `u8` as a `uint`, so the conversion keeps the low byte itself.
			"u8" => "(uint($0) & 0xFFu)",
			"workgroup_barrier" => "GroupMemoryBarrierWithGroupSync()",
			"set_task_mesh_output_count" => "besl_mesh_output_count = $0",
			"set_mesh_vertex_position" => "besl_vertices[$0].position = $1",
			"set_mesh_triangle" => "besl_triangles[$0] = $1",
			"set_mesh_primitive_render_target_array_index" => "besl_primitives[$0].render_target_array_index = $1",
			// HLSL's `sign` returns an integer of the argument's width, so the result is cast back to the float type.
			"sign" => {
				string.push_str("((");
				string.push_str(program.name(*r#return).map_or("float", Self::translate_type));
				string.push_str(")sign(");
				self.emit_call_arguments(string, arguments);
				string.push_str("))");
				return;
			}
			// Every other intrinsic is one HLSL call, renamed where HLSL spells the operation differently.
			_ => {
				string.push_str(match name {
					"is_nan" => "isnan",
					"is_infinite" => "isinf",
					"is_finite" => "isfinite",
					"is_normal" => "isnormal",
					// firstbitlow already returns 0xFFFFFFFF for zero.
					"find_lsb" => "firstbitlow",
					"fract" => "frac",
					"mix" => "lerp",
					"u32" => "uint",
					"f32" | "f16" | "u16" | "vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" => {
						Self::translate_type(name)
					}
					"inversesqrt" => "rsqrt",
					"subgroup_ballot" => "WaveActiveBallot",
					"subgroup_ballot_any" => "_besl_subgroup_ballot_any",
					"subgroup_ballot_find_lsb" => "_besl_subgroup_ballot_find_lsb",
					"subgroup_ballot_count" => "_besl_subgroup_ballot_count",
					"subgroup_ballot_and_not" => "_besl_subgroup_ballot_and_not",
					"subgroup_broadcast_u32" | "subgroup_broadcast_f32" => "WaveReadLaneAt",
					"subgroup_shuffle_xor_f32" => "_besl_subgroup_shuffle_xor_f32",
					"fma" if matches!(program.name(*r#return), Some("f16" | "vec2f16" | "vec3f16" | "vec4f16")) => {
						"_besl_fma_f16"
					}
					"fma" => "mad",
					"set_mesh_output_counts" => "SetMeshOutputCounts",
					"min" | "max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin"
					| "atan2" | "floor" | "ceil" | "round" | "fwidth" | "step" | "radians" | "smoothstep" | "dot" | "cross"
					| "normalize" | "reflect" | "length" | "distance" | "exp2" => name,
					// Intrinsics without an HLSL call emit their elements.
					_ => {
						for element in elements {
							self.emit_node_string(string, element);
						}
						return;
					}
				});
				"($*)"
			}
		};
		self.emit_template(string, template, arguments);
	}
	/// Writes the HLSL forms of a resource argument: `$s` the sampler paired with a texture, `$d` the subscript that
	/// picks one element of a descriptor array, and `$c` the texel of an image at the coordinate the next argument holds.
	fn emit_template_argument(&mut self, string: &mut String, kind: u8, arguments: &[besl::NodeId], index: usize) {
		match kind {
			b's' => self.emit_sampler(string, &arguments[index]),
			b'd' => self.emit_descriptor_array_index(string, &arguments[index]),
			b'c' => self.emit_image_texel(string, &arguments[index], &arguments[index + 1]),
			_ => unreachable!("HLSL intrinsic templates use no `${}` placeholder.", kind as char),
		}
	}
	fn emit_node(&mut self, string: &mut String, node: &besl::NodeId) {
		self.emit_node_string(string, node)
	}
}
