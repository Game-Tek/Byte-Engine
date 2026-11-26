use super::*;
impl<'p> crate::shader::generator::NodeEmitter<'p> for Generator<'p> {
	fn program(&self) -> &'p besl::Program<'p> {
		self.program
	}

	fn type_from_besl(source: &str) -> &str {
		Generator::translate_type(source)
	}
	const SPECIALIZATION_QUALIFIER: &'static str = "constant";
	fn emit_specialization_constant(
		&self,
		string: &mut String,
		type_name: &str,
		name: std::fmt::Arguments<'_>,
		id: u32,
		_default: &str,
	) {
		// Metal folds the value in when the pipeline is created, so the host passes every constant the shader reads.
		let _ = write!(string, "constant {type_name} {name} [[function_constant({id})]];");
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
	fn emit_discard(&mut self, string: &mut String) {
		string.push_str("discard_fragment()");
	}
	// Keep the intrinsic table contiguous because each arm defines one exact Metal lowering contract.
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
			r#return: return_type,
		} = program[intrinsic].node()
		else {
			unreachable!("An intrinsic call always calls an intrinsic.");
		};
		let name = program.text(*name);

		let template = match name {
			// Texture lowerings bypass intrinsic bodies.
			"sample" => {
				let accessor = resource_accessor(program, &arguments[0]);
				match &accessor {
					Some((kind, resource, index)) => {
						self.emit_intrinsic_resource_reference(string, resource);
						if *kind == ResourceAccessorKind::DescriptorArray {
							string.push('[');
							self.emit_node_string(string, index);
							string.push(']');
						}
					}
					None => self.emit_node_string(string, &arguments[0]),
				}
				string.push_str(".sample(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[1]);
				if let Some((ResourceAccessorKind::Texture2DArrayLayer, _, index)) = &accessor {
					string.push_str(", ");
					self.emit_node_string(string, index);
				}
				string.push(')');
				return;
			}
			"sample_texture_2d_array_grad" => {
				self.emit_template(string, "$r0[$1].sample(", arguments);
				if !emit_fixed_sampler_name(program, string, &arguments[0]) {
					self.emit_template(string, "$r0_sampler[$1]", arguments);
				}
				"$,$2, metal::gradient2d($3$,$4))"
			}
			// Metal reduces in a sampler only on Apple10 GPUs, so a min or max sampler reduces in shader code, through the
			// downsample helpers. The texture has one channel, which a sampler expands to `(value, 0, 0, 1)`.
			"texture_lod" if besl::lexer::binding_sampler(program, &arguments[0]).is_some_and(|sampler| sampler.reduces()) => {
				let helper = match besl::lexer::binding_sampler(program, &arguments[0]).map(|sampler| sampler.reduction) {
					Some(besl::sampler::Reduction::Min) => "_besl_downsample_min",
					_ => "_besl_downsample_max",
				};
				let _ = write!(string, "float4({helper}(");
				self.emit_template(string, "$0, $s0, $1, ", arguments);
				if arguments.len() == 2 {
					"0.0), 0.0, 0.0, 1.0)"
				} else {
					"$2), 0.0, 0.0, 1.0)"
				}
			}
			// Qualify the Metal helper so BESL identifiers such as `level` cannot shadow it.
			"texture_lod" if arguments.len() == 2 => "$0.sample($s0, $1, metal::level(0.0))",
			"texture_lod" => "$0.sample($s0, $1, metal::level($2))",
			"texture_cube_array_lod" => "$0.sample($s0, $1, $2, metal::level($3))",
			"gather" if arguments.len() == 3 => "$0.gather($s0, $1, $2, int2(0), component::x)",
			"gather" => "$0.gather($s0, $1, int2(0), component::x)",
			// The helpers gather and reduce in shader code; see `generate_msl_header_block`.
			"downsample_min" | "downsample_max" => {
				let _ = write!(string, "_besl_{name}(");
				if arguments.len() == 4 {
					"$0, $s0, $1, $2, $3)"
				} else {
					"$0, $s0, $1, $2)"
				}
			}
			// Ray tracing stage values are kernel parameters, or members of the hit a closest hit shades. See
			// `ray_tracing.rs`.
			"launch_index" => LAUNCH_INDEX_ARGUMENT,
			"launch_size" => LAUNCH_SIZE_ARGUMENT,
			"hit_distance"
			| "hit_custom_index"
			| "hit_geometry_index"
			| "hit_primitive_index"
			| "hit_barycentrics"
			| "ray_origin"
			| "ray_direction" => return self.emit_hit_value(string, name),
			// The helper runs the closest hit of the hit's record through the kernel's shader binding table, and passes it
			// the addresses of the kernel's resources.
			"invoke" => {
				string.push_str("_besl_invoke(");
				string.push_str(HITS_ARGUMENT);
				self.emit_separator(string);
				string.push_str(CONTEXT_ARGUMENT);
				"$,$*)"
			}
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
			// BESL integer literals are unsigned, but C++ spells them as `int`, so `min(uint, 3)` matches no Metal
			// overload exactly. Casting every argument selects the unsigned overload the lexer resolved.
			"min" | "max" | "clamp" if program.name(*return_type) == Some("u32") => {
				string.push_str(name);
				string.push('(');
				for (index, argument) in arguments.iter().enumerate() {
					if index > 0 {
						self.emit_separator(string);
					}
					string.push_str("uint(");
					self.emit_node_string(string, argument);
					string.push(')');
				}
				")"
			}
			"sincos" => "_besl_sincos($0)",
			"round_to_i32" => "int2(round($0))",
			"radians" if self.minified => "($0*(PI/180.0))",
			"radians" => "($0 * (PI / 180.0))",
			"atomic_exchange" | "atomic_add" | "atomic_sub" | "atomic_min" | "atomic_max" | "atomic_and" | "atomic_or"
			| "atomic_xor" | "atomic_load" | "atomic_store" => {
				let operation = &name["atomic_".len()..];
				// Metal names its read-modify-write operations `atomic_fetch_<operation>_explicit`.
				let fetch = if matches!(operation, "exchange" | "load" | "store") {
					""
				} else {
					"fetch_"
				};
				let _ = write!(string, "atomic_{fetch}{operation}_explicit(&");
				"$*, memory_order_relaxed)"
			}
			"thread_position" => "thread_position",
			// A fragment's invocation coordinate is the pixel it covers.
			"thread_id" if self.raster_stage_context.is_some_and(|context| context.fragment) => {
				"uint2(_besl_fragment_position.xy)"
			}
			"thread_id" => "gid.xy",
			"thread_layer" => "gid.z",
			"thread_idx" => "thread_index",
			"subgroup_lane_index" => "simd_lane_id",
			"threadgroup_position" if self.compute_stage_context.is_some() => "threadgroup_position.x",
			"threadgroup_position" => "threadgroup_position",
			"workgroup_barrier" => "threadgroup_barrier(mem_flags::mem_threadgroup)",
			"set_task_mesh_output_count" => "mesh_grid.set_threadgroups_per_grid(uint3($0, 1, 1))",
			"set_mesh_output_counts" => "if(thread_index==0){out_mesh.set_primitive_count($1);}",
			"set_mesh_vertex_position" => "out_mesh.set_vertex($0, VertexOutput{.position = $1})",
			// Materialize each argument once because Metal needs three index writes for one triangle.
			"set_mesh_triangle" => {
				"{uint _besl_triangle_index=$0;uint3 _besl_triangle=$1;out_mesh.set_index(_besl_triangle_index*3+0,_besl_triangle.x);out_mesh.set_index(_besl_triangle_index*3+1,_besl_triangle.y);out_mesh.set_index(_besl_triangle_index*3+2,_besl_triangle.z);}"
			}
			"set_mesh_primitive_render_target_array_index" => {
				"out_mesh.set_primitive($0, PrimitiveOutput{.render_target_array_index = $1})"
			}
			// Texture atomics (MSL 3.1) need the texture bound `read_write` and created for atomic use, see
			// `ghi::Uses::Atomic`. They return the previous texel as a vector whose first component is the value.
			"image_atomic_or" => "$0.atomic_fetch_or($1$,$2).x",
			"image_atomic_max" => "$0.atomic_fetch_max($1$,$2).x",
			"image_load" => "$i0.read($1$l0)",
			"image_load_u32" => "$i0.read($1$l0).x",
			"fetch" if arguments.len() == 3 => "$0.read($1$,$2)",
			"fetch" => "$0.read($1)",
			"texture_size" | "image_size" => "uint2($i0.get_width(),$i0.get_height())",
			// A tile output lives in the tile's imageblock, which holds only this tile, so the write skips pixels outside
			// the tile and past the outputs' extent. With one output the pixel's struct is the output; with several, the
			// write keeps the pixel's other outputs.
			"write" if self.tile_stage_context.is_some() && besl::referenced_output(program, &arguments[0]).is_some() => {
				let tile = self.tile_stage_context.as_ref().map_or(1, |tile| tile.tile);
				let keeps_others = self.tile_stage_context.as_ref().is_some_and(|tile| tile.output_count > 1);
				string.push_str("{uint2 _besl_pixel=");
				self.emit_node_string(string, &arguments[1]);
				let _ = write!(
					string,
					";uint2 _besl_offset=_besl_pixel-threadgroup_position.xy*{tile}u;if(all(_besl_offset<uint2({tile}u))&&all(_besl_pixel<output_extent)){{{TILE_OUTPUTS_STRUCT} _besl_outputs"
				);
				if keeps_others {
					string.push_str("=tile_block.read(ushort2(_besl_offset))");
				}
				string.push_str(";_besl_outputs.");
				self.emit_output_name(string, &arguments[0]);
				"=$2;tile_block.write(_besl_outputs,ushort2(_besl_offset));}}"
			}
			"output_extent" => "output_extent",
			"write_u32" => "$i0.write(uint4($2)$,$1$l0)",
			"write" => "$i0.write($2$,$1$l0)",
			"guard_image_bounds" => "if($1.x>=$i0.get_width()||$1.y>=$i0.get_height()){return;}",
			// Every other intrinsic is a Metal call, renamed where Metal spells the operation differently.
			_ => {
				string.push_str(match name {
					"is_nan" => "isnan",
					"is_infinite" => "isinf",
					"is_finite" => "isfinite",
					"is_normal" => "isnormal",
					"inversesqrt" => "rsqrt",
					"f32" | "f16" | "u32" | "u16" | "u8" | "vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" => {
						Self::translate_type(name)
					}
					// These call the helpers that `generate_msl_header_block` declares.
					"atomic_compare_exchange" => "_besl_atomic_compare_exchange",
					"find_lsb" => "_besl_find_lsb",
					"intersect" => "_besl_intersect",
					"trace_occlusion" => "_besl_trace_occlusion",
					"subgroup_ballot" => "_besl_subgroup_ballot",
					"subgroup_ballot_any" => "_besl_subgroup_ballot_any",
					"subgroup_ballot_find_lsb" => "_besl_subgroup_ballot_find_lsb",
					"subgroup_ballot_count" => "_besl_subgroup_ballot_count",
					"subgroup_ballot_and_not" => "_besl_subgroup_ballot_and_not",
					"subgroup_broadcast_u32" => "_besl_subgroup_broadcast_u32",
					"subgroup_broadcast_f32" => "_besl_subgroup_broadcast_f32",
					"subgroup_shuffle_xor_f32" => "_besl_subgroup_shuffle_xor_f32",
					name => name,
				});
				"($*)"
			}
		};
		self.emit_template(string, template, arguments);
	}
	/// Writes the Metal forms of a resource argument: `$r` a resource through the stage's resource context, `$s` the
	/// sampler paired with a texture, and `$i` and `$l` the texture of a storage image and the layer it selects.
	fn emit_template_argument(&mut self, string: &mut String, kind: u8, arguments: &[besl::NodeId], index: usize) {
		let argument = &arguments[index];
		match kind {
			b'r' => self.emit_intrinsic_resource_reference(string, argument),
			b's' => self.emit_sampler(string, argument),
			b'i' => self.emit_storage_image(string, argument),
			b'l' => self.emit_storage_image_layer(string, argument),
			_ => unreachable!("MSL intrinsic templates use no `${}` placeholder.", kind as char),
		}
	}
	fn emit_function_extra_parameters(
		&mut self,
		string: &mut String,
		node: &besl::NodeId,
		_name: &str,
		has_previous_parameter: bool,
	) {
		self.emit_hidden_context(string, node, has_previous_parameter, true);
	}
	fn emit_function_statement_block(&mut self, string: &mut String, statements: &[besl::NodeId], indent: usize) {
		self.emit_statement_block(string, statements, indent);
	}
	fn emit_function_call_extra_arguments(
		&mut self,
		string: &mut String,
		function: &besl::NodeId,
		has_previous_argument: bool,
	) {
		self.emit_hidden_context(string, function, has_previous_argument, false);
	}
	// Metal textures carry no sampler, so a texture parameter is paired with one named after it, as bindings are.
	fn emit_texture_parameter_sampler(&mut self, string: &mut String, name: &str) {
		self.emit_separator(string);
		let _ = write!(string, "sampler {}_sampler", Self::identifier(name));
	}
	fn emit_texture_argument_sampler(&mut self, string: &mut String, argument: &besl::NodeId) {
		self.emit_separator(string);
		self.emit_sampler(string, argument);
	}
	fn emit_variable_declaration(&mut self, string: &mut String, name: &str, type_name: &str) {
		Self::emit_value_type(string, type_name);
		string.push(' ');
		Self::identifier(name).push_to(string);
	}
	fn emit_function_call(&mut self, string: &mut String, function: &besl::NodeId, parameters: &[besl::NodeId]) -> bool {
		let program = self.program;
		let function_node = &program[function];

		// An array constructor has the array type for its name. Metal has no `float4[3](...)` constructor
		// syntax, so the elements initialize a `metal::array` of that type instead.
		if let Some(type_name) = program.name(*function)
			&& crate::shader::generator::value_array_parts(type_name).is_some()
		{
			Self::emit_value_type(string, type_name);
			string.push('{');
			self.emit_call_arguments(string, parameters);
			string.push('}');
			return true;
		}

		let besl::Nodes::Struct {
			name,
			fields,
			template: None,
			..
		} = function_node.node()
		else {
			return false;
		};
		if crate::shader::generator::is_builtin_struct_type(program.text(*name)) {
			return false;
		}

		// Metal user structs are aggregates, so their portable BESL constructors lower to brace initialization.
		Self::identifier(program.text(*name)).push_to(string);
		string.push('{');
		for (index, parameter) in parameters.iter().enumerate() {
			if index > 0 {
				self.emit_separator(string);
			}
			if fields.get(index).is_some_and(|field| self.is_packed_mat4x3_member(field)) {
				string.push_str("_besl_pack_mat4x3(");
				self.emit_node_string(string, parameter);
				string.push(')');
			} else if let Some(elements) = crate::shader::generator::array_constructor_elements(program, parameter) {
				// Struct fields stay C arrays so buffer layouts don't change, and a C array only initializes from braces.
				string.push('{');
				self.emit_call_arguments(string, &elements);
				string.push('}');
			} else {
				self.emit_node_string(string, parameter);
			}
		}
		string.push('}');
		true
	}
	fn emit_expression_override(&mut self, string: &mut String, expression: &besl::Expressions) -> bool {
		let program = self.program;
		let besl::Expressions::Operator { operator, left, right } = expression else {
			return false;
		};
		if *operator == besl::Operators::Multiply {
			return self.emit_packed_vector_matrix_product(string, left, right);
		}
		if *operator != besl::Operators::Assignment || !self.expression_is_packed_mat4x3_accessor(left) {
			return false;
		}

		let left = &program[left];
		let besl::Nodes::Expression(besl::Expressions::Accessor { left, right: target }) = left.node() else {
			return false;
		};
		string.push_str("_besl_store_mat4x3(");
		self.emit_accessor_expression_raw(string, left, target);
		self.emit_separator(string);
		self.emit_node_string(string, right);
		string.push(')');
		true
	}
	fn emit_expression_member(&mut self, string: &mut String, name: besl::Name, source: &besl::NodeId) -> bool {
		let program = self.program;
		match program[source].node() {
			// Outside a stage context, a binding keeps its plain name.
			besl::Nodes::Binding { .. }
				if self.raster_stage_context.is_some()
					|| self.in_compute_body
					|| self.mesh_stage_context.is_some()
					|| self.ray_tracing_stage_context.is_some() =>
			{
				self.emit_binding_reference(string, name);
				return true;
			}
			besl::Nodes::TaskPayload { .. } => {
				string.push_str("payload.");
				Self::identifier(program.text(name)).push_to(string);
				return true;
			}
			besl::Nodes::Workgroup { .. } => {
				Self::identifier(program.text(name)).push_to(string);
				return true;
			}
			_ => {}
		}
		false
	}
	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeId, right: &besl::NodeId) {
		if self.accessor_returns_packed_mat4x3(left, right) {
			string.push_str("_besl_load_mat4x3(");
			self.emit_accessor_expression_raw(string, left, right);
			string.push(')');
		} else {
			self.emit_accessor_expression_raw(string, left, right);
		}
	}
	fn emit_node(&mut self, string: &mut String, node: &besl::NodeId) {
		self.emit_node_string(string, node)
	}
}
