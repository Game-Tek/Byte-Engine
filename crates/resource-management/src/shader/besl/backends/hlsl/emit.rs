use super::*;
impl<'p> Generator<'p> {
	pub(crate) fn emit_object_payload_struct(&self, string: &mut String) {
		if !self.task_payloads.is_empty() {
			self.emit_task_payload_struct(string, "ObjectPayload", &self.task_payloads);
		}
	}

	/// Emits the fixed vertex output and the authored per-vertex and per-primitive mesh outputs.
	pub(crate) fn emit_mesh_output_structs(&self, string: &mut String) {
		let formatting = ShaderFormatting::new(self.minified);
		self.emit_named_struct_start(string, "VertexOutput");
		formatting.push_indentation(string, 1);
		string.push_str("float4 position : SV_Position");
		formatting.push_statement_end(string);
		self.emit_mesh_output_fields(string, true);
		self.emit_struct_declaration_end(string);

		self.emit_named_struct_start(string, "PrimitiveOutput");
		if self.mesh_uses_render_target_array_index {
			formatting.push_indentation(string, 1);
			string.push_str("uint32_t render_target_array_index : SV_RenderTargetArrayIndex");
			formatting.push_statement_end(string);
		}
		self.emit_mesh_output_fields(string, false);
		self.emit_struct_declaration_end(string);
	}

	/// Emits the mesh output arrays of one rate as fields of their native vertex or primitive struct.
	fn emit_mesh_output_fields(&self, string: &mut String, vertex_rate: bool) {
		let program = self.program;
		let formatting = ShaderFormatting::new(self.minified);
		for output in &self.mesh_outputs {
			let output = &program[output];
			let besl::Nodes::Output {
				name,
				location,
				format,
				count: Some(_),
				per_vertex,
			} = output.node()
			else {
				continue;
			};
			if *per_vertex != vertex_rate {
				continue;
			}

			formatting.push_indentation(string, 1);
			let format = &program[format];
			let besl_type = program.text(format.get_name().unwrap());
			let type_name = Self::translate_type(besl_type);
			if is_integer_besl_type(besl_type) {
				string.push_str("nointerpolation ");
			}
			let _ = write!(
				string,
				"{} {} : TEXCOORD{location}",
				type_name,
				Self::identifier(program.text(*name))
			);
			formatting.push_statement_end(string);
		}
	}

	/// Emits raster stage I/O as mutable entry-point parameters because HLSL semantic globals are immutable.
	pub(crate) fn emit_raster_entry_parameters(&self, string: &mut String, mut has_previous_parameter: bool) {
		let program = self.program;
		// The vertex or mesh shader writes the position first, so a pixel shader declares it first too, read or not, and
		// every varying after it lines up with the register the previous stage wrote it to.
		if matches!(self.stage, Stages::Fragment) {
			if has_previous_parameter {
				self.emit_separator(string);
			}
			string.push_str("float4 _besl_fragment_position : SV_Position");
			has_previous_parameter = true;
		}
		for input in &self.raster_inputs {
			let input = &program[input];
			let besl::Nodes::Input {
				name, location, format, ..
			} = input.node()
			else {
				continue;
			};
			if has_previous_parameter {
				self.emit_separator(string);
			}
			let format = &program[format];
			let besl_type = program.text(format.get_name().unwrap());
			let type_name = Self::translate_type(besl_type);
			if matches!(self.stage, Stages::Vertex) && crate::shader::generator::is_vertex_builtin_input(program.text(*name)) {
				let _ = write!(string, "{} {}", type_name, Self::identifier(program.text(*name)));
				string.push_str(match program.text(*name) {
					besl::VERTEX_INDEX_BUILTIN => " : SV_VertexID",
					besl::INSTANCE_INDEX_BUILTIN => " : SV_InstanceID",
					_ => unreachable!("Expected a validated vertex builtin"),
				});
				has_previous_parameter = true;
				continue;
			}
			if matches!(self.stage, Stages::Fragment) && program.text(*name) == besl::PRIMITIVE_INDEX_BUILTIN {
				let _ = write!(
					string,
					"{} {} : SV_PrimitiveID",
					type_name,
					Self::identifier(program.text(*name))
				);
				has_previous_parameter = true;
				continue;
			}
			if self.stage.interpolates_inputs() && is_integer_besl_type(besl_type) {
				string.push_str("nointerpolation ");
			}
			let _ = write!(
				string,
				"{} {} : TEXCOORD{location}",
				type_name,
				Self::identifier(program.text(*name))
			);
			has_previous_parameter = true;
		}

		for output in &self.raster_outputs {
			let output = &program[output];
			let besl::Nodes::Output {
				name,
				location,
				format,
				count: None,
				..
			} = output.node()
			else {
				continue;
			};
			if has_previous_parameter {
				self.emit_separator(string);
			}
			let format = &program[format];
			let besl_type = program.text(format.get_name().unwrap());
			let type_name = Self::translate_type(besl_type);
			let vertex = matches!(self.stage, Stages::Vertex);
			let layer = vertex && besl::is_render_target_array_index_output(program.text(*name));
			// The layer is a system value the rasterizer reads, never interpolates, so it takes no interpolation mode.
			if !layer && self.stage.interpolates_outputs() && is_integer_besl_type(besl_type) {
				string.push_str("nointerpolation ");
			}
			string.push_str("out ");
			let _ = write!(string, "{} {}", type_name, Self::identifier(program.text(*name)));
			if vertex && besl::is_position_output(program.text(*name)) {
				string.push_str(" : SV_Position");
			} else if layer {
				string.push_str(" : SV_RenderTargetArrayIndex");
			} else {
				let semantic = if matches!(self.stage, Stages::Fragment) {
					"SV_Target"
				} else {
					"TEXCOORD"
				};
				let _ = write!(string, " : {semantic}{location}");
			}
			has_previous_parameter = true;
		}
	}

	/// Emits the subscript that picks one texture or sampler of a descriptor array.
	pub(crate) fn emit_descriptor_array_index(&mut self, string: &mut String, index: &besl::NodeId) {
		// Any expression may pick the element, so lanes of one wave can disagree, which D3D12 leaves undefined unless the
		// index is marked non-uniform.
		string.push_str("[NonUniformResourceIndex(");
		self.emit_node_string(string, index);
		string.push_str(")]");
	}

	/// Emits the sampler paired with a texture argument. Inside a descriptor array it shares the texture's index.
	/// Emits the texel of `image` at `coord` that a load reads or a store writes: `image[coord]`, or
	/// `image[uint3(coord, layer)]` for a layer of a `StorageImageArray`.
	pub(crate) fn emit_image_texel(&mut self, string: &mut String, image: &besl::NodeId, coord: &besl::NodeId) {
		match besl::layered_image_access(self.program, image) {
			Some((image, layer)) => {
				self.emit_node_string(string, &image);
				string.push_str("[uint3(");
				self.emit_node_string(string, coord);
				string.push_str(", ");
				self.emit_node_string(string, &layer);
				string.push_str(")]");
			}
			None => {
				self.emit_node_string(string, image);
				string.push('[');
				self.emit_node_string(string, coord);
				string.push(']');
			}
		}
	}

	pub(crate) fn emit_sampler(&mut self, string: &mut String, texture: &besl::NodeId) {
		let program = self.program;
		let accessor = resource_accessor(program, texture);
		self.emit_node_string(string, accessor.as_ref().map_or(texture, |(_, resource, _)| resource));
		string.push_str("_sampler");
		if let Some((ResourceAccessorKind::DescriptorArray, _, index)) = &accessor {
			self.emit_descriptor_array_index(string, index);
		}
	}
}
