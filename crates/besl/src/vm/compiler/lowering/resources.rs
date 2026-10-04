use super::*;

impl<'a> Compiler<'a> {
	pub(super) fn resolve_memory_access(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
	) -> Result<ResolvedBufferAccess, VmError> {
		let (binding, selectors) = extract_access_chain(expression)?;

		let (slot, layout, runtime_element_type) = match binding.borrow().node() {
			Nodes::PushConstant { members } => {
				if access.requires_write() {
					return Err(VmError::UnsupportedAssignmentTarget {
						message: "Push constant members are read-only".to_string(),
					});
				}

				(PUSH_CONSTANT_SLOT, compile_buffer_layout(members)?, None)
			}
			node => match accessible_binding(node, access)? {
				(slot, BindingTypes::Buffer { members }) => (slot, compile_buffer_layout(members)?, None),
				(slot, BindingTypes::BufferArray { element, fixed }) => {
					let count = fixed.as_ref().map(|fixed| fixed.count);
					let (layout, element_type) = compile_buffer_array_layout(element, count)?;
					(slot, layout, Some(element_type))
				}
				(slot, _) => {
					return Err(VmError::UnsupportedDescriptor {
						slot,
						message: "Only buffer descriptors are supported".to_string(),
					});
				}
			},
		};

		self.claim_descriptor_layout(
			slot,
			if slot == PUSH_CONSTANT_SLOT {
				DescriptorLayout::PushConstant(layout.clone())
			} else {
				DescriptorLayout::Buffer(layout.clone())
			},
			"layout",
		)?;

		resolve_buffer_access(slot, &layout, runtime_element_type, &selectors)
	}

	pub(super) fn resolve_texture_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
	) -> Result<ResourceSlot, VmError> {
		let Ok(binding) = extract_binding_reference(expression) else {
			let value_type = self.infer_expression_type(expression, &ValueType::Texture2D)?;
			if !matches!(
				value_type,
				ValueType::Texture2D
					| ValueType::Texture3D
					| ValueType::TextureCube
					| ValueType::TextureCubeArray
					| ValueType::ArrayTexture2D
			) {
				return Err(VmError::TypeMismatch {
					expected: "texture resource".to_string(),
					found: value_type.name().to_string(),
				});
			}
			let register = self.compile_value_expression(expression, &value_type)?;
			return Ok(dynamic_resource_slot(register));
		};

		let slot = match accessible_binding(binding.borrow().node(), access)? {
			(slot, BindingTypes::CombinedImageSampler { .. }) => slot,
			(slot, _) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Only texture descriptors can be sampled or fetched".to_string(),
				});
			}
		};
		self.claim_descriptor_layout(slot, DescriptorLayout::Texture, "layout")?;
		Ok(slot)
	}

	/// Resolves `array_texture[layer]` without treating the layer as a descriptor-array index.
	pub(super) fn resolve_array_texture_layer_access(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
	) -> Result<Option<(ResourceSlot, NodeReference)>, VmError> {
		let (texture, layer) = {
			let borrowed = expression.borrow();
			let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
				return Ok(None);
			};
			(left.clone(), right.clone())
		};
		let Ok(binding) = extract_binding_reference(&texture) else {
			return Ok(None);
		};
		let is_layered_texture = matches!(
			binding.borrow().node(),
			Nodes::Binding {
				r#type: BindingTypes::CombinedImageSampler { format },
				count: None,
				..
			} if format == "ArrayTexture2D"
		);
		if !is_layered_texture {
			return Ok(None);
		}

		let slot = self.resolve_texture_slot(&texture, access)?;
		Ok(Some((slot, layer)))
	}

	pub(super) fn resolve_image_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
	) -> Result<ResourceSlot, VmError> {
		let Ok(binding) = extract_binding_reference(expression) else {
			let value_type = self.infer_expression_type(expression, &ValueType::Texture2D)?;
			if value_type != ValueType::Texture2D {
				return Err(type_mismatch(&ValueType::Texture2D, &value_type));
			}
			let register = self.compile_value_expression(expression, &value_type)?;
			return Ok(dynamic_resource_slot(register));
		};

		let slot = match accessible_binding(binding.borrow().node(), access)? {
			(slot, BindingTypes::Image { .. }) => slot,
			(slot, _) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Only image descriptors can be written through `write`".to_string(),
				});
			}
		};
		self.claim_descriptor_layout(slot, DescriptorLayout::Image, "layout")?;
		Ok(slot)
	}

	pub(super) fn resolve_output_access(&mut self, expression: &NodeReference) -> Result<ResolvedBufferAccess, VmError> {
		let (source, output_name) = match expression.borrow().node() {
			Nodes::Expression(Expressions::Member { source, name }) => (source.clone(), name.clone()),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an output member access, but found {}", describe_node(node)),
				});
			}
		};

		let (slot, value_type, count) = match source.borrow().node() {
			// The VM stores per-vertex and per-primitive mesh outputs alike, as plain arrays tests read back.
			Nodes::Output {
				name,
				format,
				location,
				count,
				per_vertex: _,
			} => {
				if name != &output_name {
					return Err(VmError::UnsupportedExpression {
						message: format!("Only direct output assignment is supported for `{}`", output_name),
					});
				}

				let slot = if crate::is_position_output(&output_name) {
					builtin_position_slot()
				} else {
					output_slot(*location)
				};
				(
					slot,
					resolve_value_type(format)?,
					count.map_or(1, std::num::NonZeroUsize::get),
				)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an output interface, but found {}", describe_node(node)),
				});
			}
		};

		self.interface_access(slot, output_name, value_type, count)
	}

	/// Resolves one dynamically indexed mesh output-array write.
	pub(super) fn resolve_output_array_access(&mut self, expression: &NodeReference) -> Result<ResolvedBufferAccess, VmError> {
		let (left, index_expression) = {
			let borrowed = expression.borrow();
			let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
				return Err(VmError::UnsupportedAssignmentTarget {
					message: "Expected an indexed output array".to_string(),
				});
			};
			(left.clone(), right.clone())
		};
		let mut target = self.resolve_output_access(&left)?;
		target.index_expression = Some(index_expression);
		Ok(target)
	}

	pub(super) fn resolve_input_access(&mut self, expression: &NodeReference) -> Result<ResolvedBufferAccess, VmError> {
		let (source, input_name) = match expression.borrow().node() {
			Nodes::Expression(Expressions::Member { source, name }) => (source.clone(), name.clone()),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an input member access, but found {}", describe_node(node)),
				});
			}
		};

		let (slot, value_type) = match source.borrow().node() {
			Nodes::Input { name, format, location } => {
				if name != &input_name {
					return Err(VmError::UnsupportedExpression {
						message: format!("Only direct input reads are supported for `{}`", input_name),
					});
				}

				let value_type = resolve_value_type(format)?;
				let slot = match name.as_str() {
					crate::VERTEX_INDEX_BUILTIN => builtin_vertex_index_slot(),
					crate::INSTANCE_INDEX_BUILTIN => builtin_instance_index_slot(),
					_ => input_slot(*location),
				};
				(slot, value_type)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an input interface, but found {}", describe_node(node)),
				});
			}
		};

		self.interface_access(slot, input_name, value_type, 1)
	}

	/// Declares the one-member buffer the VM keeps for an input or output interface at `slot` and returns its access.
	fn interface_access(
		&mut self,
		slot: ResourceSlot,
		name: String,
		value_type: ValueType,
		count: usize,
	) -> Result<ResolvedBufferAccess, VmError> {
		let stride = value_type.size();
		let layout = BufferLayout {
			members: vec![BufferMemberLayout {
				name,
				offset: 0,
				value_type: value_type.clone(),
				count,
			}],
			size: stride * count,
			element: None,
			element_count: None,
		};
		self.claim_descriptor_layout(slot, DescriptorLayout::Buffer(layout), "layout")?;

		Ok(ResolvedBufferAccess {
			slot,
			offset: 0,
			stride,
			count: Some(count),
			index_expression: None,
			value_type,
		})
	}

	/// Records `layout` for `slot`, or rejects it when an earlier access gave the slot a different layout, so every
	/// access to one slot agrees on what it holds. `conflict` names what differs in the rejection message.
	pub(super) fn claim_descriptor_layout(
		&mut self,
		slot: ResourceSlot,
		layout: DescriptorLayout,
		conflict: &str,
	) -> Result<(), VmError> {
		match self.descriptor_layouts.get(&slot) {
			Some(existing) if *existing != layout => Err(VmError::UnsupportedDescriptor {
				slot,
				message: format!("Descriptor slot was reused with a different {conflict}"),
			}),
			Some(_) => Ok(()),
			None => {
				self.descriptor_layouts.insert(slot, layout);
				Ok(())
			}
		}
	}
}

/// Returns the slot and resource type of a binding node after checking that the binding allows `access`.
fn accessible_binding(node: &Nodes, access: RequiredAccess) -> Result<(ResourceSlot, &BindingTypes), VmError> {
	let Nodes::Binding {
		slot,
		read,
		write,
		r#type,
		..
	} = node
	else {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected a binding access, but found {}", describe_node(node)),
		});
	};
	let slot = ResourceSlot::new(*slot);
	require_descriptor_access(slot, *read, *write, access)?;
	Ok((slot, r#type))
}

/// Resolves selectors rooted at either a fixed buffer member or a runtime buffer element.
fn resolve_buffer_access(
	slot: ResourceSlot,
	layout: &BufferLayout,
	runtime_element_type: Option<ValueType>,
	selectors: &[AccessSelector],
) -> Result<ResolvedBufferAccess, VmError> {
	let (mut resolved, mut current_stride, mut current_count) = if let Some(value_type) = runtime_element_type {
		let Some(AccessSelector::Index(index_expression)) = selectors.first() else {
			return Err(VmError::UnsupportedExpression {
				message: "Runtime-sized buffer access must select an element first".to_string(),
			});
		};
		(
			ResolvedBufferAccess {
				slot,
				offset: 0,
				stride: layout.size(),
				count: None,
				index_expression: Some(index_expression.clone()),
				value_type,
			},
			layout.size(),
			1,
		)
	} else {
		let Some(AccessSelector::Member(member_name)) = selectors.first() else {
			return Err(VmError::UnsupportedExpression {
				message: "Buffer access must select a named member first".to_string(),
			});
		};
		let member = layout.member(member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.clone(),
		})?;
		(
			ResolvedBufferAccess {
				slot,
				offset: member.offset(),
				stride: member.value_type().size(),
				count: Some(member.count()),
				index_expression: None,
				value_type: member.value_type().clone(),
			},
			member.value_type().size(),
			member.count(),
		)
	};

	for selector in selectors.iter().skip(1) {
		match selector {
			AccessSelector::Index(index_expression) => {
				if resolved.index_expression.is_some() {
					return Err(VmError::UnsupportedExpression {
						message: "Buffer access cannot use more than one dynamic index".to_string(),
					});
				}
				resolved.stride = current_stride;
				resolved.count = Some(current_count);
				resolved.index_expression = Some(index_expression.clone());
				current_count = 1;
			}
			AccessSelector::Member(field_name) => {
				if current_count > 1 {
					return Err(VmError::UnsupportedExpression {
						message: "Buffer array requires an element index before member access".to_string(),
					});
				}
				let (field_offset, field_type, field_count) = aggregate_member_layout(&resolved.value_type, field_name)?;
				resolved.offset += field_offset;
				resolved.value_type = field_type;
				current_stride = resolved.value_type.size();
				current_count = field_count;
			}
		}
	}
	if current_count > 1 {
		return Err(VmError::UnsupportedExpression {
			message: "Buffer array requires an element index".to_string(),
		});
	}

	Ok(resolved)
}
