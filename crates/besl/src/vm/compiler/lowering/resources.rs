use super::*;

impl<'a> Compiler<'a> {
	pub(super) fn resolve_memory_access(
		&self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let (binding, selectors) = extract_access_chain(expression)?;
		let Some(AccessSelector::Member(member_name)) = selectors.first() else {
			return Err(VmError::UnsupportedExpression {
				message: "Buffer access must select a named member first".to_string(),
			});
		};

		let binding_ref = binding.borrow();
		let (slot, layout) = match binding_ref.node() {
			Nodes::Binding {
				slot,
				read,
				write,
				r#type,
				..
			} => {
				let slot = ResourceSlot::new(*slot);
				require_descriptor_access(slot, *read, *write, access)?;
				let layout = match r#type {
					BindingTypes::Buffer { members } => compile_buffer_layout(members)?,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only buffer descriptors are supported".to_string(),
						});
					}
				};

				(slot, layout)
			}
			Nodes::PushConstant { members } => {
				if access.requires_write() {
					return Err(VmError::UnsupportedAssignmentTarget {
						message: "Push constant members are read-only".to_string(),
					});
				}

				(PUSH_CONSTANT_SLOT, compile_buffer_layout(members)?)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		let descriptor_layout = if slot == PUSH_CONSTANT_SLOT {
			DescriptorLayout::PushConstant(layout.clone())
		} else {
			DescriptorLayout::Buffer(layout.clone())
		};

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &descriptor_layout => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, descriptor_layout);
			}
		}

		let member = layout.member(member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.clone(),
		})?;
		let mut offset = member.offset();
		let mut current_stride = member.value_type().size();
		let mut current_count = member.count();
		let mut value_type = member.value_type().clone();
		let mut index = None;
		let mut indexed_stride = current_stride;
		let mut indexed_count = current_count;
		for selector in selectors.iter().skip(1) {
			match selector {
				AccessSelector::Index(index_expression) => {
					if index.is_some() {
						return Err(VmError::UnsupportedExpression {
							message: format!("Buffer member `{}` cannot use more than one dynamic index", member_name),
						});
					}
					indexed_stride = current_stride;
					indexed_count = current_count;
					index = Some(index_expression.clone());
					current_count = 1;
				}
				AccessSelector::Member(field_name) => {
					if current_count > 1 {
						return Err(VmError::UnsupportedExpression {
							message: format!("Buffer member `{}` is an array and requires an element index", member_name),
						});
					}
					let (field_offset, field_type, field_count) = aggregate_member_layout(&value_type, field_name)?;
					offset += field_offset;
					value_type = field_type;
					current_stride = value_type.size();
					current_count = field_count;
				}
			}
		}
		if current_count > 1 {
			return Err(VmError::UnsupportedExpression {
				message: format!("Buffer member `{}` is an array and requires an element index", member_name),
			});
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset,
			stride: indexed_stride,
			count: indexed_count,
			index_expression: index,
			value_type,
		})
	}

	pub(super) fn resolve_texture_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResourceSlot, VmError> {
		let binding = match extract_binding_reference(expression) {
			Ok(binding) => binding,
			Err(_) => {
				let value_type = self.infer_expression_type(expression, &ValueType::Texture2D, descriptor_layouts)?;
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
				let register = self.compile_value_expression(expression, &value_type, descriptor_layouts)?;
				return Ok(dynamic_resource_slot(register));
			}
		};

		let binding_ref = binding.borrow();
		let slot = match binding_ref.node() {
			Nodes::Binding {
				slot,
				read,
				write,
				r#type,
				..
			} => {
				let slot = ResourceSlot::new(*slot);
				require_descriptor_access(slot, *read, *write, access)?;
				match r#type {
					BindingTypes::CombinedImageSampler { .. } => slot,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only texture descriptors can be sampled or fetched".to_string(),
						});
					}
				}
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Texture => Err(VmError::UnsupportedDescriptor {
				slot,
				message: "Descriptor slot was reused with a different layout".to_string(),
			}),
			Some(_) => Ok(slot),
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Texture);
				Ok(slot)
			}
		}
	}

	pub(super) fn resolve_image_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResourceSlot, VmError> {
		let binding = match extract_binding_reference(expression) {
			Ok(binding) => binding,
			Err(_) => {
				let value_type = self.infer_expression_type(expression, &ValueType::Texture2D, descriptor_layouts)?;
				if value_type != ValueType::Texture2D {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Texture2D.name().to_string(),
						found: value_type.name().to_string(),
					});
				}
				let register = self.compile_value_expression(expression, &value_type, descriptor_layouts)?;
				return Ok(dynamic_resource_slot(register));
			}
		};

		let binding_ref = binding.borrow();
		let slot = match binding_ref.node() {
			Nodes::Binding {
				slot,
				read,
				write,
				r#type,
				..
			} => {
				let slot = ResourceSlot::new(*slot);
				require_descriptor_access(slot, *read, *write, access)?;
				match r#type {
					BindingTypes::Image { .. } => slot,
					_ => {
						return Err(VmError::UnsupportedDescriptor {
							slot,
							message: "Only image descriptors can be written through `write`".to_string(),
						});
					}
				}
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Image => Err(VmError::UnsupportedDescriptor {
				slot,
				message: "Descriptor slot was reused with a different layout".to_string(),
			}),
			Some(_) => Ok(slot),
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Image);
				Ok(slot)
			}
		}
	}

	pub(super) fn resolve_output_access(
		&self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let borrowed = expression.borrow();
		let (source, output_name) = match borrowed.node() {
			Nodes::Expression(Expressions::Member { source, name }) => (source.clone(), name.clone()),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an output member access, but found {}", describe_node(node)),
				});
			}
		};
		drop(borrowed);

		let source_ref = source.borrow();
		let (slot, layout) = match source_ref.node() {
			Nodes::Output {
				name,
				format,
				location,
				count,
			} => {
				if name != &output_name {
					return Err(VmError::UnsupportedExpression {
						message: format!("Only direct output assignment is supported for `{}`", output_name),
					});
				}

				let value_type = resolve_value_type(format)?;
				let count = count.map(std::num::NonZeroUsize::get).unwrap_or(1);
				(
					if output_name == "position" {
						builtin_position_slot()
					} else {
						output_slot(*location)
					},
					BufferLayout {
						members: vec![BufferMemberLayout {
							name: output_name,
							offset: 0,
							value_type: value_type.clone(),
							count,
						}],
						size: value_type.size() * count,
					},
				)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an output interface, but found {}", describe_node(node)),
				});
			}
		};
		drop(source_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Buffer(layout.clone()) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Buffer(layout.clone()));
			}
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset: 0,
			stride: layout.members()[0].value_type().size(),
			count: layout.members()[0].count(),
			index_expression: None,
			value_type: layout.members()[0].value_type().clone(),
		})
	}

	/// Resolves one dynamically indexed mesh output-array write.
	pub(super) fn resolve_output_array_access(
		&self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let (left, index_expression) = {
			let borrowed = expression.borrow();
			let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
				return Err(VmError::UnsupportedAssignmentTarget {
					message: "Expected an indexed output array".to_string(),
				});
			};
			(left.clone(), right.clone())
		};
		let mut target = self.resolve_output_access(&left, descriptor_layouts)?;
		target.index_expression = Some(index_expression);
		Ok(target)
	}

	pub(super) fn resolve_input_access(
		&self,
		expression: &NodeReference,
		descriptor_layouts: &mut HashMap<ResourceSlot, DescriptorLayout>,
	) -> Result<ResolvedBufferAccess, VmError> {
		let borrowed = expression.borrow();
		let (source, input_name) = match borrowed.node() {
			Nodes::Expression(Expressions::Member { source, name }) => (source.clone(), name.clone()),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an input member access, but found {}", describe_node(node)),
				});
			}
		};
		drop(borrowed);

		let source_ref = source.borrow();
		let (slot, layout) = match source_ref.node() {
			Nodes::Input { name, format, location } => {
				if name != &input_name {
					return Err(VmError::UnsupportedExpression {
						message: format!("Only direct input reads are supported for `{}`", input_name),
					});
				}

				let value_type = resolve_value_type(format)?;
				(
					input_slot(*location),
					BufferLayout {
						members: vec![BufferMemberLayout {
							name: input_name,
							offset: 0,
							value_type: value_type.clone(),
							count: 1,
						}],
						size: value_type.size(),
					},
				)
			}
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an input interface, but found {}", describe_node(node)),
				});
			}
		};
		drop(source_ref);

		match descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Buffer(layout.clone()) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				descriptor_layouts.insert(slot, DescriptorLayout::Buffer(layout.clone()));
			}
		}

		Ok(ResolvedBufferAccess {
			slot,
			offset: 0,
			stride: layout.size(),
			count: 1,
			index_expression: None,
			value_type: layout.members()[0].value_type().clone(),
		})
	}
}
