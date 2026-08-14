use std::collections::HashSet;

use super::super::*;

/// The `ResolvedBufferAccess` struct carries a validated packed-memory target into instruction lowering.
pub(super) struct ResolvedBufferAccess {
	pub(super) slot: ResourceSlot,
	pub(super) offset: usize,
	pub(super) stride: usize,
	pub(super) count: usize,
	pub(super) index_expression: Option<NodeReference>,
	pub(super) value_type: ValueType,
}

/// The `LoweredBufferAccess` struct carries the single compiled index register used by a memory instruction.
pub(super) struct LoweredBufferAccess {
	pub(super) slot: ResourceSlot,
	pub(super) offset: usize,
	pub(super) stride: usize,
	pub(super) count: usize,
	pub(super) index: Option<usize>,
	pub(super) value_type: ValueType,
}

pub(super) struct ResolvedTaskPayloadAccess {
	pub(super) name: String,
	pub(super) index_expression: NodeReference,
	pub(super) count: usize,
	pub(super) value_type: ValueType,
}

/// The `ResolvedWorkgroupAccess` struct carries one typed workgroup value into instruction lowering.
pub(super) struct ResolvedWorkgroupAccess {
	pub(super) name: String,
	pub(super) index_expression: Option<NodeReference>,
	pub(super) count: usize,
	pub(super) value_type: ValueType,
}

/// Resolves a workgroup reference without confusing it with descriptor-backed memory.
pub(super) fn resolve_workgroup_access(expression: &NodeReference) -> Result<Option<ResolvedWorkgroupAccess>, VmError> {
	let (workgroup, index_expression) = {
		let borrowed = expression.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::Accessor { left, right }) => {
				let Some(workgroup) = extract_workgroup_reference(left) else {
					return Ok(None);
				};
				(workgroup, Some(right.clone()))
			}
			_ => {
				drop(borrowed);
				let Some(workgroup) = extract_workgroup_reference(expression) else {
					return Ok(None);
				};
				(workgroup, None)
			}
		}
	};
	let workgroup = workgroup.borrow();
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
		name: name.clone(),
		index_expression,
		count: declared_count.map_or(1, |count| count.get()),
		value_type: resolve_value_type(format)?,
	}))
}

/// Peels expression wrappers around a directly referenced workgroup declaration.
pub(super) fn extract_workgroup_reference(expression: &NodeReference) -> Option<NodeReference> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Workgroup { .. } => Some(expression.clone()),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			extract_workgroup_reference(&elements[0])
		}
		Nodes::Expression(Expressions::Member { source, .. }) if matches!(source.borrow().node(), Nodes::Workgroup { .. }) => {
			Some(source.clone())
		}
		_ => None,
	}
}

pub(super) fn resolve_task_payload_access(expression: &NodeReference) -> Result<Option<ResolvedTaskPayloadAccess>, VmError> {
	let (left, index_expression) = {
		let borrowed = expression.borrow();
		let Nodes::Expression(Expressions::Accessor { left, right }) = borrowed.node() else {
			return Ok(None);
		};
		(left.clone(), right.clone())
	};

	let Some(payload) = extract_task_payload_reference(&left) else {
		return Ok(None);
	};
	let payload = payload.borrow();
	let Nodes::TaskPayload { name, format, count } = payload.node() else {
		unreachable!("Task-payload references are validated before resolving their layout")
	};
	Ok(Some(ResolvedTaskPayloadAccess {
		name: name.clone(),
		index_expression,
		count: count.get(),
		value_type: resolve_value_type(format)?,
	}))
}

pub(super) fn extract_task_payload_reference(expression: &NodeReference) -> Option<NodeReference> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::TaskPayload { .. } => Some(expression.clone()),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			extract_task_payload_reference(&elements[0])
		}
		Nodes::Expression(Expressions::Member { source, .. })
			if matches!(source.borrow().node(), Nodes::TaskPayload { .. }) =>
		{
			Some(source.clone())
		}
		_ => None,
	}
}

/// The `FunctionParameter` struct links one lexical parameter identity to its portable VM value type.
pub(super) struct FunctionParameter {
	pub(super) node: NodeReference,
	pub(super) value_type: ValueType,
}

/// The `FunctionSignature` struct supplies parameter, return, and body information while lowering function calls.
pub(super) struct FunctionSignature {
	pub(super) params: Vec<FunctionParameter>,
	pub(super) return_type: Option<ValueType>,
	pub(super) statements: Vec<NodeReference>,
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
	left: &NodeReference,
	right: &NodeReference,
	mut left_type: ValueType,
	mut right_type: ValueType,
) -> Result<(ValueType, ValueType), VmError> {
	let left_is_literal = is_literal_expression(left);
	let right_is_literal = is_literal_expression(right);
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

pub(super) fn is_literal_expression(expression: &NodeReference) -> bool {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Literal { .. }) => true,
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => is_literal_expression(&elements[0]),
		_ => false,
	}
}

pub(super) fn resolve_main_function(program: &NodeReference) -> Result<NodeReference, VmError> {
	let function = {
		let node = program.borrow();
		match node.node() {
			Nodes::Function { name, .. } if name == "main" => Some(program.clone()),
			_ => None,
		}
	};

	if let Some(function) = function {
		return Ok(function);
	}

	program.get_main().ok_or(VmError::MissingMainFunction)
}

pub(super) fn collect_functions(main: &NodeReference) -> Vec<NodeReference> {
	let mut functions = Vec::new();
	let mut seen = HashSet::new();
	collect_reachable_function(main, &mut seen, &mut functions);
	functions
}

/// Adds one function and every function referenced by its executable expressions.
pub(super) fn collect_reachable_function(
	function: &NodeReference,
	seen: &mut HashSet<usize>,
	functions: &mut Vec<NodeReference>,
) {
	if !seen.insert(function.identity()) {
		return;
	}
	functions.push(function.clone());
	let statements = match function.borrow().node() {
		Nodes::Function { statements, .. } => statements.clone(),
		_ => return,
	};
	for statement in statements {
		collect_function_references(&statement, seen, functions);
	}
}

pub(super) fn collect_function_references(node: &NodeReference, seen: &mut HashSet<usize>, functions: &mut Vec<NodeReference>) {
	let (called_function, children) = {
		let borrowed = node.borrow();
		match borrowed.node() {
			Nodes::Conditional { condition, statements } => {
				let mut children = Vec::with_capacity(statements.len() + 1);
				children.push(condition.clone());
				children.extend(statements.iter().cloned());
				(None, children)
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let mut children = Vec::with_capacity(statements.len() + 3);
				children.extend([initializer.clone(), condition.clone(), update.clone()]);
				children.extend(statements.iter().cloned());
				(None, children)
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				(Some(function.clone()), parameters.clone())
			}
			Nodes::Expression(Expressions::IntrinsicCall { arguments, .. }) => (None, arguments.clone()),
			Nodes::Expression(Expressions::Operator { left, right, .. })
			| Nodes::Expression(Expressions::Accessor { left, right }) => (None, vec![left.clone(), right.clone()]),
			Nodes::Expression(Expressions::Expression { elements }) => (None, elements.clone()),
			Nodes::Expression(Expressions::Return { value }) => (None, value.iter().cloned().collect()),
			Nodes::Const { value, .. } | Nodes::Literal { value, .. } => (None, vec![value.clone()]),
			_ => (None, Vec::new()),
		}
	};
	if let Some(function) = called_function {
		if matches!(function.borrow().node(), Nodes::Function { .. }) {
			collect_reachable_function(&function, seen, functions);
		}
	}
	for child in children {
		collect_function_references(&child, seen, functions);
	}
}

pub(super) fn reject_raw_code_nodes(node: &NodeReference) -> Result<(), VmError> {
	let children = {
		let borrowed = node.borrow();
		match borrowed.node() {
			Nodes::Raw { glsl, hlsl, msl, .. } => {
				let has_code = [glsl.as_deref(), hlsl.as_deref(), msl.as_deref()]
					.into_iter()
					.flatten()
					.any(|code| !code.trim().is_empty());
				if has_code {
					return Err(VmError::UnsupportedRawCode);
				}
				Vec::new()
			}
			Nodes::Function { statements, .. } => statements.clone(),
			Nodes::Conditional { condition, statements } => {
				let mut children = Vec::with_capacity(statements.len() + 1);
				children.push(condition.clone());
				children.extend(statements.iter().cloned());
				children
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let mut children = Vec::with_capacity(statements.len() + 3);
				children.extend([initializer.clone(), condition.clone(), update.clone()]);
				children.extend(statements.iter().cloned());
				children
			}
			Nodes::Expression(Expressions::FunctionCall { parameters, .. }) => parameters.clone(),
			Nodes::Expression(Expressions::IntrinsicCall { arguments, .. }) => arguments.clone(),
			Nodes::Expression(Expressions::Operator { left, right, .. })
			| Nodes::Expression(Expressions::Accessor { left, right }) => vec![left.clone(), right.clone()],
			Nodes::Expression(Expressions::Expression { elements }) => elements.clone(),
			Nodes::Expression(Expressions::Return { value }) => value.iter().cloned().collect(),
			Nodes::Const { value, .. } | Nodes::Literal { value, .. } => vec![value.clone()],
			_ => Vec::new(),
		}
	};

	for child in children {
		reject_raw_code_nodes(&child)?;
	}

	Ok(())
}

pub(super) fn extract_function_signature(function: &NodeReference) -> Result<FunctionSignature, VmError> {
	let function_ref = function.borrow();
	let (params, return_type, statements) = match function_ref.node() {
		Nodes::Function {
			params,
			return_type,
			statements,
			..
		} => (params.clone(), return_type.clone(), statements.clone()),
		node => {
			return Err(VmError::UnsupportedExpression {
				message: format!("Expected a function, but found {}", describe_node(node)),
			});
		}
	};
	drop(function_ref);

	let mut compiled_params = Vec::with_capacity(params.len());
	for param in params {
		let param_ref = param.borrow();
		let value_type = match param_ref.node() {
			Nodes::Parameter { r#type, .. } => resolve_value_type(r#type)?,
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a parameter, but found {}", describe_node(node)),
				});
			}
		};
		drop(param_ref);
		compiled_params.push(FunctionParameter { node: param, value_type });
	}

	let return_type = resolve_function_return_type(&return_type)?;
	Ok(FunctionSignature {
		params: compiled_params,
		return_type,
		statements,
	})
}

pub(super) fn resolve_function_return_type(return_type: &NodeReference) -> Result<Option<ValueType>, VmError> {
	if return_type.borrow().get_name() == Some("void") {
		Ok(None)
	} else {
		Ok(Some(resolve_value_type(return_type)?))
	}
}

pub(super) fn resolve_callable_return_type(callable: &NodeReference) -> Result<ValueType, VmError> {
	let callable_ref = callable.borrow();
	match callable_ref.node() {
		Nodes::Struct { .. } => resolve_value_type(callable),
		Nodes::Intrinsic { r#return, .. } => {
			let return_type = r#return.clone();
			drop(callable_ref);
			resolve_value_type(&return_type)
		}
		Nodes::Function { return_type, .. } => {
			let return_type = return_type.clone();
			drop(callable_ref);
			resolve_function_return_type(&return_type)?.ok_or_else(|| VmError::UnsupportedExpression {
				message: "Void functions cannot be used as value expressions".to_string(),
			})
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a callable value, but found {}", describe_node(node)),
		}),
	}
}

pub(super) fn resolve_value_type(node: &NodeReference) -> Result<ValueType, VmError> {
	let type_name = node
		.borrow()
		.get_name()
		.map(str::to_string)
		.unwrap_or_else(|| "unknown".to_string());

	match type_name.as_str() {
		"bool" => Ok(ValueType::Bool),
		"u8" => Ok(ValueType::U8),
		"u16" => Ok(ValueType::U16),
		"u32" => Ok(ValueType::U32),
		"i32" => Ok(ValueType::I32),
		"f16" => Ok(ValueType::F16),
		"f32" => Ok(ValueType::F32),
		"atomicu32" => Ok(ValueType::U32),
		"vec2u16" => Ok(ValueType::Vec2U16),
		"vec4u16" => Ok(ValueType::Vec4U16),
		"vec2i" => Ok(ValueType::Vec2I),
		"vec2u" => Ok(ValueType::Vec2U),
		"vec3u" => Ok(ValueType::Vec3U),
		"vec4u" => Ok(ValueType::Vec4U),
		"vec2f16" => Ok(ValueType::Vec2F16),
		"vec3f16" => Ok(ValueType::Vec3F16),
		"vec4f16" => Ok(ValueType::Vec4F16),
		"vec2f" => Ok(ValueType::Vec2F),
		"vec3f" => Ok(ValueType::Vec3F),
		"vec4f" => Ok(ValueType::Vec4F),
		"packed_vec4f" => Ok(ValueType::PackedVec4F),
		"mat4f" => Ok(ValueType::Mat4F),
		"mat4x3f" => Ok(ValueType::Mat4x3F),
		"Texture2D" => Ok(ValueType::Texture2D),
		"Texture3D" => Ok(ValueType::Texture3D),
		"TextureCube" => Ok(ValueType::TextureCube),
		"TextureCubeArray" => Ok(ValueType::TextureCubeArray),
		"ArrayTexture2D" => Ok(ValueType::ArrayTexture2D),
		_ => {
			let fields = match node.borrow().node() {
				Nodes::Struct { fields, .. } => fields.clone(),
				_ => return Err(VmError::UnsupportedType { type_name }),
			};
			let (fields, size) = compile_member_layouts(&fields, false)?;
			Ok(ValueType::Struct {
				name: type_name,
				fields,
				size,
			})
		}
	}
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

pub(super) fn compile_buffer_layout(members: &[NodeReference]) -> Result<BufferLayout, VmError> {
	let (compiled_members, offset) = compile_member_layouts(members, true)?;

	Ok(BufferLayout {
		members: compiled_members,
		size: offset,
	})
}

pub(super) fn compile_member_layouts(
	members: &[NodeReference],
	allow_array_members: bool,
) -> Result<(Vec<BufferMemberLayout>, usize), VmError> {
	let mut offset = 0;
	let mut compiled_members = Vec::with_capacity(members.len());
	for member in members {
		let member = member.borrow();
		match member.node() {
			Nodes::Member { name, r#type, count } => {
				// Aggregate `Value` instances do not represent nested arrays, so only outer buffer layouts may retain counts.
				if count.is_some() && !allow_array_members {
					return Err(VmError::UnsupportedBufferLayout {
						message: format!("Struct field `{}` cannot be an array", name),
					});
				}
				let value_type = resolve_value_type(r#type)?;
				if is_resource_type(&value_type) {
					return Err(VmError::UnsupportedBufferLayout {
						message: format!("Buffer member `{}` cannot contain resource handles", name),
					});
				}
				let count = count.map(std::num::NonZeroUsize::get).unwrap_or(1);
				let member_size = value_type
					.size()
					.checked_mul(count)
					.ok_or_else(|| VmError::UnsupportedBufferLayout {
						message: format!("Buffer member `{}` exceeds addressable CPU memory", name),
					})?;
				compiled_members.push(BufferMemberLayout {
					name: name.clone(),
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
	Ok((compiled_members, offset))
}

pub(super) enum AccessSelector {
	Member(String),
	Index(NodeReference),
}

pub(super) fn extract_access_chain(expression: &NodeReference) -> Result<(NodeReference, Vec<AccessSelector>), VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			let inner = elements[0].clone();
			drop(borrowed);
			extract_access_chain(&inner)
		}
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			let left = left.clone();
			let selector = match right.borrow().node() {
				Nodes::Expression(Expressions::Member { name, .. }) => AccessSelector::Member(name.clone()),
				_ => AccessSelector::Index(right.clone()),
			};
			drop(borrowed);
			let (binding, mut selectors) = extract_access_chain(&left)?;
			selectors.push(selector);
			Ok((binding, selectors))
		}
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = source.clone();
			drop(borrowed);
			if matches!(source.borrow().node(), Nodes::Binding { .. } | Nodes::PushConstant { .. }) {
				Ok((source, Vec::new()))
			} else {
				Err(VmError::UnsupportedExpression {
					message: "Accessor is not rooted in a buffer binding".to_string(),
				})
			}
		}
		Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok((expression.clone(), Vec::new())),
		Nodes::Expression(expression) => Err(VmError::UnsupportedExpression {
			message: format!(
				"Expected a buffer accessor, but found {}",
				match expression {
					Expressions::Return { .. } => "return",
					Expressions::Continue => "continue",
					Expressions::Discard => "discard",
					Expressions::Member { .. } => "member",
					Expressions::Expression { .. } => "multi-element expression group",
					Expressions::Literal { .. } => "literal",
					Expressions::FunctionCall { .. } => "function call",
					Expressions::IntrinsicCall { .. } => "intrinsic call",
					Expressions::Operator { .. } => "operator",
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

pub(super) fn accessor_references_buffer(expression: &NodeReference) -> bool {
	extract_access_chain(expression)
		.ok()
		.is_some_and(|(binding, _)| matches!(binding.borrow().node(), Nodes::Binding { .. } | Nodes::PushConstant { .. }))
}

pub(super) fn accessor_references_output(expression: &NodeReference) -> bool {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Accessor { left, .. }) => output_member_references_interface(left),
		_ => false,
	}
}

pub(super) fn output_member_references_interface(expression: &NodeReference) -> bool {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
			output_member_references_interface(&elements[0])
		}
		Nodes::Expression(Expressions::Member { source, .. }) => matches!(source.borrow().node(), Nodes::Output { .. }),
		_ => false,
	}
}

pub(super) fn extract_binding_reference(expression: &NodeReference) -> Result<NodeReference, VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok(expression.clone()),
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = source.clone();
			drop(borrowed);

			let result = match source.borrow().node() {
				Nodes::Binding { .. } | Nodes::PushConstant { .. } => Ok(source.clone()),
				Nodes::Expression(Expressions::Member { .. }) => extract_binding_reference(&source),
				_ => Err(VmError::UnsupportedExpression {
					message: format!(
						"Only direct binding or push constant member access is supported, but found {}",
						describe_node(source.borrow().node())
					),
				}),
			};

			result
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!(
				"Expected a binding or push constant reference, but found {}",
				describe_node(node)
			),
		}),
	}
}

pub(super) fn extract_member_name(expression: &NodeReference) -> Result<String, VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Member { name, .. }) => Ok(name.clone()),
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
		ValueType::Vec4U16 | ValueType::Vec4U | ValueType::Vec4F16 | ValueType::Vec4F | ValueType::PackedVec4F => {
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
	match value_type {
		ValueType::Mat4F => return Ok((ValueType::Vec4F, 4)),
		ValueType::Mat4x3F => return Ok((ValueType::Vec3F, 4)),
		_ => {}
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

pub(super) fn resolve_referenced_value_type(source: &NodeReference) -> Result<ValueType, VmError> {
	match source.borrow().node() {
		Nodes::Member { r#type, .. }
		| Nodes::Parameter { r#type, .. }
		| Nodes::Specialization { r#type, .. }
		| Nodes::Const { r#type, .. } => resolve_value_type(r#type),
		Nodes::Input { format, .. } | Nodes::Output { format, .. } => resolve_value_type(format),
		Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => resolve_value_type(r#type),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Cannot resolve a value type from {}", describe_node(node)),
		}),
	}
}

pub(super) fn describe_node(node: &Nodes) -> &'static str {
	match node {
		Nodes::Null => "null",
		Nodes::Scope { .. } => "scope",
		Nodes::Struct { .. } => "struct",
		Nodes::Member { .. } => "member",
		Nodes::Function { .. } => "function",
		Nodes::Conditional { .. } => "conditional",
		Nodes::ForLoop { .. } => "for loop",
		Nodes::Specialization { .. } => "specialization",
		Nodes::Expression(_) => "expression",
		Nodes::Raw { .. } => "raw",
		Nodes::Binding { .. } => "binding",
		Nodes::PushConstant { .. } => "push constant",
		Nodes::Intrinsic { .. } => "intrinsic",
		Nodes::Input { .. } => "input",
		Nodes::Output { .. } => "output",
		Nodes::TaskPayload { .. } => "task payload",
		Nodes::Workgroup { .. } => "workgroup storage",
		Nodes::Parameter { .. } => "parameter",
		Nodes::Literal { .. } => "literal",
		Nodes::Const { .. } => "const",
	}
}
