use super::super::*;

/// The `ResolvedBufferAccess` struct carries a validated packed-memory target into instruction lowering.
pub(super) struct ResolvedBufferAccess {
	pub(super) slot: ResourceSlot,
	pub(super) offset: usize,
	pub(super) stride: usize,
	/// `None` defers bounds to the byte length of the buffer bound at execution time.
	pub(super) count: Option<usize>,
	pub(super) index_expression: Option<NodeId>,
	pub(super) value_type: ValueType,
}

/// The `LoweredBufferAccess` struct carries the single compiled index register used by a memory instruction.
pub(super) struct LoweredBufferAccess {
	pub(super) slot: ResourceSlot,
	pub(super) offset: usize,
	pub(super) stride: usize,
	pub(super) count: Option<usize>,
	pub(super) index: Option<usize>,
	pub(super) value_type: ValueType,
}

pub(super) struct ResolvedTaskPayloadAccess {
	pub(super) name: String,
	pub(super) index_expression: NodeId,
	pub(super) count: usize,
	pub(super) value_type: ValueType,
}

/// The `ResolvedWorkgroupAccess` struct carries one typed workgroup value into instruction lowering.
pub(super) struct ResolvedWorkgroupAccess {
	pub(super) name: String,
	pub(super) index_expression: Option<NodeId>,
	pub(super) count: usize,
	pub(super) value_type: ValueType,
}

/// Resolves a workgroup reference without confusing it with descriptor-backed memory.
pub(super) fn resolve_workgroup_access(
	program: &Program,
	expression: &NodeId,
) -> Result<Option<ResolvedWorkgroupAccess>, VmError> {
	let (workgroup, index_expression) = {
		let borrowed = &program[expression];
		match borrowed.node() {
			Nodes::Expression(Expressions::Accessor { left, right }) => {
				let Some(workgroup) = extract_workgroup_reference(program, left) else {
					return Ok(None);
				};
				(workgroup, Some(*right))
			}
			_ => {
				let Some(workgroup) = extract_workgroup_reference(program, expression) else {
					return Ok(None);
				};
				(workgroup, None)
			}
		}
	};
	let workgroup = &program[workgroup];
	let Nodes::Workgroup {
		name,
		format,
		count: declared_count,
	} = workgroup.node()
	else {
		unreachable!(
			"Invalid resolved workgroup reference. The most likely cause is that workgroup reference extraction returned a different node kind."
		)
	};
	if index_expression.is_some() && declared_count.is_none() {
		return Ok(None);
	}
	Ok(Some(ResolvedWorkgroupAccess {
		name: program.text(*name).to_string(),
		index_expression,
		count: declared_count.map_or(1, |count| count.get()),
		value_type: resolve_value_type(program, format)?,
	}))
}

/// Peels expression wrappers around a directly referenced workgroup declaration.
pub(super) fn extract_workgroup_reference(program: &Program, expression: &NodeId) -> Option<NodeId> {
	let borrowed = &program[expression];
	match borrowed.node() {
		Nodes::Workgroup { .. } => Some(*expression),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			extract_workgroup_reference(program, &elements[0])
		}
		Nodes::Expression(Expressions::Member { source, .. }) if matches!(program[source].node(), Nodes::Workgroup { .. }) => {
			Some(*source)
		}
		_ => None,
	}
}

pub(super) fn resolve_task_payload_access(
	program: &Program,
	expression: &NodeId,
) -> Result<Option<ResolvedTaskPayloadAccess>, VmError> {
	let (left, index_expression) = {
		let borrowed = &program[expression];
		let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
			return Ok(None);
		};
		(*left, *right)
	};

	let Some(payload) = extract_task_payload_reference(program, &left) else {
		return Ok(None);
	};
	let payload = &program[payload];
	let Nodes::TaskPayload { name, format, count } = payload.node() else {
		unreachable!("Task-payload references are validated before resolving their layout")
	};
	Ok(Some(ResolvedTaskPayloadAccess {
		name: program.text(*name).to_string(),
		index_expression,
		count: count.get(),
		value_type: resolve_value_type(program, format)?,
	}))
}

pub(super) fn extract_task_payload_reference(program: &Program, expression: &NodeId) -> Option<NodeId> {
	let borrowed = &program[expression];
	match borrowed.node() {
		Nodes::TaskPayload { .. } => Some(*expression),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			extract_task_payload_reference(program, &elements[0])
		}
		Nodes::Expression(Expressions::Member { source, .. })
			if matches!(program[source].node(), Nodes::TaskPayload { .. }) =>
		{
			Some(*source)
		}
		_ => None,
	}
}

/// The `FunctionParameter` struct links one lexical parameter identity to its portable VM value type.
pub(super) struct FunctionParameter {
	pub(super) node: NodeId,
	pub(super) value_type: ValueType,
}

/// The `FunctionSignature` struct supplies parameter, return, and body information while lowering function calls.
pub(super) struct FunctionSignature {
	pub(super) params: Vec<FunctionParameter>,
	pub(super) return_type: Option<ValueType>,
	pub(super) statements: Vec<NodeId>,
}

#[derive(Clone, Copy)]
pub(super) enum RequiredAccess {
	Read,
	Write,
	ReadWrite,
	Any,
}

impl RequiredAccess {
	pub(super) const fn requires_read(self) -> bool {
		matches!(self, Self::Read | Self::ReadWrite)
	}

	pub(super) const fn requires_write(self) -> bool {
		matches!(self, Self::Write | Self::ReadWrite)
	}
}

/// Validates one binding's declared access at the shared descriptor-resolution seam.
pub(super) fn require_descriptor_access(
	slot: ResourceSlot,
	readable: bool,
	writable: bool,
	required: RequiredAccess,
) -> Result<(), VmError> {
	if required.requires_read() && !readable {
		return Err(VmError::DescriptorAccessDenied { slot, access: "read" });
	}
	if required.requires_write() && !writable {
		return Err(VmError::DescriptorAccessDenied { slot, access: "write" });
	}
	Ok(())
}

/// Resolves untyped comparison literals from their typed peer and rejects incompatible operands before lowering.
pub(super) fn resolve_comparison_operand_types(
	program: &Program,
	left: &NodeId,
	right: &NodeId,
	mut left_type: ValueType,
	mut right_type: ValueType,
) -> Result<(ValueType, ValueType), VmError> {
	let left_is_literal = is_literal_expression(program, left);
	let right_is_literal = is_literal_expression(program, right);
	if left_is_literal && !right_is_literal {
		left_type = right_type.clone();
	} else if right_is_literal && !left_is_literal {
		right_type = left_type.clone();
	}

	if left_type != right_type {
		return Err(VmError::TypeMismatch {
			expected: left_type.name().to_string(),
			found: right_type.name().to_string(),
		});
	}

	Ok((left_type, right_type))
}

pub(super) fn is_literal_expression(program: &Program, expression: &NodeId) -> bool {
	let borrowed = &program[expression];
	match borrowed.node() {
		Nodes::Expression(Expressions::Literal { .. }) => true,
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			is_literal_expression(program, &elements[0])
		}
		_ => false,
	}
}

pub(super) fn extract_function_signature(program: &Program, function: &NodeId) -> Result<FunctionSignature, VmError> {
	let function_ref = &program[function];
	let (params, return_type, statements) = match function_ref.node() {
		Nodes::Function {
			params,
			return_type,
			statements,
			..
		} => (params.clone(), *return_type, statements.clone()),
		node => {
			return Err(VmError::UnsupportedExpression {
				message: format!("Expected a function, but found {}", describe_node(node)),
			});
		}
	};

	let mut compiled_params = Vec::with_capacity(params.len());
	for param in params {
		let param_ref = &program[param];
		let value_type = match param_ref.node() {
			Nodes::Parameter { r#type, .. } => resolve_value_type(program, r#type)?,
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a parameter, but found {}", describe_node(node)),
				});
			}
		};
		compiled_params.push(FunctionParameter { node: param, value_type });
	}

	let return_type = resolve_function_return_type(program, &return_type)?;
	Ok(FunctionSignature {
		params: compiled_params,
		return_type,
		statements,
	})
}

pub(super) fn resolve_function_return_type(program: &Program, return_type: &NodeId) -> Result<Option<ValueType>, VmError> {
	if program.name(*return_type) == Some("void") {
		Ok(None)
	} else {
		Ok(Some(resolve_value_type(program, return_type)?))
	}
}

pub(super) fn resolve_callable_return_type(program: &Program, callable: &NodeId) -> Result<ValueType, VmError> {
	let callable_ref = &program[callable];
	match callable_ref.node() {
		Nodes::Struct { .. } => resolve_value_type(program, callable),
		Nodes::Intrinsic { r#return, .. } => {
			let return_type = *r#return;
			resolve_value_type(program, &return_type)
		}
		Nodes::Function { return_type, .. } => {
			let return_type = *return_type;
			resolve_function_return_type(program, &return_type)?.ok_or_else(|| VmError::UnsupportedExpression {
				message: "Void functions cannot be used as value expressions".to_string(),
			})
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a callable value, but found {}", describe_node(node)),
		}),
	}
}

pub(super) fn resolve_value_type(program: &Program, node: &NodeId) -> Result<ValueType, VmError> {
	let node_ref = &program[node];
	let type_name = program.name(*node).unwrap_or("unknown");
	// Atomic storage types hold plain integers in VM memory.
	let builtin = match type_name {
		"atomicu32" => Some(ValueType::U32),
		"atomici32" => Some(ValueType::I32),
		name => ValueType::builtin(name),
	};
	if let Some(value_type) = builtin {
		return Ok(value_type);
	}

	let Nodes::Struct { fields, .. } = node_ref.node() else {
		return Err(VmError::UnsupportedType {
			type_name: type_name.to_string(),
		});
	};
	let (fields, size, alignment) = compile_member_layouts(program, fields, false)?;
	Ok(ValueType::Struct {
		name: type_name.to_string(),
		fields,
		size,
		alignment,
	})
}

pub(super) fn is_resource_type(value_type: &ValueType) -> bool {
	matches!(
		value_type,
		ValueType::Texture2D
			| ValueType::Texture3D
			| ValueType::TextureCube
			| ValueType::TextureCubeArray
			| ValueType::ArrayTexture2D
	)
}

pub(super) fn compile_buffer_layout(program: &Program, members: &[NodeId]) -> Result<BufferLayout, VmError> {
	let (compiled_members, size, _) = compile_member_layouts(program, members, true)?;

	Ok(BufferLayout {
		members: compiled_members,
		size,
		element: None,
		element_count: None,
	})
}

/// Compiles the host-visible layout of one array-buffer element and the array's fixed count, if any.
///
/// Struct elements keep one layout member per field. Scalar and vector elements have no members; hosts address them
/// with [`crate::vm::Buffer::write_array_element`].
pub(super) fn compile_buffer_array_layout(
	program: &Program,
	element: &NodeId,
	count: Option<std::num::NonZeroUsize>,
) -> Result<(BufferLayout, ValueType), VmError> {
	let unsupported = || VmError::UnsupportedBufferLayout {
		message:
			"Unsupported buffer array element. The most likely cause is that its type is empty, boolean, or a resource handle."
				.to_string(),
	};
	let value_type = resolve_value_type(program, element)?;
	let mut layout = match program[element].node() {
		Nodes::Struct { fields, .. } if matches!(value_type, ValueType::Struct { .. }) => {
			compile_buffer_layout(program, fields)?
		}
		_ if is_resource_type(&value_type) || matches!(value_type, ValueType::Bool) => return Err(unsupported()),
		_ => BufferLayout::scalar_element(value_type.clone()),
	};
	if layout.size() == 0 {
		return Err(unsupported());
	}
	if let Some(count) = count {
		// Validating the total here lets `Buffer::new` allocate fixed arrays without a fallible size.
		layout.size().checked_mul(count.get()).ok_or_else(|| {
			VmError::UnsupportedBufferLayout {
			message: "Buffer array exceeds addressable CPU memory. The most likely cause is that its fixed element count is too large."
				.to_string(),
		}
		})?;
		layout.element_count = Some(count.get());
	}
	Ok((layout, value_type))
}

/// Places `members` with the scalar layout the GPU backends use: each member starts on its type's alignment, and the
/// size is rounded up to the largest alignment. Returns the member layouts, the size, and that alignment.
pub(super) fn compile_member_layouts(
	program: &Program,
	members: &[NodeId],
	allow_array_members: bool,
) -> Result<(Vec<BufferMemberLayout>, usize, usize), VmError> {
	let mut offset: usize = 0;
	let mut alignment = 1;
	let mut compiled_members = Vec::with_capacity(members.len());
	for member in members {
		let member = &program[member];
		match member.node() {
			Nodes::Member { name, r#type, count } => {
				let name = program.text(*name);
				// Aggregate `Value` instances do not represent nested arrays, so only outer buffer layouts may retain counts.
				if count.is_some() && !allow_array_members {
					return Err(VmError::UnsupportedBufferLayout {
						message: format!("Struct field `{}` cannot be an array", name),
					});
				}
				let value_type = resolve_value_type(program, r#type)?;
				if is_resource_type(&value_type) {
					return Err(VmError::UnsupportedBufferLayout {
						message: format!("Buffer member `{}` cannot contain resource handles", name),
					});
				}
				if value_type == ValueType::Bool {
					return Err(VmError::UnsupportedBufferLayout {
						message: format!(
							"Member `{name}` is an unpacked `bool`. The most likely cause is a program that was not linked with `besl::lex`, which packs `bool` members into bytes; link it with `besl::lex` or `besl::compile_to_besl`."
						),
					});
				}
				offset = offset.next_multiple_of(value_type.alignment());
				alignment = alignment.max(value_type.alignment());
				let count = count.map(std::num::NonZeroUsize::get).unwrap_or(1);
				let member_size = value_type
					.size()
					.checked_mul(count)
					.ok_or_else(|| VmError::UnsupportedBufferLayout {
						message: format!("Buffer member `{}` exceeds addressable CPU memory", name),
					})?;
				compiled_members.push(BufferMemberLayout {
					name: name.to_string(),
					offset,
					value_type: value_type.clone(),
					count,
				});
				offset = offset
					.checked_add(member_size)
					.ok_or_else(|| VmError::UnsupportedBufferLayout {
						message: format!("Buffer member `{}` exceeds addressable CPU memory", name),
					})?;
			}
			node => {
				return Err(VmError::UnsupportedBufferLayout {
					message: format!("Unsupported buffer member node: {}", describe_node(node)),
				});
			}
		}
	}
	Ok((compiled_members, offset.next_multiple_of(alignment), alignment))
}

pub(super) enum AccessSelector {
	Member(String),
	Index(NodeId),
}

pub(super) fn extract_access_chain(program: &Program, expression: &NodeId) -> Result<(NodeId, Vec<AccessSelector>), VmError> {
	let borrowed = &program[expression];
	match borrowed.node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			let inner = elements[0];
			extract_access_chain(program, &inner)
		}
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			let left = *left;
			let selector = match program[right].node() {
				Nodes::Expression(Expressions::Member { name, .. }) => AccessSelector::Member(program.text(*name).to_string()),
				_ => AccessSelector::Index(*right),
			};
			let (binding, mut selectors) = extract_access_chain(program, &left)?;
			selectors.push(selector);
			Ok((binding, selectors))
		}
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = *source;
			if matches!(program[source].node(), Nodes::Binding { .. } | Nodes::PushConstant { .. }) {
				Ok((source, Vec::new()))
			} else {
				Err(VmError::UnsupportedExpression {
					message: "Accessor is not rooted in a buffer binding".to_string(),
				})
			}
		}
		Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok((*expression, Vec::new())),
		Nodes::Expression(expression) => Err(VmError::UnsupportedExpression {
			message: format!(
				"Expected a buffer accessor, but found {}",
				match expression {
					Expressions::Return { .. } => "return",
					Expressions::Continue => "continue",
					Expressions::Break => "break",
					Expressions::Discard => "discard",
					Expressions::Member { .. } => "member",
					Expressions::Expression { .. } => "multi-element expression group",
					Expressions::Literal { .. } => "literal",
					Expressions::FunctionCall { .. } => "function call",
					Expressions::IntrinsicCall { .. } => "intrinsic call",
					Expressions::Operator { .. } => "operator",
					Expressions::Unary { .. } => "prefix operator",
					Expressions::Ternary { .. } => "ternary",
					Expressions::VariableDeclaration { .. } => "variable declaration",
					Expressions::Accessor { .. } => "accessor",
					Expressions::Macro { .. } => "macro",
				}
			),
		}),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a buffer accessor, but found {}", describe_node(node)),
		}),
	}
}

pub(super) fn accessor_references_buffer(program: &Program, expression: &NodeId) -> bool {
	extract_access_chain(program, expression)
		.ok()
		.is_some_and(|(binding, _)| matches!(program[binding].node(), Nodes::Binding { .. } | Nodes::PushConstant { .. }))
}

/// Resolves one sampled-texture descriptor element using the binding's declared flat slot range.
pub(super) fn resolve_texture_array_access(
	program: &Program,
	expression: &NodeId,
) -> Result<Option<(ResourceSlot, usize, NodeId, ValueType)>, VmError> {
	let borrowed = &program[expression];
	let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
		return Ok(None);
	};
	let Ok(binding) = extract_binding_reference(program, left) else {
		return Ok(None);
	};
	let binding = &program[binding];
	let Nodes::Binding {
		name,
		placement,
		read,
		write,
		r#type: BindingTypes::CombinedImageSampler { format, .. },
		count: Some(count),
		..
	} = binding.node()
	else {
		return Ok(None);
	};
	let slot = placed_slot(program.text(*name), *placement)?;
	require_descriptor_access(slot, *read, *write, RequiredAccess::Read)?;
	slot.slot()
		.checked_add(count.get() - 1)
		.ok_or_else(|| VmError::UnsupportedDescriptor {
			slot,
			message: "The descriptor array exceeds the flat resource slot range".to_string(),
		})?;
	let value_type = match format.as_str() {
		"" | "Texture2D" => ValueType::Texture2D,
		"Texture3D" => ValueType::Texture3D,
		"TextureCube" => ValueType::TextureCube,
		"TextureCubeArray" => ValueType::TextureCubeArray,
		"ArrayTexture2D" => ValueType::ArrayTexture2D,
		_ => {
			return Err(VmError::UnsupportedType {
				type_name: format.clone(),
			});
		}
	};
	Ok(Some((slot, count.get() as usize, *right, value_type)))
}

/// Resolves one storage-image descriptor element, `images[index]`, using the binding's declared flat slot range.
///
/// Returns the binding, the array's first slot, its element count, and the index expression. The caller checks
/// the access.
/// The most layers of one `StorageImageArray` the VM can address, enough for the six faces of a cube. The VM checks a
/// layer index against it, because it doesn't know the bound image's layer count when it compiles the shader.
const VM_IMAGE_LAYERS: u32 = 6;

pub(super) fn resolve_image_array_access(
	program: &Program,
	expression: &NodeId,
) -> Result<Option<(NodeId, ResourceSlot, usize, NodeId)>, VmError> {
	let borrowed = &program[expression];
	let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
		return Ok(None);
	};
	let Ok(binding) = extract_binding_reference(program, left) else {
		return Ok(None);
	};
	let (slot, count) = match program[binding].node() {
		Nodes::Binding {
			name,
			placement,
			r#type: BindingTypes::Image { .. },
			count: Some(count),
			..
		} => (placed_slot(program.text(*name), *placement)?, count.get()),
		// The VM holds each layer of a `StorageImageArray` as its own 2D image, at the binding's slot plus the layer,
		// the way it holds the elements of an image array.
		Nodes::Binding {
			name,
			placement,
			r#type: BindingTypes::Image { layered: true, .. },
			count: None,
			..
		} => (placed_slot(program.text(*name), *placement)?, VM_IMAGE_LAYERS),
		_ => return Ok(None),
	};
	slot.slot()
		.checked_add(count - 1)
		.ok_or_else(|| VmError::UnsupportedDescriptor {
			slot,
			message: "The descriptor array exceeds the flat resource slot range".to_string(),
		})?;
	Ok(Some((binding, slot, count as usize, *right)))
}

pub(super) fn accessor_references_output(program: &Program, expression: &NodeId) -> bool {
	let borrowed = &program[expression];
	match borrowed.node() {
		Nodes::Expression(Expressions::Accessor { left, .. }) => crate::referenced_output(program, left).is_some(),
		_ => false,
	}
}

pub(super) fn extract_binding_reference(program: &Program, expression: &NodeId) -> Result<NodeId, VmError> {
	let borrowed = &program[expression];
	match borrowed.node() {
		Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok(*expression),
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = *source;

			match program[source].node() {
				Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok(source),
				Nodes::Expression(Expressions::Member { .. }) => extract_binding_reference(program, &source),
				_ => Err(VmError::UnsupportedExpression {
					message: format!(
						"Only direct binding or push constant member access is supported, but found {}",
						describe_node(program[source].node())
					),
				}),
			}
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!(
				"Expected a binding or push constant reference, but found {}",
				describe_node(node)
			),
		}),
	}
}

pub(super) fn extract_member_name(program: &Program, expression: &NodeId) -> Result<String, VmError> {
	let borrowed = &program[expression];
	match borrowed.node() {
		Nodes::Expression(Expressions::Member { name, .. }) => Ok(program.text(*name).to_string()),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a buffer member name, but found {}", describe_node(node)),
		}),
	}
}

pub(super) fn aggregate_member(value_type: &ValueType, member_name: &str) -> Result<(usize, ValueType), VmError> {
	match value_type {
		ValueType::Struct { fields, .. } => fields
			.iter()
			.enumerate()
			.find(|(_, field)| field.name() == member_name)
			.map(|(index, field)| (index, field.value_type().clone()))
			.ok_or_else(|| VmError::UnknownBufferMember {
				member: member_name.to_string(),
			}),
		ValueType::Vec2U16 | ValueType::Vec2I | ValueType::Vec2U | ValueType::Vec2F16 | ValueType::Vec2F => {
			vector_member(value_type, member_name, 2)
		}
		ValueType::Vec3U | ValueType::Vec3F16 | ValueType::Vec3F => vector_member(value_type, member_name, 3),
		ValueType::Vec4U16 | ValueType::Vec4U | ValueType::Vec4F16 | ValueType::Vec4F => {
			vector_member(value_type, member_name, 4)
		}
		ValueType::Mat4F => matrix_member(member_name, ValueType::Vec4F),
		ValueType::Mat4x3F => matrix_member(member_name, ValueType::Vec3F),
		_ => Err(VmError::UnsupportedExpression {
			message: format!("`{}` has no selectable members", value_type.name()),
		}),
	}
}

pub(super) fn array_element_type(value_type: &ValueType) -> Result<(ValueType, usize), VmError> {
	// An index selects one component of an arithmetic vector, as `words[i]` does in every shading language.
	let vector_width = match value_type {
		ValueType::Mat4F => return Ok((ValueType::Vec4F, 4)),
		ValueType::Mat4x3F => return Ok((ValueType::Vec3F, 4)),
		ValueType::Vec2U16 | ValueType::Vec2I | ValueType::Vec2U | ValueType::Vec2F16 | ValueType::Vec2F => Some(2),
		ValueType::Vec3U | ValueType::Vec3F16 | ValueType::Vec3F => Some(3),
		ValueType::Vec4U16 | ValueType::Vec4U | ValueType::Vec4F16 | ValueType::Vec4F => Some(4),
		_ => None,
	};
	if let Some(width) = vector_width
		&& let Some(scalar) = vector_scalar_type(value_type)
	{
		return Ok((scalar, width));
	}
	let ValueType::Struct { fields, .. } = value_type else {
		return Err(VmError::UnsupportedExpression {
			message: format!("`{}` cannot be indexed as an aggregate value", value_type.name()),
		});
	};
	let first = fields.first().ok_or_else(|| VmError::UnsupportedExpression {
		message: "Cannot index an empty aggregate value".to_string(),
	})?;
	if fields.iter().enumerate().any(|(index, field)| {
		field
			.name()
			.strip_prefix("value_")
			.and_then(|suffix| suffix.parse::<usize>().ok())
			!= Some(index)
			|| field.value_type() != first.value_type()
	}) {
		return Err(VmError::UnsupportedExpression {
			message: format!("`{}` is a struct, not an indexable array value", value_type.name()),
		});
	}
	Ok((first.value_type().clone(), fields.len()))
}

pub(super) fn aggregate_member_layout(value_type: &ValueType, member_name: &str) -> Result<(usize, ValueType, usize), VmError> {
	let (index, field_type) = aggregate_member(value_type, member_name)?;
	let offset = match value_type {
		ValueType::Struct { fields, .. } => fields[index].offset(),
		ValueType::Mat4F | ValueType::Mat4x3F => index * field_type.size(),
		_ => index * field_type.size(),
	};
	let count = match value_type {
		ValueType::Struct { fields, .. } => fields[index].count(),
		_ => 1,
	};
	Ok((offset, field_type, count))
}

pub(super) fn vector_member(
	value_type: &ValueType,
	member_name: &str,
	component_count: usize,
) -> Result<(usize, ValueType), VmError> {
	let index = component_index(member_name)
		.filter(|index| *index < component_count)
		.ok_or_else(|| VmError::UnsupportedExpression {
			message: format!("`{}` is not a component of `{}`", member_name, value_type.name()),
		})?;
	let scalar = vector_scalar_type(value_type).expect("Vector types have scalar components");
	Ok((index, scalar))
}

pub(super) fn matrix_member(member_name: &str, column_type: ValueType) -> Result<(usize, ValueType), VmError> {
	let index = component_index(member_name).ok_or_else(|| VmError::UnsupportedExpression {
		message: format!("`{}` is not a matrix column", member_name),
	})?;
	Ok((index, column_type))
}

pub(super) fn component_index(name: &str) -> Option<usize> {
	match name {
		"x" | "r" => Some(0),
		"y" | "g" => Some(1),
		"z" | "b" => Some(2),
		"w" | "a" => Some(3),
		_ => None,
	}
}

pub(super) fn resolve_referenced_value_type(program: &Program, source: &NodeId) -> Result<ValueType, VmError> {
	match program[source].node() {
		Nodes::Member { r#type, .. }
		| Nodes::Parameter { r#type, .. }
		| Nodes::Specialization { r#type, .. }
		| Nodes::Const { r#type, .. } => resolve_value_type(program, r#type),
		Nodes::Input { format, .. } | Nodes::Output { format, .. } => resolve_value_type(program, format),
		Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => resolve_value_type(program, r#type),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Cannot resolve a value type from {}", describe_node(node)),
		}),
	}
}

pub(super) fn describe_node(node: &Nodes) -> &'static str {
	match node {
		Nodes::Scope { .. } => "scope",
		Nodes::Use { .. } => "use declaration",
		Nodes::Layer { .. } => "layer",
		Nodes::Signature { .. } => "signature",
		Nodes::Struct { .. } => "struct",
		Nodes::Member { .. } => "member",
		Nodes::Function { .. } => "function",
		Nodes::Conditional { .. } => "conditional",
		Nodes::Match { .. } => "match",
		Nodes::ForLoop { .. } => "for loop",
		Nodes::Specialization { .. } => "specialization",
		Nodes::Expression(_) => "expression",
		Nodes::Binding { .. } => "binding",
		Nodes::PushConstant { .. } => "push constant",
		Nodes::Intrinsic { .. } => "intrinsic",
		Nodes::Input { .. } => "input",
		Nodes::Output { .. } => "output",
		Nodes::TaskPayload { .. } => "task payload",
		Nodes::Workgroup { .. } => "workgroup storage",
		Nodes::Parameter { .. } => "parameter",
		Nodes::Const { .. } => "const",
	}
}

/// Returns the slot of the first descriptor of binding `name`, from its placement among the program's sets.
pub(super) fn placed_slot(name: &str, placement: Option<crate::Placement>) -> Result<ResourceSlot, VmError> {
	placement.map(ResourceSlot::placed).ok_or_else(|| VmError::UnsupportedExpression {
		message: format!(
			"Descriptor `{name}` belongs to no descriptor set of the program. The most likely cause is a descriptor of a layer that `main` doesn't take. Take the layer as a parameter of `main`"
		),
	})
}
