use super::super::*;

impl<A: Allocator + Clone> Generator<A> {
	/// Emits a resource passed to an intrinsic using the active stage's resource context.
	pub(crate) fn emit_intrinsic_resource_reference(&mut self, string: &mut String, resource: &besl::NodeReference) {
		let resource_node = resource.borrow();
		if let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = resource_node.node() {
			if self.in_compute_body || self.task_stage_context.is_some() {
				self.emit_compute_binding_reference(string, name);
			} else {
				self.emit_raster_binding_reference(string, name);
			}
			return;
		}
		if let besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) = resource_node.node() {
			let left = left.borrow();
			if let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = left.node() {
				if self.in_compute_body || self.task_stage_context.is_some() {
					self.emit_compute_binding_reference(string, name);
				} else {
					self.emit_raster_binding_reference(string, name);
				}
				return;
			}
		}
		drop(resource_node);
		self.emit_node_string(string, resource);
	}

	pub(crate) fn emit_texture_2d_array_grad_sample(
		&mut self,
		string: &mut String,
		texture_array: &besl::NodeReference,
		texture_index: &besl::NodeReference,
		uv: &besl::NodeReference,
		uv_derivative_x: &besl::NodeReference,
		uv_derivative_y: &besl::NodeReference,
	) {
		self.emit_intrinsic_resource_reference(string, texture_array);
		string.push('[');
		self.emit_node_string(string, texture_index);
		string.push_str("].sample(");
		self.emit_intrinsic_resource_reference(string, texture_array);
		string.push_str("_sampler[");
		self.emit_node_string(string, texture_index);
		string.push_str("],");
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv);
		string.push_str(", metal::gradient2d(");
		self.emit_node_string(string, uv_derivative_x);
		string.push(',');
		if !self.minified {
			string.push(' ');
		}
		self.emit_node_string(string, uv_derivative_y);
		string.push_str("))");
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
			..
		} = intrinsic.node()
		else {
			for element in elements {
				self.emit_node_string(string, element);
			}
			return;
		};

		match name.as_str() {
			"sample" => {
				if let Some((kind, resource, index)) = resource_accessor(&arguments[0]) {
					self.emit_intrinsic_resource_reference(string, &resource);
					if kind == ResourceAccessorKind::DescriptorArray {
						string.push('[');
						self.emit_node_string(string, &index);
						string.push(']');
					}
					string.push_str(".sample(");
					self.emit_intrinsic_resource_reference(string, &resource);
					string.push_str("_sampler");
					if kind == ResourceAccessorKind::DescriptorArray {
						string.push('[');
						self.emit_node_string(string, &index);
						string.push(']');
					}
					string.push_str(", ");
					self.emit_node_string(string, &arguments[1]);
					if kind == ResourceAccessorKind::Texture2DArrayLayer {
						string.push_str(", ");
						self.emit_node_string(string, &index);
					}
					string.push(')');
					return;
				}
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".sample(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
				return;
			}
			"sample_texture_2d_array_grad" => {
				self.emit_texture_2d_array_grad_sample(
					string,
					&arguments[0],
					&arguments[1],
					&arguments[2],
					&arguments[3],
					&arguments[4],
				);
				return;
			}
			"texture_lod" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".sample(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
				self.emit_node_string(string, &arguments[1]);
				// Qualify the Metal helper so BESL identifiers such as `level` cannot shadow it.
				string.push_str(", metal::level(");
				if let Some(lod) = arguments.get(2) {
					self.emit_node_string(string, lod);
				} else {
					string.push_str("0.0");
				}
				string.push_str("))");
				return;
			}
			"texture_cube_array_lod" => {
				self.emit_node_string(string, &arguments[0]);
				string.push_str(".sample(");
				self.emit_node_string(string, &arguments[0]);
				string.push_str("_sampler, ");
				self.emit_node_string(string, &arguments[1]);
				string.push_str(", ");
				self.emit_node_string(string, &arguments[2]);
				string.push_str(", metal::level(");
				self.emit_node_string(string, &arguments[3]);
				string.push_str("))");
				return;
			}
			"downsample_min" | "downsample_max" => {
				if self.downsample_strategy == DownsampleStrategy::ShaderGather {
					string.push_str(if name == "downsample_min" {
						"_besl_downsample_min("
					} else {
						"_besl_downsample_max("
					});
					self.emit_node_string(string, &arguments[0]);
					string.push_str(", ");
					self.emit_node_string(string, &arguments[0]);
					string.push_str("_sampler, ");
					self.emit_node_string(string, &arguments[1]);
					string.push_str(", ");
					if arguments.len() == 4 {
						self.emit_node_string(string, &arguments[2]);
						string.push_str(", ");
						self.emit_node_string(string, &arguments[3]);
					} else {
						self.emit_node_string(string, &arguments[2]);
					}
					string.push(')');
				} else {
					self.emit_node_string(string, &arguments[0]);
					string.push_str(".sample(");
					self.emit_node_string(string, &arguments[0]);
					string.push_str("_sampler, ");
					self.emit_node_string(string, &arguments[1]);
					if arguments.len() == 4 {
						string.push_str(", ");
						self.emit_node_string(string, &arguments[2]);
					}
					string.push_str(", metal::level(");
					self.emit_node_string(string, &arguments[if arguments.len() == 4 { 3 } else { 2 }]);
					string.push_str(")).x");
				}
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
			"pow" if arguments.len() == 2 && super::super::super::is_two(&arguments[0]) => {
				string.push_str("exp2(");
				self.emit_node_string(string, &arguments[1]);
				string.push(')');
			}
			"min" | "max" | "clamp" | "log2" | "pow" | "abs" | "sqrt" | "exp" | "sin" | "cos" | "tan" | "asin" | "atan2"
			| "floor" | "round" | "fract" | "fwidth" | "step" | "smoothstep" | "mix" => {
				string.push_str(name);
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"fma" => {
				string.push_str("fma(");
				self.emit_call_arguments(string, arguments);
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
			"f16" => {
				string.push_str("half(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" | "packed_vec4f" => {
				string.push_str(Self::translate_type(name));
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"u32" => {
				string.push_str("uint(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"u16" => {
				string.push_str("ushort(");
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
			"atomic_compare_exchange" => {
				string.push_str("_besl_atomic_compare_exchange(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
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
			"subgroup_lane_index" => string.push_str("simd_lane_id"),
			"threadgroup_position" => {
				string.push_str("threadgroup_position");
			}
			"subgroup_ballot" => {
				string.push_str("_besl_subgroup_ballot(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_any" => {
				string.push_str("_besl_subgroup_ballot_any(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_find_lsb" => {
				string.push_str("_besl_subgroup_ballot_find_lsb(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_count" => {
				string.push_str("_besl_subgroup_ballot_count(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_ballot_and_not" => {
				string.push_str("_besl_subgroup_ballot_and_not(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_broadcast_u32" => {
				string.push_str("_besl_subgroup_broadcast_u32(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			"subgroup_broadcast_f32" => {
				string.push_str("_besl_subgroup_broadcast_f32(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
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
				if let Some(layer) = arguments.get(2) {
					if self.minified {
						string.push(',');
					} else {
						string.push_str(", ");
					}
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
}
