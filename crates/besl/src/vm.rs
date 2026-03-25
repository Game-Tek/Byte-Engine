use std::collections::HashMap;

use crate::lexer::{BindingTypes, Expressions, NodeReference, Nodes, Operators};

/// The `DescriptorSlot` struct identifies the descriptor set and binding that a VM resource uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DescriptorSlot {
	set: u32,
	binding: u32,
}

impl DescriptorSlot {
	pub const fn new(set: u32, binding: u32) -> Self {
		Self { set, binding }
	}

	pub const fn set(&self) -> u32 {
		self.set
	}

	pub const fn binding(&self) -> u32 {
		self.binding
	}
}

/// The `ValueType` enum stores the scalar BESL value kinds that the first VM pass can execute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueType {
	U8,
	U16,
	U32,
	I32,
	F32,
}

impl ValueType {
	pub const fn size(&self) -> usize {
		match self {
			ValueType::U8 => 1,
			ValueType::U16 => 2,
			ValueType::U32 | ValueType::I32 | ValueType::F32 => 4,
		}
	}

	fn name(&self) -> &'static str {
		match self {
			ValueType::U8 => "u8",
			ValueType::U16 => "u16",
			ValueType::U32 => "u32",
			ValueType::I32 => "i32",
			ValueType::F32 => "f32",
		}
	}
}

/// The `BufferMemberLayout` struct stores the packed VM layout information for one buffer member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferMemberLayout {
	name: String,
	offset: usize,
	value_type: ValueType,
}

impl BufferMemberLayout {
	pub fn name(&self) -> &str {
		&self.name
	}

	pub const fn offset(&self) -> usize {
		self.offset
	}

	pub fn value_type(&self) -> &ValueType {
		&self.value_type
	}
}

/// The `BufferLayout` struct stores the packed CPU memory layout that the VM uses for a buffer binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferLayout {
	members: Vec<BufferMemberLayout>,
	size: usize,
}

impl BufferLayout {
	pub fn members(&self) -> &[BufferMemberLayout] {
		&self.members
	}

	pub const fn size(&self) -> usize {
		self.size
	}

	fn member(&self, name: &str) -> Option<&BufferMemberLayout> {
		self.members.iter().find(|member| member.name == name)
	}
}

/// The `DescriptorLayout` enum stores the VM resource layout required by one descriptor slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescriptorLayout {
	Buffer(BufferLayout),
}

/// The `Buffer` struct stores mutable bytes together with the VM layout that gives those bytes meaning.
#[derive(Debug)]
pub struct Buffer {
	layout: BufferLayout,
	data: Vec<u8>,
}

impl Buffer {
	pub fn new(layout: BufferLayout) -> Self {
		Self {
			data: vec![0; layout.size()],
			layout,
		}
	}

	pub fn layout(&self) -> &BufferLayout {
		&self.layout
	}

	pub fn bytes(&self) -> &[u8] {
		&self.data
	}

	/// Reads an `f32` member from the buffer layout by name.
	pub fn read_f32(&self, member_name: &str) -> Result<f32, VmError> {
		let member = self.layout.member(member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.to_string(),
		})?;

		if member.value_type != ValueType::F32 {
			return Err(VmError::TypeMismatch {
				expected: "f32".to_string(),
				found: member.value_type.name().to_string(),
			});
		}

		let bytes = self.read_bytes(member.offset, member.value_type.size())?;

		Ok(f32::from_ne_bytes(bytes.try_into().expect("Invalid f32 byte count")))
	}

	fn read_scalar(&self, offset: usize, value_type: &ValueType) -> Result<ScalarValue, VmError> {
		let bytes = self.read_bytes(offset, value_type.size())?;

		let value = match value_type {
			ValueType::U8 => ScalarValue::U8(bytes[0]),
			ValueType::U16 => ScalarValue::U16(u16::from_ne_bytes(bytes.try_into().expect("Invalid u16 byte count"))),
			ValueType::U32 => ScalarValue::U32(u32::from_ne_bytes(bytes.try_into().expect("Invalid u32 byte count"))),
			ValueType::I32 => ScalarValue::I32(i32::from_ne_bytes(bytes.try_into().expect("Invalid i32 byte count"))),
			ValueType::F32 => ScalarValue::F32(f32::from_ne_bytes(bytes.try_into().expect("Invalid f32 byte count"))),
		};

		Ok(value)
	}

	fn write_scalar(&mut self, offset: usize, value_type: &ValueType, value: &ScalarValue) -> Result<(), VmError> {
		if value_type != &value.value_type() {
			return Err(VmError::TypeMismatch {
				expected: value_type.name().to_string(),
				found: value.value_type().name().to_string(),
			});
		}

		let bytes = match value {
			ScalarValue::U8(value) => value.to_ne_bytes().to_vec(),
			ScalarValue::U16(value) => value.to_ne_bytes().to_vec(),
			ScalarValue::U32(value) => value.to_ne_bytes().to_vec(),
			ScalarValue::I32(value) => value.to_ne_bytes().to_vec(),
			ScalarValue::F32(value) => value.to_ne_bytes().to_vec(),
		};

		self.write_bytes(offset, &bytes)
	}

	fn read_bytes(&self, offset: usize, size: usize) -> Result<&[u8], VmError> {
		self.data.get(offset..offset + size).ok_or(VmError::BufferAccessOutOfBounds {
			offset,
			size,
			buffer_size: self.data.len(),
		})
	}

	fn write_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<(), VmError> {
		let buffer_size = self.data.len();
		let slice = self
			.data
			.get_mut(offset..offset + bytes.len())
			.ok_or(VmError::BufferAccessOutOfBounds {
				offset,
				size: bytes.len(),
				buffer_size,
			})?;

		slice.copy_from_slice(bytes);

		Ok(())
	}
}

enum DescriptorBinding<'a> {
	Buffer(&'a mut Buffer),
}

/// The `DescriptorBindings` struct stores the mutable resources that a compiled BESL VM program can access.
pub struct DescriptorBindings<'a> {
	bindings: HashMap<DescriptorSlot, DescriptorBinding<'a>>,
}

impl<'a> Default for DescriptorBindings<'a> {
	fn default() -> Self {
		Self::new()
	}
}

impl<'a> DescriptorBindings<'a> {
	pub fn new() -> Self {
		Self {
			bindings: HashMap::new(),
		}
	}

	pub fn bind_buffer(&mut self, slot: DescriptorSlot, buffer: &'a mut Buffer) {
		self.bindings.insert(slot, DescriptorBinding::Buffer(buffer));
	}

	fn buffer_mut(&mut self, slot: DescriptorSlot) -> Result<&mut Buffer, VmError> {
		let descriptor = self.bindings.get_mut(&slot).ok_or(VmError::UnboundDescriptor { slot })?;

		match descriptor {
			DescriptorBinding::Buffer(buffer) => Ok(&mut **buffer),
		}
	}
}

/// The `ExecutableProgram` struct stores the runnable VM form of a lexed BESL program.
pub struct ExecutableProgram {
	descriptor_layouts: HashMap<DescriptorSlot, DescriptorLayout>,
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	register_count: usize,
}

impl ExecutableProgram {
	/// Compiles a lexed BESL program into a runnable VM program.
	pub fn compile(program: NodeReference) -> Result<Self, VmError> {
		let main = resolve_main_function(&program)?;

		let function = main.borrow();
		let (params, return_type, statements) = match function.node() {
			Nodes::Function {
				params,
				return_type,
				statements,
				..
			} => (params.clone(), return_type.clone(), statements.clone()),
			_ => {
				return Err(VmError::MissingMainFunction);
			}
		};
		drop(function);

		if !params.is_empty() {
			return Err(VmError::UnsupportedMainSignature {
				message: "Main functions with parameters are not supported".to_string(),
			});
		}

		let return_type_name = return_type
			.borrow()
			.get_name()
			.map(str::to_string)
			.unwrap_or_else(|| "unknown".to_string());

		if return_type_name != "void" {
			return Err(VmError::UnsupportedMainSignature {
				message: format!("Main functions must return void, but found `{}`", return_type_name),
			});
		}

		let mut compiler = Compiler::default();

		// Build the bytecode for the main function in statement order.
		for statement in &statements {
			compiler.compile_statement(statement)?;
		}

		compiler.instructions.push(Instruction::Return);

		Ok(Self {
			descriptor_layouts: compiler.descriptor_layouts,
			instructions: compiler.instructions,
			local_types: compiler.local_types,
			register_count: compiler.register_count,
		})
	}

	pub fn descriptor_layout(&self, slot: DescriptorSlot) -> Option<&DescriptorLayout> {
		self.descriptor_layouts.get(&slot)
	}

	pub fn buffer_layout(&self, slot: DescriptorSlot) -> Option<&BufferLayout> {
		match self.descriptor_layouts.get(&slot) {
			Some(DescriptorLayout::Buffer(layout)) => Some(layout),
			None => None,
		}
	}

	/// Executes the compiled `main` function using the currently bound descriptor resources.
	pub fn run_main(&self, descriptors: &mut DescriptorBindings<'_>) -> Result<(), VmError> {
		let mut registers = vec![None; self.register_count];
		let mut locals = vec![None; self.local_types.len()];

		for instruction in &self.instructions {
			match instruction {
				Instruction::LoadLiteral { register, value } => {
					registers[*register] = Some(value.clone());
				}
				Instruction::LoadLocal { register, local } => {
					let value = locals
						.get(*local)
						.and_then(Option::clone)
						.ok_or(VmError::UninitializedLocal { local: *local })?;
					registers[*register] = Some(value);
				}
				Instruction::StoreLocal { local, register } => {
					let value = read_register(&registers, *register)?;
					locals[*local] = Some(value.clone());
				}
				Instruction::LoadBuffer {
					register,
					slot,
					offset,
					value_type,
				} => {
					let value = descriptors.buffer_mut(*slot)?.read_scalar(*offset, value_type)?;
					registers[*register] = Some(value);
				}
				Instruction::StoreBuffer {
					slot,
					offset,
					value_type,
					register,
				} => {
					let value = read_register(&registers, *register)?;
					descriptors.buffer_mut(*slot)?.write_scalar(*offset, value_type, &value)?;
				}
				Instruction::Return => {
					break;
				}
			}
		}

		Ok(())
	}
}

#[derive(Clone, Debug, PartialEq)]
enum ScalarValue {
	U8(u8),
	U16(u16),
	U32(u32),
	I32(i32),
	F32(f32),
}

impl ScalarValue {
	fn value_type(&self) -> ValueType {
		match self {
			ScalarValue::U8(_) => ValueType::U8,
			ScalarValue::U16(_) => ValueType::U16,
			ScalarValue::U32(_) => ValueType::U32,
			ScalarValue::I32(_) => ValueType::I32,
			ScalarValue::F32(_) => ValueType::F32,
		}
	}
}

#[derive(Clone, Debug, PartialEq)]
enum Instruction {
	LoadLiteral {
		register: usize,
		value: ScalarValue,
	},
	LoadLocal {
		register: usize,
		local: usize,
	},
	StoreLocal {
		local: usize,
		register: usize,
	},
	LoadBuffer {
		register: usize,
		slot: DescriptorSlot,
		offset: usize,
		value_type: ValueType,
	},
	StoreBuffer {
		slot: DescriptorSlot,
		offset: usize,
		value_type: ValueType,
		register: usize,
	},
	Return,
}

#[derive(Default)]
struct Compiler {
	descriptor_layouts: HashMap<DescriptorSlot, DescriptorLayout>,
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	locals_by_reference: HashMap<NodeReference, usize>,
	register_count: usize,
}

impl Compiler {
	/// Compiles one BESL statement into bytecode while tracking locals and descriptors.
	fn compile_statement(&mut self, statement: &NodeReference) -> Result<(), VmError> {
		let borrowed = statement.borrow();
		let result = match borrowed.node() {
			Nodes::Expression(Expressions::Operator {
				operator: Operators::Assignment,
				left,
				right,
			}) => {
				let left = left.clone();
				let right = right.clone();
				drop(borrowed);
				self.compile_assignment(statement, left, right)
			}
			Nodes::Expression(Expressions::Return) => {
				drop(borrowed);
				self.instructions.push(Instruction::Return);
				Ok(())
			}
			Nodes::Expression(Expressions::Member { .. }) | Nodes::Expression(Expressions::Accessor { .. }) => Ok(()),
			Nodes::Expression(other) => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported statement expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedStatement {
				message: format!("Unsupported statement node: {}", describe_node(node)),
			}),
		};

		result
	}

	fn compile_assignment(
		&mut self,
		statement: &NodeReference,
		left: NodeReference,
		right: NodeReference,
	) -> Result<(), VmError> {
		let left_expression = left.borrow();

		match left_expression.node() {
			Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
				let name = name.clone();
				let value_type = resolve_value_type(r#type)?;
				drop(left_expression);

				let local = self.define_local(statement.clone(), left, &name, value_type.clone());
				let register = self.compile_value_expression(&right, &value_type)?;
				self.instructions.push(Instruction::StoreLocal { local, register });
				Ok(())
			}
			Nodes::Expression(Expressions::Accessor { .. }) => {
				drop(left_expression);

				let target = self.resolve_buffer_access(&left, RequiredAccess::Write)?;
				let register = self.compile_value_expression(&right, &target.value_type)?;
				self.instructions.push(Instruction::StoreBuffer {
					slot: target.slot,
					offset: target.offset,
					value_type: target.value_type,
					register,
				});
				Ok(())
			}
			node => Err(VmError::UnsupportedAssignmentTarget {
				message: format!("Unsupported assignment target: {}", describe_node(node)),
			}),
		}
	}

	/// Compiles a scalar BESL expression into one register-producing VM instruction sequence.
	fn compile_value_expression(&mut self, expression: &NodeReference, expected_type: &ValueType) -> Result<usize, VmError> {
		let borrowed = expression.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::Literal { value }) => {
				let value = value.clone();
				drop(borrowed);

				let register = self.allocate_register();
				let value = parse_literal(&value, expected_type)?;
				self.instructions.push(Instruction::LoadLiteral { register, value });
				Ok(register)
			}
			Nodes::Expression(Expressions::Member { source, .. }) => {
				let source = source.clone();
				drop(borrowed);

				let local = self
					.locals_by_reference
					.get(&source)
					.copied()
					.ok_or_else(|| VmError::UnsupportedExpression {
						message: "Only local variable reads are supported as bare member expressions".to_string(),
					})?;

				let actual_type = self.local_types.get(local).ok_or(VmError::UninitializedLocal { local })?;
				if actual_type != expected_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: actual_type.name().to_string(),
					});
				}

				let register = self.allocate_register();
				self.instructions.push(Instruction::LoadLocal { register, local });
				Ok(register)
			}
			Nodes::Expression(Expressions::Accessor { .. }) => {
				drop(borrowed);

				let target = self.resolve_buffer_access(expression, RequiredAccess::Read)?;
				if &target.value_type != expected_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: target.value_type.name().to_string(),
					});
				}

				let register = self.allocate_register();
				self.instructions.push(Instruction::LoadBuffer {
					register,
					slot: target.slot,
					offset: target.offset,
					value_type: target.value_type,
				});
				Ok(register)
			}
			Nodes::Expression(other) => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value expression: {:?}", other),
			}),
			node => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported value node: {}", describe_node(node)),
			}),
		}
	}

	fn define_local(
		&mut self,
		statement: NodeReference,
		declaration: NodeReference,
		_name: &str,
		value_type: ValueType,
	) -> usize {
		let local = self.local_types.len();
		self.local_types.push(value_type);
		self.locals_by_reference.insert(statement, local);
		self.locals_by_reference.insert(declaration, local);
		local
	}

	fn allocate_register(&mut self) -> usize {
		let register = self.register_count;
		self.register_count += 1;
		register
	}

	/// Resolves a BESL accessor into the descriptor slot and packed byte offset that the VM should access.
	fn resolve_buffer_access(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
	) -> Result<ResolvedBufferAccess, VmError> {
		let (binding, member_name) = extract_buffer_member_access(expression)?;

		let binding_ref = binding.borrow();
		let (slot, layout) = match binding_ref.node() {
			Nodes::Binding {
				set,
				binding,
				read,
				write,
				r#type,
				..
			} => {
				match access {
					RequiredAccess::Read if !read => {
						return Err(VmError::DescriptorAccessDenied {
							slot: DescriptorSlot::new(*set, *binding),
							access: "read",
						});
					}
					RequiredAccess::Write if !write => {
						return Err(VmError::DescriptorAccessDenied {
							slot: DescriptorSlot::new(*set, *binding),
							access: "write",
						});
					}
					_ => {}
				}

				let slot = DescriptorSlot::new(*set, *binding);
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
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected a binding access, but found {}", describe_node(node)),
				});
			}
		};
		drop(binding_ref);

		match self.descriptor_layouts.get(&slot) {
			Some(existing) if existing != &DescriptorLayout::Buffer(layout.clone()) => {
				return Err(VmError::UnsupportedDescriptor {
					slot,
					message: "Descriptor slot was reused with a different layout".to_string(),
				});
			}
			Some(_) => {}
			None => {
				self.descriptor_layouts.insert(slot, DescriptorLayout::Buffer(layout.clone()));
			}
		}

		let member = layout.member(&member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.clone(),
		})?;

		Ok(ResolvedBufferAccess {
			slot,
			offset: member.offset,
			value_type: member.value_type.clone(),
		})
	}
}

struct ResolvedBufferAccess {
	slot: DescriptorSlot,
	offset: usize,
	value_type: ValueType,
}

#[derive(Clone, Copy)]
enum RequiredAccess {
	Read,
	Write,
}

fn resolve_main_function(program: &NodeReference) -> Result<NodeReference, VmError> {
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

fn resolve_value_type(node: &NodeReference) -> Result<ValueType, VmError> {
	let type_name = node
		.borrow()
		.get_name()
		.map(str::to_string)
		.unwrap_or_else(|| "unknown".to_string());

	match type_name.as_str() {
		"u8" => Ok(ValueType::U8),
		"u16" => Ok(ValueType::U16),
		"u32" => Ok(ValueType::U32),
		"i32" => Ok(ValueType::I32),
		"f32" => Ok(ValueType::F32),
		_ => Err(VmError::UnsupportedType { type_name }),
	}
}

fn compile_buffer_layout(members: &[NodeReference]) -> Result<BufferLayout, VmError> {
	let mut offset = 0;
	let mut compiled_members = Vec::with_capacity(members.len());

	// Pack the supported scalar buffer members into the VM's CPU layout.
	for member in members {
		let member = member.borrow();
		match member.node() {
			Nodes::Member { name, r#type, count } => {
				if count.is_some() {
					return Err(VmError::UnsupportedBufferLayout {
						message: format!("Array members are not supported for `{}`", name),
					});
				}

				let value_type = resolve_value_type(r#type)?;
				compiled_members.push(BufferMemberLayout {
					name: name.clone(),
					offset,
					value_type: value_type.clone(),
				});
				offset += value_type.size();
			}
			node => {
				return Err(VmError::UnsupportedBufferLayout {
					message: format!("Unsupported buffer member node: {}", describe_node(node)),
				});
			}
		}
	}

	Ok(BufferLayout {
		members: compiled_members,
		size: offset,
	})
}

fn extract_buffer_member_access(expression: &NodeReference) -> Result<(NodeReference, String), VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Accessor { left, right }) => {
			let binding = extract_binding_reference(left)?;
			let member_name = extract_member_name(right)?;
			Ok((binding, member_name))
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a buffer member accessor, but found {}", describe_node(node)),
		}),
	}
}

fn extract_binding_reference(expression: &NodeReference) -> Result<NodeReference, VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = source.clone();
			drop(borrowed);

			if matches!(source.borrow().node(), Nodes::Binding { .. }) {
				Ok(source)
			} else {
				Err(VmError::UnsupportedExpression {
					message: "Only direct binding member access is supported".to_string(),
				})
			}
		}
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a binding reference, but found {}", describe_node(node)),
		}),
	}
}

fn extract_member_name(expression: &NodeReference) -> Result<String, VmError> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Member { name, .. }) => Ok(name.clone()),
		node => Err(VmError::UnsupportedExpression {
			message: format!("Expected a buffer member name, but found {}", describe_node(node)),
		}),
	}
}

fn describe_node(node: &Nodes) -> &'static str {
	match node {
		Nodes::Null => "null",
		Nodes::Scope { .. } => "scope",
		Nodes::Struct { .. } => "struct",
		Nodes::Member { .. } => "member",
		Nodes::Function { .. } => "function",
		Nodes::Specialization { .. } => "specialization",
		Nodes::Expression(_) => "expression",
		Nodes::Raw { .. } => "raw",
		Nodes::Binding { .. } => "binding",
		Nodes::PushConstant { .. } => "push constant",
		Nodes::Intrinsic { .. } => "intrinsic",
		Nodes::Input { .. } => "input",
		Nodes::Output { .. } => "output",
		Nodes::Parameter { .. } => "parameter",
		Nodes::Literal { .. } => "literal",
	}
}

fn parse_literal(value: &str, value_type: &ValueType) -> Result<ScalarValue, VmError> {
	let parsed = match value_type {
		ValueType::U8 => value
			.parse::<u8>()
			.map(ScalarValue::U8)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::U16 => value
			.parse::<u16>()
			.map(ScalarValue::U16)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::U32 => value
			.parse::<u32>()
			.map(ScalarValue::U32)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::I32 => value
			.parse::<i32>()
			.map(ScalarValue::I32)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::F32 => value
			.parse::<f32>()
			.map(ScalarValue::F32)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
	};

	Ok(parsed)
}

fn read_register(registers: &[Option<ScalarValue>], register: usize) -> Result<ScalarValue, VmError> {
	registers
		.get(register)
		.and_then(Option::clone)
		.ok_or(VmError::UninitializedRegister { register })
}

#[derive(Debug, PartialEq, Eq)]
pub enum VmError {
	MissingMainFunction,
	UnsupportedMainSignature { message: String },
	UnsupportedType { type_name: String },
	UnsupportedStatement { message: String },
	UnsupportedAssignmentTarget { message: String },
	UnsupportedExpression { message: String },
	UnsupportedBufferLayout { message: String },
	UnsupportedDescriptor { slot: DescriptorSlot, message: String },
	DescriptorAccessDenied { slot: DescriptorSlot, access: &'static str },
	UnknownBufferMember { member: String },
	UnboundDescriptor { slot: DescriptorSlot },
	BufferAccessOutOfBounds { offset: usize, size: usize, buffer_size: usize },
	InvalidLiteral { value: String, value_type: String },
	TypeMismatch { expected: String, found: String },
	UninitializedRegister { register: usize },
	UninitializedLocal { local: usize },
}

impl std::fmt::Display for VmError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			VmError::MissingMainFunction => {
				write!(f, "Missing main function. The most likely cause is that the lexed BESL program does not define `main`.")
			}
			VmError::UnsupportedMainSignature { message } => write!(
				f,
				"Unsupported main signature: {}. The most likely cause is that the VM only accepts `main: fn () -> void` right now.",
				message
			),
			VmError::UnsupportedType { type_name } => write!(
				f,
				"Unsupported type `{}`. The most likely cause is that the VM currently only supports scalar buffer and local value types.",
				type_name
			),
			VmError::UnsupportedStatement { message } => write!(
				f,
				"Unsupported statement. {}. The most likely cause is that the VM currently only supports assignments, plain member access, and `return`.",
				message
			),
			VmError::UnsupportedAssignmentTarget { message } => write!(
				f,
				"Unsupported assignment target. {}. The most likely cause is that the VM can currently assign only to local declarations or buffer members.",
				message
			),
			VmError::UnsupportedExpression { message } => write!(
				f,
				"Unsupported expression. {}. The most likely cause is that the VM currently only supports literals, local reads, and buffer member reads.",
				message
			),
			VmError::UnsupportedBufferLayout { message } => write!(
				f,
				"Unsupported buffer layout. {}. The most likely cause is that the VM currently only supports packed scalar buffer members.",
				message
			),
			VmError::UnsupportedDescriptor { slot, message } => write!(
				f,
				"Unsupported descriptor at set {} binding {}. {}. The most likely cause is that the VM currently only supports buffer descriptors with stable layouts.",
				slot.set(),
				slot.binding(),
				message
			),
			VmError::DescriptorAccessDenied { slot, access } => write!(
				f,
				"Descriptor access denied at set {} binding {}. The most likely cause is that the BESL binding was not declared with `{}` access.",
				slot.set(),
				slot.binding(),
				access
			),
			VmError::UnknownBufferMember { member } => write!(
				f,
				"Unknown buffer member `{}`. The most likely cause is that the BESL accessor does not match the bound buffer layout.",
				member
			),
			VmError::UnboundDescriptor { slot } => write!(
				f,
				"Unbound descriptor at set {} binding {}. The most likely cause is that no buffer was bound into the descriptor slot before execution.",
				slot.set(),
				slot.binding()
			),
			VmError::BufferAccessOutOfBounds {
				offset,
				size,
				buffer_size,
			} => write!(
				f,
				"Buffer access out of bounds at byte {} for {} bytes in a {} byte buffer. The most likely cause is that the bound buffer does not match the compiled BESL buffer layout.",
				offset,
				size,
				buffer_size
			),
			VmError::InvalidLiteral { value, value_type } => write!(
				f,
				"Invalid literal `{}` for `{}`. The most likely cause is that the literal cannot be parsed as the target BESL scalar type.",
				value,
				value_type
			),
			VmError::TypeMismatch { expected, found } => write!(
				f,
				"Type mismatch: expected `{}` but found `{}`. The most likely cause is that the BESL assignment mixes incompatible scalar types.",
				expected,
				found
			),
			VmError::UninitializedRegister { register } => write!(
				f,
				"Uninitialized register {}. The most likely cause is that the VM tried to use a register before any instruction wrote a value into it.",
				register
			),
			VmError::UninitializedLocal { local } => write!(
				f,
				"Uninitialized local {}. The most likely cause is that the BESL program read a local variable before assigning a value to it.",
				local
			),
		}
	}
}

impl std::error::Error for VmError {}

#[cfg(test)]
mod tests {
	use crate::{compile_to_besl, BindingTypes, Node};

	use super::{Buffer, DescriptorBindings, DescriptorSlot, ExecutableProgram};

	#[test]
	fn executable_program_runs_main_and_writes_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			buff.value = 42.0;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				0,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let slot = DescriptorSlot::new(0, 0);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 42.0);
	}

	#[test]
	fn executable_program_reads_locals_before_writing_to_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = 7.5;
			buff.value = value;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				1,
				3,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let slot = DescriptorSlot::new(1, 3);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 7.5);
	}
}
