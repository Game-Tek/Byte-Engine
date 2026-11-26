use super::super::*;

impl<'p> Generator<'p> {
	/// Reports whether `function`, or a function it calls, writes a tile output or reads a tile value, all of which the
	/// kernel holds and passes to its helpers.
	pub(crate) fn uses_tile_context(&self, function: &besl::NodeId) -> bool {
		let program = self.program;
		any_code_node(program, function, true, &mut |node| match program[node].node() {
			besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
				intrinsic, arguments, ..
			}) => match program.name(*intrinsic) {
				Some("output_extent") => true,
				Some("write") => arguments
					.first()
					.and_then(|node| besl::referenced_output(program, node))
					.is_some(),
				_ => false,
			},
			_ => false,
		})
	}

	/// Writes the member name of the tile output that `argument` names.
	/// Emits the texture a storage image intrinsic acts on: the argument itself, or the whole `texture2d_array` when the
	/// argument selects a layer of a `StorageImageArray`, whose layer [`Self::emit_storage_image_layer`] passes.
	pub(crate) fn emit_storage_image(&mut self, string: &mut String, image: &besl::NodeId) {
		match besl::layered_image_access(self.program, image) {
			Some((image, _)) => self.emit_node_string(string, &image),
			None => self.emit_node_string(string, image),
		}
	}

	/// Appends the layer a `StorageImageArray` argument selects as the last argument of a Metal `read` or `write`, and
	/// nothing for any other image.
	pub(crate) fn emit_storage_image_layer(&mut self, string: &mut String, image: &besl::NodeId) {
		if let Some((_, layer)) = besl::layered_image_access(self.program, image) {
			self.emit_separator(string);
			self.emit_node_string(string, &layer);
		}
	}

	pub(crate) fn emit_output_name(&self, string: &mut String, argument: &besl::NodeId) {
		let program = self.program;
		if let Some(output) = besl::referenced_output(program, argument)
			&& let Some(name) = program.name(output)
		{
			Self::identifier(name).push_to(string);
		}
	}
}

impl<'p> Generator<'p> {
	/// Emits a resource passed to an intrinsic using the active stage's resource context.
	pub(crate) fn emit_intrinsic_resource_reference(&mut self, string: &mut String, resource: &besl::NodeId) {
		let program = self.program;
		let resource_node = &program[resource];
		if let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = resource_node.node() {
			self.emit_binding_reference(string, *name);
			return;
		}
		if let besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) = resource_node.node() {
			let left = &program[left];
			if let besl::Nodes::Expression(besl::Expressions::Member { name, .. }) = left.node() {
				self.emit_binding_reference(string, *name);
				return;
			}
		}
		self.emit_node_string(string, resource);
	}

	/// Emits the sampler paired with a texture argument. Inside a descriptor array it shares the texture's index, unless
	/// the shader fixes it, in which case every element reads through one `constexpr sampler`.
	pub(crate) fn emit_sampler(&mut self, string: &mut String, texture: &besl::NodeId) {
		let program = self.program;
		if emit_fixed_sampler_name(program, string, texture) {
			return;
		}
		let Some((kind, resource, index)) = resource_accessor(program, texture) else {
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
}

/// Writes the program-scope name of the `constexpr sampler` the shader fixes for a texture argument, through the index
/// or layer the argument selects, and returns whether the texture has one. See [`Generator::emit_static_samplers`].
pub(crate) fn emit_fixed_sampler_name(program: &besl::Program, string: &mut String, texture: &besl::NodeId) -> bool {
	match program[texture].node() {
		besl::Nodes::Binding {
			name,
			r#type: besl::BindingTypes::CombinedImageSampler { sampler: Some(_), .. },
			..
		} => {
			Generator::emit_static_sampler_name(string, program.text(*name));
			true
		}
		besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => emit_fixed_sampler_name(program, string, source),
		besl::Nodes::Expression(besl::Expressions::Accessor { left, .. }) => emit_fixed_sampler_name(program, string, left),
		_ => false,
	}
}
