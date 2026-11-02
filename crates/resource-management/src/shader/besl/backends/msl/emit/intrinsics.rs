use super::super::*;

impl Generator {
	/// Reports whether `function`, or a function it calls, writes a tile output or reads a tile value, all of which the
	/// kernel holds and passes to its helpers.
	pub(crate) fn uses_tile_context(function: &besl::NodeReference) -> bool {
		any_code_node(function, true, &mut |node| match node.borrow().node() {
			besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
				intrinsic, arguments, ..
			}) => match intrinsic.borrow().get_name() {
				Some("output_extent") => true,
				Some("write") => arguments.first().and_then(besl::referenced_output).is_some(),
				_ => false,
			},
			_ => false,
		})
	}

	/// Writes the member name of the tile output that `argument` names.
	fn emit_output_name(&self, string: &mut String, argument: &besl::NodeReference) {
		if let Some(output) = besl::referenced_output(argument)
			&& let Some(name) = output.borrow().get_name()
		{
			Self::identifier(name).push_to(string);
		}
	}
}

impl Generator {
	/// Emits a resource passed to an intrinsic using the active stage's resource context.
	pub(crate) fn emit_intrinsic_resource_reference(&mut self, string: &mut String, resource: &besl::NodeReference) {
		let resource_node = resource.borrow();
		if let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = resource_node.node() {
			self.emit_binding_reference(string, name);
			return;
		}
		if let besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) = resource_node.node() {
			let left = left.borrow();
			if let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = left.node() {
				self.emit_binding_reference(string, name);
				return;
			}
		}
		drop(resource_node);
		self.emit_node_string(string, resource);
	}

	/// Emits the sampler paired with a texture argument. Inside a descriptor array it shares the texture's index, unless
	/// the shader fixes it, in which case every element reads through one `constexpr sampler`.
	pub(crate) fn emit_sampler(&mut self, string: &mut String, texture: &besl::NodeReference) {
		if emit_fixed_sampler_name(string, texture) {
			return;
		}
		let Some((kind, resource, index)) = resource_accessor(texture) else {
			self.emit_node_string(string, texture);
			string.push_str("_sampler");
			return;
		};
		self.emit_intrinsic_resource_reference(string, &resource);
		string.push_str("_sampler");
		if kind == ResourceAccessorKind::DescriptorArray {
			string.push('[');
			self.emit_node_string(string, &index);
			string.push(']');
		}
	}

	// Keep the intrinsic table contiguous because each arm defines one exact Metal lowering contract.
	#[allow(clippy::too_many_lines)]
	pub(crate) fn emit_intrinsic_call(
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
			r#return: return_type,
		} = intrinsic.node()
		else {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		};

		let has_body = definition
			.iter()
			.any(|element| !matches!(element.borrow().node(), besl::Nodes::Parameter { .. }));
		match name.as_str() {
			// Texture lowerings bypass intrinsic bodies.
			"sample" => {
				let accessor = resource_accessor(&arguments[0]);
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
			}
			"sample_texture_2d_array_grad" => {
				self.emit_intrinsic_resource_reference(string, &arguments[0]);
				string.push('[');
				self.emit_node_string(string, &arguments[1]);
				string.push_str("].sample(");
				if !emit_fixed_sampler_name(string, &arguments[0]) {
					self.emit_intrinsic_resource_reference(string, &arguments[0]);
					string.push_str("_sampler[");
					self.emit_node_string(string, &arguments[1]);
					string.push(']');
				}
				self.emit_separator(string);
				self.emit_node_string(string, &arguments[2]);
				string.push_str(", metal::gradient2d(");
				self.emit_node_string(string, &arguments[3]);
				self.emit_separator(string);
				self.emit_node_string(string, &arguments[4]);
				string.push_str("))");
			}
			// Metal reduces in a sampler only on Apple10 GPUs, so a min or max sampler reduces in shader code, through the
			// downsample helpers. The texture has one channel, which a sampler expands to `(value, 0, 0, 1)`.
			"texture_lod" if besl::lexer::binding_sampler(&arguments[0]).is_some_and(|sampler| sampler.reduces()) => {
				let helper = match besl::lexer::binding_sampler(&arguments[0]).map(|sampler| sampler.reduction) {
					Some(besl::sampler::Reduction::Min) => "_besl_downsample_min",
					_ => "_besl_downsample_max",
				};
				let _ = write!(string, "float4({helper}(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(", ");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", ");
				match arguments.get(2) {
					Some(lod) => self.emit_node_string(string, lod),
					None => string.push_str("0.0"),
				}
				string.push_str("), 0.0, 0.0, 1.0)");
			}
			"texture_lod" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".sample(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[1]);
				// Qualify the Metal helper so BESL identifiers such as `level` cannot shadow it.
				string.push_str(", metal::level(");
				if let Some(lod) = arguments.get(2) {
					self.emit_node_string(string, lod);
				} else {
					string.push_str("0.0");
				}
				string.push_str("))");
			}
			"texture_cube_array_lod" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".sample(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[2]);
				string.push_str(", metal::level(");
				self.emit_node_string(string, &arguments[3]);
				string.push_str("))");
			}
			"gather" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".gather(");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[1]);
				if let Some(layer) = arguments.get(2) {
					string.push_str(", ");
					self.emit_node_string(string, layer);
				}
				string.push_str(", int2(0), component::x)");
			}
			// The helpers gather and reduce in shader code; see `generate_msl_header_block`.
			"downsample_min" | "downsample_max" => {
				let _ = write!(string, "_besl_{name}(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(", ");
				self.emit_sampler(string, &arguments[0]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[2]);
				if arguments.len() == 4 {
					string.push_str(", ");
					self.emit_node_string(string, &arguments[3]);
				}
				string.push(')');
			}
			// Ray tracing stage values are kernel parameters, or members of the hit a closest hit shades. See
			// `ray_tracing.rs`.
			"launch_index" => string.push_str(LAUNCH_INDEX_ARGUMENT),
			"launch_size" => string.push_str(LAUNCH_SIZE_ARGUMENT),
			"hit_distance"
			| "hit_custom_index"
			| "hit_geometry_index"
			| "hit_primitive_index"
			| "hit_barycentrics"
			| "ray_origin"
			| "ray_direction" => self.emit_hit_value(string, name),
			// The helper runs the closest hit of the hit's record through the kernel's shader binding table, and passes it
			// the addresses of the kernel's resources.
			"invoke" => {
				string.push_str("_besl_invoke(");
				string.push_str(HITS_ARGUMENT);
				self.emit_separator(string);
				string.push_str(CONTEXT_ARGUMENT);
				self.emit_separator(string);
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			// Every other intrinsic with a body emits its expansion.
			_ if has_body => {
				for element in elements {
					self.emit_node_string(string, element);
				}
			}
			"pow" if arguments.len() == 2 && super::super::super::is_two(&arguments[0]) => {
				string.push_str("exp2(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			// BESL integer literals are unsigned, but C++ spells them as `int`, so `min(uint, 3)` matches no Metal
			// overload exactly. Casting every argument selects the unsigned overload the lexer resolved.
			"min" | "max" | "clamp" if return_type.borrow().get_name() == Some("u32") => {
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
				string.push(')');
			}
			"sincos" => {
				string.push_str("_besl_sincos(");
				self.emit_node_string(string, &arguments[0]);
				string.push(')');
			}
			"round_to_i32" => {
				string.push_str("int2(round(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("))");
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
			"atomic_exchange" | "atomic_add" | "atomic_sub" | "atomic_min" | "atomic_max" | "atomic_and" | "atomic_or"
			| "atomic_xor" => {
				string.push_str(match name.as_str() {
					"atomic_exchange" => "atomic_exchange_explicit(&",
					"atomic_add" => "atomic_fetch_add_explicit(&",
					"atomic_sub" => "atomic_fetch_sub_explicit(&",
					"atomic_min" => "atomic_fetch_min_explicit(&",
					"atomic_max" => "atomic_fetch_max_explicit(&",
					"atomic_and" => "atomic_fetch_and_explicit(&",
					"atomic_or" => "atomic_fetch_or_explicit(&",
					"atomic_xor" => "atomic_fetch_xor_explicit(&",
					_ => unreachable!("Expected an atomic read-modify-write intrinsic"),
				});
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
			"thread_position" => string.push_str("thread_position"),
			"thread_id" => string.push_str("gid"),
			"thread_idx" => string.push_str("thread_index"),
			"subgroup_lane_index" => string.push_str("simd_lane_id"),
			"threadgroup_position" => {
				string.push_str("threadgroup_position");
				if self.compute_stage_context.is_some() {
					string.push_str(".x");
				}
			}
			"workgroup_barrier" => string.push_str("threadgroup_barrier(mem_flags::mem_threadgroup)"),
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
				// Materialize each argument once because Metal needs three index writes for one triangle.
				string.push_str("{uint _besl_triangle_index=");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(";uint3 _besl_triangle=");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(";out_mesh.set_index(_besl_triangle_index*3+0,_besl_triangle.x);out_mesh.set_index(_besl_triangle_index*3+1,_besl_triangle.y);out_mesh.set_index(_besl_triangle_index*3+2,_besl_triangle.z);}");
			}
			"set_mesh_primitive_render_target_array_index" => {
				string.push_str("out_mesh.set_primitive(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(", PrimitiveOutput{.render_target_array_index = ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str("})");
			}
			"image_load" | "image_load_u32" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(if name == "image_load_u32" { ").x" } else { ")" });
			}
			"fetch" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".read(");
				self.emit_node_string(string, &arguments[1]);
				if let Some(layer) = arguments.get(2) {
					self.emit_separator(string);
					self.emit_node_string(string, layer);
				}
				string.push(')');
			}
			"texture_size" | "image_size" => {
				string.push_str("uint2(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_width(),");
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".get_height())");
			}
			// A tile output lives in the tile's imageblock, which holds only this tile, so the write skips pixels outside
			// the tile and past the outputs' extent. With one output the pixel's struct is the output; with several, the
			// write keeps the pixel's other outputs.
			"write" if self.tile_stage_context.is_some() && besl::referenced_output(&arguments[0]).is_some() => {
				let tile = self.tile_stage_context.as_ref().map_or(1, |tile| tile.tile);
				let keeps_others = self.tile_stage_context.as_ref().is_some_and(|tile| tile.output_count > 1);
				string.push_str("{uint2 _besl_pixel=");
				self.emit_node_string(string, &arguments[1]);
				let _ = write!(
					string,
					";uint2 _besl_offset=_besl_pixel-threadgroup_position*{tile}u;if(all(_besl_offset<uint2({tile}u))&&all(_besl_pixel<output_extent)){{{TILE_OUTPUTS_STRUCT} _besl_outputs"
				);
				if keeps_others {
					string.push_str("=tile_block.read(ushort2(_besl_offset))");
				}
				string.push_str(";_besl_outputs.");
				self.emit_output_name(string, &arguments[0]);
				string.push('=');
				self.emit_node_string(string, &arguments[2]);
				string.push_str(";tile_block.write(_besl_outputs,ushort2(_besl_offset));}}");
			}
			"output_extent" => string.push_str("output_extent"),
			"write" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".write(");
				self.emit_node_string(string, &arguments[2]);
				self.emit_separator(string);
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
			// Every other intrinsic is a Metal call, renamed where Metal spells the operation differently.
			_ => {
				string.push_str(match name.as_str() {
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
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
		}
	}
}

/// Writes the program-scope name of the `constexpr sampler` the shader fixes for a texture argument, through the index
/// or layer the argument selects, and returns whether the texture has one. See [`Generator::emit_static_samplers`].
fn emit_fixed_sampler_name(string: &mut String, texture: &besl::NodeReference) -> bool {
	match texture.borrow().node() {
		besl::Nodes::Binding {
			name,
			r#type: besl::BindingTypes::CombinedImageSampler { sampler: Some(_), .. },
			..
		} => {
			Generator::emit_static_sampler_name(string, name);
			true
		}
		besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => emit_fixed_sampler_name(string, source),
		besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) => emit_fixed_sampler_name(string, left),
		_ => false,
	}
}
