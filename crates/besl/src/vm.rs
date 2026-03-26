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
	Vec2U,
	Vec2F,
	Vec3F,
	Vec4F,
	Mat4F,
}

impl ValueType {
	pub const fn size(&self) -> usize {
		match self {
			ValueType::U8 => 1,
			ValueType::U16 => 2,
			ValueType::U32 | ValueType::I32 | ValueType::F32 => 4,
			ValueType::Vec2U | ValueType::Vec2F => 8,
			ValueType::Vec3F => 12,
			ValueType::Vec4F => 16,
			ValueType::Mat4F => 64,
		}
	}

	fn name(&self) -> &'static str {
		match self {
			ValueType::U8 => "u8",
			ValueType::U16 => "u16",
			ValueType::U32 => "u32",
			ValueType::I32 => "i32",
			ValueType::F32 => "f32",
			ValueType::Vec2U => "vec2u",
			ValueType::Vec2F => "vec2f",
			ValueType::Vec3F => "vec3f",
			ValueType::Vec4F => "vec4f",
			ValueType::Mat4F => "mat4f",
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
	Texture,
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

	/// Reads a VM value from the buffer layout by member name.
	pub fn read(&self, member_name: &str) -> Result<Value, VmError> {
		let member = self.member_layout(member_name)?;

		self.read_value(member.offset, &member.value_type)
	}

	/// Writes a VM value into the buffer layout by member name.
	pub fn write(&mut self, member_name: &str, value: Value) -> Result<(), VmError> {
		let (offset, value_type) = {
			let member = self.member_layout(member_name)?;
			(member.offset, member.value_type.clone())
		};

		self.write_value(offset, &value_type, &value)
	}

	/// Reads an `f32` member from the buffer layout by name.
	pub fn read_f32(&self, member_name: &str) -> Result<f32, VmError> {
		match self.read(member_name)? {
			Value::F32(value) => Ok(value),
			value => Err(VmError::TypeMismatch {
				expected: "f32".to_string(),
				found: value.value_type().name().to_string(),
			}),
		}
	}

	fn read_value(&self, offset: usize, value_type: &ValueType) -> Result<ScalarValue, VmError> {
		let bytes = self.read_bytes(offset, value_type.size())?;

		let value = match value_type {
			ValueType::U8 => ScalarValue::U8(bytes[0]),
			ValueType::U16 => ScalarValue::U16(u16::from_ne_bytes(bytes.try_into().expect("Invalid u16 byte count"))),
			ValueType::U32 => ScalarValue::U32(u32::from_ne_bytes(bytes.try_into().expect("Invalid u32 byte count"))),
			ValueType::I32 => ScalarValue::I32(i32::from_ne_bytes(bytes.try_into().expect("Invalid i32 byte count"))),
			ValueType::F32 => ScalarValue::F32(f32::from_ne_bytes(bytes.try_into().expect("Invalid f32 byte count"))),
			ValueType::Vec2U => ScalarValue::Vec2U(read_u32_array::<2>(bytes)?),
			ValueType::Vec2F => ScalarValue::Vec2F(read_f32_array::<2>(bytes)?),
			ValueType::Vec3F => ScalarValue::Vec3F(read_f32_array::<3>(bytes)?),
			ValueType::Vec4F => ScalarValue::Vec4F(read_f32_array::<4>(bytes)?),
			ValueType::Mat4F => ScalarValue::Mat4F(read_f32_array::<16>(bytes)?),
		};

		Ok(value)
	}

	fn write_value(&mut self, offset: usize, value_type: &ValueType, value: &ScalarValue) -> Result<(), VmError> {
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
			ScalarValue::Vec2U(value) => write_u32_slice(value),
			ScalarValue::Vec2F(value) => write_f32_slice(value),
			ScalarValue::Vec3F(value) => write_f32_slice(value),
			ScalarValue::Vec4F(value) => write_f32_slice(value),
			ScalarValue::Mat4F(value) => write_f32_slice(value),
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

	fn member_layout(&self, member_name: &str) -> Result<&BufferMemberLayout, VmError> {
		self.layout.member(member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.to_string(),
		})
	}
}

/// The `Texture` struct stores a CPU texture that the VM can fetch or bilinearly sample.
#[derive(Debug)]
pub struct Texture {
	width: u32,
	height: u32,
	texels: Vec<[f32; 4]>,
}

impl Texture {
	pub fn new(width: u32, height: u32) -> Result<Self, VmError> {
		if width == 0 || height == 0 {
			return Err(VmError::InvalidTextureDimensions { width, height });
		}

		let texel_count = (width as usize) * (height as usize);
		Ok(Self {
			width,
			height,
			texels: vec![[0.0, 0.0, 0.0, 0.0]; texel_count],
		})
	}

	pub fn write(&mut self, coord: [u32; 2], value: [f32; 4]) -> Result<(), VmError> {
		let index = self.texel_index(coord)?;
		self.texels[index] = value;
		Ok(())
	}

	/// Fetches one texel without interpolation.
	pub fn fetch(&self, coord: [u32; 2]) -> Result<Value, VmError> {
		Ok(Value::Vec4F(self.fetch_texel(coord)?))
	}

	/// Samples one texel using bilinear interpolation in normalized UV space.
	pub fn sample(&self, uv: [f32; 2]) -> Result<Value, VmError> {
		let max_x = self.width.saturating_sub(1) as f32;
		let max_y = self.height.saturating_sub(1) as f32;
		let x = uv[0].clamp(0.0, 1.0) * max_x;
		let y = uv[1].clamp(0.0, 1.0) * max_y;

		let x0 = x.floor() as u32;
		let y0 = y.floor() as u32;
		let x1 = x.ceil() as u32;
		let y1 = y.ceil() as u32;
		let tx = x - x0 as f32;
		let ty = y - y0 as f32;

		let top = lerp_rgba(self.fetch_texel([x0, y0])?, self.fetch_texel([x1, y0])?, tx);
		let bottom = lerp_rgba(self.fetch_texel([x0, y1])?, self.fetch_texel([x1, y1])?, tx);

		Ok(Value::Vec4F(lerp_rgba(top, bottom, ty)))
	}

	fn fetch_texel(&self, coord: [u32; 2]) -> Result<[f32; 4], VmError> {
		let index = self.texel_index(coord)?;
		Ok(self.texels[index])
	}

	fn texel_index(&self, coord: [u32; 2]) -> Result<usize, VmError> {
		let [x, y] = coord;
		if x >= self.width || y >= self.height {
			return Err(VmError::TextureAccessOutOfBounds {
				x,
				y,
				width: self.width,
				height: self.height,
			});
		}

		Ok((y as usize) * (self.width as usize) + (x as usize))
	}
}

enum DescriptorBinding<'a> {
	Buffer(&'a mut Buffer),
	Texture(&'a mut Texture),
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

	pub fn bind_texture(&mut self, slot: DescriptorSlot, texture: &'a mut Texture) {
		self.bindings.insert(slot, DescriptorBinding::Texture(texture));
	}

	fn buffer_mut(&mut self, slot: DescriptorSlot) -> Result<&mut Buffer, VmError> {
		let descriptor = self.bindings.get_mut(&slot).ok_or(VmError::UnboundDescriptor { slot })?;

		match descriptor {
			DescriptorBinding::Buffer(buffer) => Ok(&mut **buffer),
			DescriptorBinding::Texture(_) => Err(VmError::DescriptorTypeMismatch {
				slot,
				expected: "buffer",
				found: "texture",
			}),
		}
	}

	fn texture_mut(&mut self, slot: DescriptorSlot) -> Result<&mut Texture, VmError> {
		let descriptor = self.bindings.get_mut(&slot).ok_or(VmError::UnboundDescriptor { slot })?;

		match descriptor {
			DescriptorBinding::Texture(texture) => Ok(&mut **texture),
			DescriptorBinding::Buffer(_) => Err(VmError::DescriptorTypeMismatch {
				slot,
				expected: "texture",
				found: "buffer",
			}),
		}
	}
}

/// The `ExecutableProgram` struct stores the runnable VM form of a lexed BESL program.
pub struct ExecutableProgram {
	descriptor_layouts: HashMap<DescriptorSlot, DescriptorLayout>,
	functions: Vec<ExecutableFunction>,
	main_function: usize,
}

struct ExecutableFunction {
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	register_count: usize,
	parameter_count: usize,
	return_type: Option<ValueType>,
}

impl ExecutableProgram {
	/// Compiles a lexed BESL program into a runnable VM program.
	pub fn compile(program: NodeReference) -> Result<Self, VmError> {
		let main = resolve_main_function(&program)?;
		let main_signature = extract_function_signature(&main)?;
		if !main_signature.params.is_empty() {
			return Err(VmError::UnsupportedMainSignature {
				message: "Main functions with parameters are not supported".to_string(),
			});
		}
		if main_signature.return_type.is_some() {
			return Err(VmError::UnsupportedMainSignature {
				message: format!(
					"Main functions must return void, but found `{}`",
					main_signature.return_type.as_ref().map(ValueType::name).unwrap_or("void")
				),
			});
		}

		let function_nodes = collect_functions(&program, &main);
		let mut function_ids = HashMap::new();
		for (index, function) in function_nodes.iter().enumerate() {
			function_ids.insert(function.clone(), index);
		}

		let mut descriptor_layouts = HashMap::new();
		let mut functions = Vec::with_capacity(function_nodes.len());
		for function in &function_nodes {
			functions.push(Compiler::compile_function(function, &function_ids, &mut descriptor_layouts)?);
		}

		Ok(Self {
			descriptor_layouts,
			functions,
			main_function: function_ids[&main],
		})
	}

	pub fn descriptor_layout(&self, slot: DescriptorSlot) -> Option<&DescriptorLayout> {
		self.descriptor_layouts.get(&slot)
	}

	pub fn buffer_layout(&self, slot: DescriptorSlot) -> Option<&BufferLayout> {
		match self.descriptor_layouts.get(&slot) {
			Some(DescriptorLayout::Buffer(layout)) => Some(layout),
			Some(DescriptorLayout::Texture) => None,
			None => None,
		}
	}

	/// Executes the compiled `main` function using the currently bound descriptor resources.
	pub fn run_main(&self, descriptors: &mut DescriptorBindings<'_>) -> Result<(), VmError> {
		let return_value = self.execute_function(self.main_function, &[], descriptors)?;
		if return_value.is_some() {
			return Err(VmError::UnsupportedMainSignature {
				message: "Main functions must not return a value".to_string(),
			});
		}
		Ok(())
	}

	fn execute_function(
		&self,
		function_index: usize,
		arguments: &[ScalarValue],
		descriptors: &mut DescriptorBindings<'_>,
	) -> Result<Option<ScalarValue>, VmError> {
		let function = self
			.functions
			.get(function_index)
			.ok_or_else(|| VmError::UnsupportedExpression {
				message: format!("Unknown function index {}", function_index),
			})?;

		let mut registers = vec![None; function.register_count];
		let mut locals = vec![None; function.local_types.len()];
		if arguments.len() != function.parameter_count {
			return Err(VmError::CallArgumentMismatch {
				expected: function.parameter_count,
				found: arguments.len(),
			});
		}
		for (index, argument) in arguments.iter().enumerate() {
			locals[index] = Some(argument.clone());
		}

		for instruction in &function.instructions {
			match instruction {
				Instruction::LoadLiteral { register, value } => {
					registers[*register] = Some(value.clone());
				}
				Instruction::Construct {
					register,
					value_type,
					components,
				} => {
					let values = components
						.iter()
						.map(|component| read_register(&registers, *component))
						.collect::<Result<Vec<_>, _>>()?;
					registers[*register] = Some(construct_value(value_type, &values)?);
				}
				Instruction::Arithmetic {
					register,
					operator,
					left,
					right,
				} => {
					let left = read_register(&registers, *left)?;
					let right = read_register(&registers, *right)?;
					registers[*register] = Some(apply_arithmetic(*operator, &left, &right)?);
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
					let value = descriptors.buffer_mut(*slot)?.read_value(*offset, value_type)?;
					registers[*register] = Some(value);
				}
				Instruction::FetchTexture { register, slot, coord } => {
					let coord = read_register(&registers, *coord)?;
					let Value::Vec2U(coord) = coord else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2U.name().to_string(),
							found: coord.value_type().name().to_string(),
						});
					};

					registers[*register] = Some(descriptors.texture_mut(*slot)?.fetch(coord)?);
				}
				Instruction::SampleTexture { register, slot, uv } => {
					let uv = read_register(&registers, *uv)?;
					let Value::Vec2F(uv) = uv else {
						return Err(VmError::TypeMismatch {
							expected: ValueType::Vec2F.name().to_string(),
							found: uv.value_type().name().to_string(),
						});
					};

					registers[*register] = Some(descriptors.texture_mut(*slot)?.sample(uv)?);
				}
				Instruction::StoreBuffer {
					slot,
					offset,
					value_type,
					register,
				} => {
					let value = read_register(&registers, *register)?;
					descriptors.buffer_mut(*slot)?.write_value(*offset, value_type, &value)?;
				}
				Instruction::Call {
					register,
					function,
					arguments,
				} => {
					let arguments = arguments
						.iter()
						.map(|argument| read_register(&registers, *argument))
						.collect::<Result<Vec<_>, _>>()?;
					let value = self.execute_function(*function, &arguments, descriptors)?;
					registers[*register] = value;
				}
				Instruction::Return { register } => {
					return match register {
						Some(register) => Ok(Some(read_register(&registers, *register)?)),
						None => Ok(None),
					};
				}
			}
		}

		match &function.return_type {
			Some(return_type) => Err(VmError::UnsupportedStatement {
				message: format!(
					"Function with return type `{}` ended without returning a value",
					return_type.name()
				),
			}),
			None => Ok(None),
		}
	}
}

/// The `Value` enum stores the VM values that can move between registers, locals, and buffers.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
	U8(u8),
	U16(u16),
	U32(u32),
	I32(i32),
	F32(f32),
	Vec2U([u32; 2]),
	Vec2F([f32; 2]),
	Vec3F([f32; 3]),
	Vec4F([f32; 4]),
	Mat4F([f32; 16]),
}

impl Value {
	fn value_type(&self) -> ValueType {
		match self {
			Value::U8(_) => ValueType::U8,
			Value::U16(_) => ValueType::U16,
			Value::U32(_) => ValueType::U32,
			Value::I32(_) => ValueType::I32,
			Value::F32(_) => ValueType::F32,
			Value::Vec2U(_) => ValueType::Vec2U,
			Value::Vec2F(_) => ValueType::Vec2F,
			Value::Vec3F(_) => ValueType::Vec3F,
			Value::Vec4F(_) => ValueType::Vec4F,
			Value::Mat4F(_) => ValueType::Mat4F,
		}
	}
}

type ScalarValue = Value;

#[derive(Clone, Debug, PartialEq)]
enum Instruction {
	LoadLiteral {
		register: usize,
		value: ScalarValue,
	},
	Construct {
		register: usize,
		value_type: ValueType,
		components: Vec<usize>,
	},
	Arithmetic {
		register: usize,
		operator: ArithmeticOperator,
		left: usize,
		right: usize,
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
	FetchTexture {
		register: usize,
		slot: DescriptorSlot,
		coord: usize,
	},
	SampleTexture {
		register: usize,
		slot: DescriptorSlot,
		uv: usize,
	},
	StoreBuffer {
		slot: DescriptorSlot,
		offset: usize,
		value_type: ValueType,
		register: usize,
	},
	Call {
		register: usize,
		function: usize,
		arguments: Vec<usize>,
	},
	Return {
		register: Option<usize>,
	},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArithmeticOperator {
	Add,
	Subtract,
	Multiply,
	Divide,
	Modulo,
}

struct Compiler {
	function_ids: HashMap<NodeReference, usize>,
	instructions: Vec<Instruction>,
	local_types: Vec<ValueType>,
	locals_by_reference: HashMap<NodeReference, usize>,
	register_count: usize,
	return_type: Option<ValueType>,
	parameter_count: usize,
}

impl Compiler {
	fn compile_function(
		function: &NodeReference,
		function_ids: &HashMap<NodeReference, usize>,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<ExecutableFunction, VmError> {
		let signature = extract_function_signature(function)?;
		let mut compiler = Self {
			function_ids: function_ids.clone(),
			instructions: Vec::new(),
			local_types: Vec::new(),
			locals_by_reference: HashMap::new(),
			register_count: 0,
			return_type: signature.return_type.clone(),
			parameter_count: signature.params.len(),
		};

		for (index, param) in signature.params.iter().enumerate() {
			compiler.local_types.push(param.value_type.clone());
			compiler.locals_by_reference.insert(param.node.clone(), index);
		}

		for statement in &signature.statements {
			compiler.compile_statement(statement, descriptor_layouts)?;
		}

		if !matches!(compiler.instructions.last(), Some(Instruction::Return { .. })) {
			compiler.instructions.push(Instruction::Return { register: None });
		}

		Ok(ExecutableFunction {
			instructions: compiler.instructions,
			local_types: compiler.local_types,
			register_count: compiler.register_count,
			parameter_count: compiler.parameter_count,
			return_type: compiler.return_type,
		})
	}

	/// Compiles one BESL statement into bytecode while tracking locals and descriptors.
	fn compile_statement(
		&mut self,
		statement: &NodeReference,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
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
				self.compile_assignment(statement, left, right, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Return { value }) => {
				let value = value.clone();
				drop(borrowed);
				self.compile_return_statement(value.as_ref(), descriptor_layouts)
			}
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				let function = function.clone();
				let parameters = parameters.clone();
				drop(borrowed);
				self.compile_call_statement(&function, &parameters, descriptor_layouts)
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
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let left_expression = left.borrow();

		match left_expression.node() {
			Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
				let name = name.clone();
				let value_type = resolve_value_type(r#type)?;
				drop(left_expression);

				let local = self.define_local(statement.clone(), left, &name, value_type.clone());
				let register = self.compile_value_expression(&right, &value_type, descriptor_layouts)?;
				self.instructions.push(Instruction::StoreLocal { local, register });
				Ok(())
			}
			Nodes::Expression(Expressions::Accessor { .. }) => {
				drop(left_expression);

				let target = self.resolve_buffer_access(&left, RequiredAccess::Write, descriptor_layouts)?;
				let register = self.compile_value_expression(&right, &target.value_type, descriptor_layouts)?;
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
	fn compile_value_expression(
		&mut self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		let borrowed = expression.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
				let function = function.clone();
				let parameters = parameters.clone();
				drop(borrowed);
				self.compile_function_call_expression(&function, &parameters, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::IntrinsicCall {
				intrinsic, arguments, ..
			}) => {
				let intrinsic = intrinsic.clone();
				let arguments = arguments.clone();
				drop(borrowed);
				self.compile_intrinsic_call_expression(&intrinsic, &arguments, expected_type, descriptor_layouts)
			}
			Nodes::Expression(Expressions::Operator { operator, left, right }) => {
				let operator = arithmetic_operator(operator).ok_or_else(|| VmError::UnsupportedExpression {
					message: format!("Unsupported value operator: {:?}", operator),
				})?;
				let left = left.clone();
				let right = right.clone();
				drop(borrowed);

				let left_type = self.infer_expression_type(&left, expected_type, descriptor_layouts)?;
				let right_type = self.infer_expression_type(&right, expected_type, descriptor_layouts)?;
				let (left_expected_type, right_expected_type) = if left_type == *expected_type && right_type == *expected_type {
					(expected_type.clone(), expected_type.clone())
				} else if supports_scalar_broadcast(expected_type)
					&& left_type == ValueType::F32
					&& right_type == *expected_type
				{
					(ValueType::F32, expected_type.clone())
				} else if supports_scalar_broadcast(expected_type)
					&& left_type == *expected_type
					&& right_type == ValueType::F32
				{
					(expected_type.clone(), ValueType::F32)
				} else {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: format!("{} and {}", left_type.name(), right_type.name()),
					});
				};

				let left = self.compile_value_expression(&left, &left_expected_type, descriptor_layouts)?;
				let right = self.compile_value_expression(&right, &right_expected_type, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::Arithmetic {
					register,
					operator,
					left,
					right,
				});
				Ok(register)
			}
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

				let target = self.resolve_buffer_access(expression, RequiredAccess::Read, descriptor_layouts)?;
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

	fn compile_intrinsic_call_expression(
		&mut self,
		intrinsic: &NodeReference,
		arguments: &[NodeReference],
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		let intrinsic_ref = intrinsic.borrow();
		let (name, return_type) = match intrinsic_ref.node() {
			Nodes::Intrinsic { name, r#return, .. } => (name.clone(), resolve_value_type(r#return)?),
			node => {
				return Err(VmError::UnsupportedExpression {
					message: format!("Expected an intrinsic, but found {}", describe_node(node)),
				});
			}
		};
		drop(intrinsic_ref);

		if &return_type != expected_type {
			return Err(VmError::TypeMismatch {
				expected: expected_type.name().to_string(),
				found: return_type.name().to_string(),
			});
		}

		match name.as_str() {
			"sample" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let uv = self.compile_value_expression(&arguments[1], &ValueType::Vec2F, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::SampleTexture { register, slot, uv });
				Ok(register)
			}
			"fetch" => {
				if arguments.len() != 2 {
					return Err(VmError::CallArgumentMismatch {
						expected: 2,
						found: arguments.len(),
					});
				}

				let slot = self.resolve_texture_slot(&arguments[0], RequiredAccess::Read, descriptor_layouts)?;
				let coord = self.compile_value_expression(&arguments[1], &ValueType::Vec2U, descriptor_layouts)?;
				let register = self.allocate_register();
				self.instructions.push(Instruction::FetchTexture { register, slot, coord });
				Ok(register)
			}
			_ => Err(VmError::UnsupportedExpression {
				message: format!("Unsupported intrinsic `{}`", name),
			}),
		}
	}

	fn compile_function_call_expression(
		&mut self,
		function: &NodeReference,
		parameters: &[NodeReference],
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		let function_ref = function.borrow();
		match function_ref.node() {
			Nodes::Struct { fields, .. } => {
				let constructor_type = resolve_value_type(function)?;
				let fields = fields.clone();
				drop(function_ref);
				self.compile_constructor_expression(
					function,
					parameters,
					expected_type,
					constructor_type,
					&fields,
					descriptor_layouts,
				)
			}
			Nodes::Function { .. } => {
				let signature = extract_function_signature(function)?;
				drop(function_ref);
				let return_type = signature.return_type.ok_or_else(|| VmError::UnsupportedExpression {
					message: "Void functions cannot be used as value expressions".to_string(),
				})?;
				if &return_type != expected_type {
					return Err(VmError::TypeMismatch {
						expected: expected_type.name().to_string(),
						found: return_type.name().to_string(),
					});
				}

				let mut arguments = Vec::with_capacity(parameters.len());
				for (parameter, signature_parameter) in parameters.iter().zip(&signature.params) {
					arguments.push(self.compile_value_expression(
						parameter,
						&signature_parameter.value_type,
						descriptor_layouts,
					)?);
				}
				if parameters.len() != signature.params.len() {
					return Err(VmError::CallArgumentMismatch {
						expected: signature.params.len(),
						found: parameters.len(),
					});
				}

				let register = self.allocate_register();
				self.instructions.push(Instruction::Call {
					register,
					function: *self
						.function_ids
						.get(function)
						.ok_or_else(|| VmError::UnsupportedExpression {
							message: "Unknown function reference".to_string(),
						})?,
					arguments,
				});
				Ok(register)
			}
			node => Err(VmError::UnsupportedExpression {
				message: format!("Expected a callable value, but found {}", describe_node(node)),
			}),
		}
	}

	fn compile_constructor_expression(
		&mut self,
		_function: &NodeReference,
		parameters: &[NodeReference],
		expected_type: &ValueType,
		constructor_type: ValueType,
		fields: &[NodeReference],
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<usize, VmError> {
		if &constructor_type != expected_type {
			return Err(VmError::TypeMismatch {
				expected: expected_type.name().to_string(),
				found: constructor_type.name().to_string(),
			});
		}

		if fields.len() != parameters.len() {
			return Err(VmError::UnsupportedExpression {
				message: format!(
					"Constructor for `{}` expected {} parameters, but found {}",
					expected_type.name(),
					fields.len(),
					parameters.len()
				),
			});
		}

		let mut components = Vec::with_capacity(parameters.len());
		for (field, parameter) in fields.iter().zip(parameters) {
			let field_type = match field.borrow().node() {
				Nodes::Member { r#type, .. } => resolve_value_type(r#type)?,
				node => {
					return Err(VmError::UnsupportedExpression {
						message: format!("Expected a constructor field, but found {}", describe_node(node)),
					});
				}
			};
			components.push(self.compile_value_expression(parameter, &field_type, descriptor_layouts)?);
		}

		let register = self.allocate_register();
		self.instructions.push(Instruction::Construct {
			register,
			value_type: constructor_type,
			components,
		});
		Ok(register)
	}

	fn infer_expression_type(
		&mut self,
		expression: &NodeReference,
		expected_type: &ValueType,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<ValueType, VmError> {
		let borrowed = expression.borrow();
		match borrowed.node() {
			Nodes::Expression(Expressions::Literal { .. }) => {
				if supports_scalar_broadcast(expected_type) {
					Ok(ValueType::F32)
				} else {
					Ok(expected_type.clone())
				}
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

				self.local_types
					.get(local)
					.cloned()
					.ok_or(VmError::UninitializedLocal { local })
			}
			Nodes::Expression(Expressions::Accessor { .. }) => {
				drop(borrowed);
				Ok(self
					.resolve_buffer_access(expression, RequiredAccess::Read, descriptor_layouts)?
					.value_type)
			}
			Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
				let intrinsic = intrinsic.clone();
				drop(borrowed);
				resolve_callable_return_type(&intrinsic)
			}
			Nodes::Expression(Expressions::FunctionCall { function, .. }) => resolve_callable_return_type(function),
			Nodes::Expression(Expressions::Operator { .. }) => Ok(expected_type.clone()),
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
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
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

		let member = layout.member(&member_name).ok_or_else(|| VmError::UnknownBufferMember {
			member: member_name.clone(),
		})?;

		Ok(ResolvedBufferAccess {
			slot,
			offset: member.offset,
			value_type: member.value_type.clone(),
		})
	}

	fn resolve_texture_slot(
		&mut self,
		expression: &NodeReference,
		access: RequiredAccess,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<DescriptorSlot, VmError> {
		let binding = extract_binding_reference(expression)?;

		let binding_ref = binding.borrow();
		let slot = match binding_ref.node() {
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

	fn compile_call_statement(
		&mut self,
		function: &NodeReference,
		parameters: &[NodeReference],
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		let function_ref = function.borrow();
		match function_ref.node() {
			Nodes::Function { .. } => {
				let signature = extract_function_signature(function)?;
				drop(function_ref);
				let mut arguments = Vec::with_capacity(parameters.len());
				for (parameter, signature_parameter) in parameters.iter().zip(&signature.params) {
					arguments.push(self.compile_value_expression(
						parameter,
						&signature_parameter.value_type,
						descriptor_layouts,
					)?);
				}
				if parameters.len() != signature.params.len() {
					return Err(VmError::CallArgumentMismatch {
						expected: signature.params.len(),
						found: parameters.len(),
					});
				}

				let register = self.allocate_register();
				self.instructions.push(Instruction::Call {
					register,
					function: *self
						.function_ids
						.get(function)
						.ok_or_else(|| VmError::UnsupportedExpression {
							message: "Unknown function reference".to_string(),
						})?,
					arguments,
				});
				Ok(())
			}
			node => Err(VmError::UnsupportedStatement {
				message: format!("Expected a function call statement, but found {}", describe_node(node)),
			}),
		}
	}

	fn compile_return_statement(
		&mut self,
		value: Option<&NodeReference>,
		descriptor_layouts: &mut HashMap<DescriptorSlot, DescriptorLayout>,
	) -> Result<(), VmError> {
		match (self.return_type.clone(), value) {
			(None, None) => {
				self.instructions.push(Instruction::Return { register: None });
				Ok(())
			}
			(None, Some(_)) => Err(VmError::UnsupportedStatement {
				message: "Void functions cannot return a value".to_string(),
			}),
			(Some(return_type), Some(value)) => {
				let register = self.compile_value_expression(value, &return_type, descriptor_layouts)?;
				self.instructions.push(Instruction::Return {
					register: Some(register),
				});
				Ok(())
			}
			(Some(return_type), None) => Err(VmError::UnsupportedStatement {
				message: format!("Function with return type `{}` must return a value", return_type.name()),
			}),
		}
	}
}

struct ResolvedBufferAccess {
	slot: DescriptorSlot,
	offset: usize,
	value_type: ValueType,
}

struct FunctionParameter {
	node: NodeReference,
	value_type: ValueType,
}

struct FunctionSignature {
	params: Vec<FunctionParameter>,
	return_type: Option<ValueType>,
	statements: Vec<NodeReference>,
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

fn collect_functions(program: &NodeReference, main: &NodeReference) -> Vec<NodeReference> {
	let mut functions = Vec::new();
	if let Some(children) = program.get_children() {
		for child in children {
			if matches!(child.borrow().node(), Nodes::Function { .. }) {
				functions.push(child);
			}
		}
	}

	if functions.is_empty() {
		functions.push(main.clone());
	}

	functions
}

fn extract_function_signature(function: &NodeReference) -> Result<FunctionSignature, VmError> {
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

fn resolve_function_return_type(return_type: &NodeReference) -> Result<Option<ValueType>, VmError> {
	if return_type.borrow().get_name() == Some("void") {
		Ok(None)
	} else {
		Ok(Some(resolve_value_type(return_type)?))
	}
}

fn resolve_callable_return_type(callable: &NodeReference) -> Result<ValueType, VmError> {
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
		"vec2u" => Ok(ValueType::Vec2U),
		"vec2f" => Ok(ValueType::Vec2F),
		"vec3f" => Ok(ValueType::Vec3F),
		"vec4f" => Ok(ValueType::Vec4F),
		"mat4f" => Ok(ValueType::Mat4F),
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
		Nodes::Binding { .. } => Ok(expression.clone()),
		Nodes::Expression(Expressions::Member { source, .. }) => {
			let source = source.clone();
			drop(borrowed);

			let result = match source.borrow().node() {
				Nodes::Binding { .. } => Ok(source.clone()),
				Nodes::Expression(Expressions::Member { .. }) => extract_binding_reference(&source),
				_ => Err(VmError::UnsupportedExpression {
					message: format!(
						"Only direct binding member access is supported, but found {}",
						describe_node(source.borrow().node())
					),
				}),
			};

			result
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
		ValueType::Vec2U => {
			return Err(VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			});
		}
		ValueType::F32 => value
			.parse::<f32>()
			.map(ScalarValue::F32)
			.map_err(|_| VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			})?,
		ValueType::Vec2F | ValueType::Vec3F | ValueType::Vec4F | ValueType::Mat4F => {
			return Err(VmError::InvalidLiteral {
				value: value.to_string(),
				value_type: value_type.name().to_string(),
			});
		}
	};

	Ok(parsed)
}

fn construct_value(value_type: &ValueType, components: &[ScalarValue]) -> Result<ScalarValue, VmError> {
	match value_type {
		ValueType::Vec2U => Ok(ScalarValue::Vec2U(extract_u32_components::<2>(components)?)),
		ValueType::Vec2F => Ok(ScalarValue::Vec2F(extract_f32_components::<2>(components)?)),
		ValueType::Vec3F => Ok(ScalarValue::Vec3F(extract_f32_components::<3>(components)?)),
		ValueType::Vec4F => Ok(ScalarValue::Vec4F(extract_f32_components::<4>(components)?)),
		ValueType::Mat4F => {
			if components.len() != 4 {
				return Err(VmError::UnsupportedExpression {
					message: format!("Constructor for `{}` expected 4 vec4f parameters", value_type.name()),
				});
			}

			let mut values = [0.0; 16];
			for (index, component) in components.iter().enumerate() {
				let ScalarValue::Vec4F(component) = component else {
					return Err(VmError::TypeMismatch {
						expected: ValueType::Vec4F.name().to_string(),
						found: component.value_type().name().to_string(),
					});
				};
				values[index * 4..(index + 1) * 4].copy_from_slice(component);
			}

			Ok(ScalarValue::Mat4F(values))
		}
		_ => Err(VmError::UnsupportedExpression {
			message: format!("`{}` is not a constructor-backed VM value type", value_type.name()),
		}),
	}
}

fn extract_f32_components<const N: usize>(components: &[ScalarValue]) -> Result<[f32; N], VmError> {
	if components.len() != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} constructor parameters, but found {}", N, components.len()),
		});
	}

	let mut values = [0.0; N];
	for (index, component) in components.iter().enumerate() {
		let ScalarValue::F32(component) = component else {
			return Err(VmError::TypeMismatch {
				expected: ValueType::F32.name().to_string(),
				found: component.value_type().name().to_string(),
			});
		};
		values[index] = *component;
	}

	Ok(values)
}

fn extract_u32_components<const N: usize>(components: &[ScalarValue]) -> Result<[u32; N], VmError> {
	if components.len() != N {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} constructor parameters, but found {}", N, components.len()),
		});
	}

	let mut values = [0; N];
	for (index, component) in components.iter().enumerate() {
		let ScalarValue::U32(component) = component else {
			return Err(VmError::TypeMismatch {
				expected: ValueType::U32.name().to_string(),
				found: component.value_type().name().to_string(),
			});
		};
		values[index] = *component;
	}

	Ok(values)
}

fn read_f32_array<const N: usize>(bytes: &[u8]) -> Result<[f32; N], VmError> {
	if bytes.len() != N * 4 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} f32 values, but found {}", N * 4, N, bytes.len()),
		});
	}

	let mut values = [0.0; N];
	for (index, chunk) in bytes.chunks_exact(4).enumerate() {
		values[index] = f32::from_ne_bytes(chunk.try_into().expect("Invalid f32 byte count"));
	}
	Ok(values)
}

fn read_u32_array<const N: usize>(bytes: &[u8]) -> Result<[u32; N], VmError> {
	if bytes.len() != N * 4 {
		return Err(VmError::UnsupportedExpression {
			message: format!("Expected {} bytes for {} u32 values, but found {}", N * 4, N, bytes.len()),
		});
	}

	let mut values = [0; N];
	for (index, chunk) in bytes.chunks_exact(4).enumerate() {
		values[index] = u32::from_ne_bytes(chunk.try_into().expect("Invalid u32 byte count"));
	}
	Ok(values)
}

fn write_f32_slice(values: &[f32]) -> Vec<u8> {
	let mut bytes = Vec::with_capacity(values.len() * 4);
	for value in values {
		bytes.extend_from_slice(&value.to_ne_bytes());
	}
	bytes
}

fn write_u32_slice(values: &[u32]) -> Vec<u8> {
	let mut bytes = Vec::with_capacity(values.len() * 4);
	for value in values {
		bytes.extend_from_slice(&value.to_ne_bytes());
	}
	bytes
}

fn lerp_rgba(left: [f32; 4], right: [f32; 4], factor: f32) -> [f32; 4] {
	let mut value = [0.0; 4];
	for index in 0..4 {
		value[index] = left[index] + (right[index] - left[index]) * factor;
	}
	value
}

fn arithmetic_operator(operator: &Operators) -> Option<ArithmeticOperator> {
	match operator {
		Operators::Plus => Some(ArithmeticOperator::Add),
		Operators::Minus => Some(ArithmeticOperator::Subtract),
		Operators::Multiply => Some(ArithmeticOperator::Multiply),
		Operators::Divide => Some(ArithmeticOperator::Divide),
		Operators::Modulo => Some(ArithmeticOperator::Modulo),
		Operators::Assignment | Operators::Equality => None,
	}
}

fn supports_scalar_broadcast(value_type: &ValueType) -> bool {
	matches!(
		value_type,
		ValueType::Vec2F | ValueType::Vec3F | ValueType::Vec4F | ValueType::Mat4F
	)
}

fn apply_arithmetic(operator: ArithmeticOperator, left: &ScalarValue, right: &ScalarValue) -> Result<ScalarValue, VmError> {
	match (left, right) {
		(ScalarValue::U8(left), ScalarValue::U8(right)) => {
			apply_integer_arithmetic(*left, *right, operator).map(ScalarValue::U8)
		}
		(ScalarValue::U16(left), ScalarValue::U16(right)) => {
			apply_integer_arithmetic(*left, *right, operator).map(ScalarValue::U16)
		}
		(ScalarValue::U32(left), ScalarValue::U32(right)) => {
			apply_integer_arithmetic(*left, *right, operator).map(ScalarValue::U32)
		}
		(ScalarValue::I32(left), ScalarValue::I32(right)) => {
			apply_integer_arithmetic(*left, *right, operator).map(ScalarValue::I32)
		}
		(ScalarValue::F32(left), ScalarValue::F32(right)) => {
			apply_float_arithmetic(*left, *right, operator).map(ScalarValue::F32)
		}
		(ScalarValue::Vec2F(left), ScalarValue::Vec2F(right)) => {
			apply_float_array_arithmetic::<2>(*left, *right, operator).map(ScalarValue::Vec2F)
		}
		(ScalarValue::Vec3F(left), ScalarValue::Vec3F(right)) => {
			apply_float_array_arithmetic::<3>(*left, *right, operator).map(ScalarValue::Vec3F)
		}
		(ScalarValue::Vec4F(left), ScalarValue::Vec4F(right)) => {
			apply_float_array_arithmetic::<4>(*left, *right, operator).map(ScalarValue::Vec4F)
		}
		(ScalarValue::Mat4F(left), ScalarValue::Mat4F(right)) => {
			apply_float_array_arithmetic::<16>(*left, *right, operator).map(ScalarValue::Mat4F)
		}
		(ScalarValue::Vec2F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<2>(*left, *right, operator).map(ScalarValue::Vec2F)
		}
		(ScalarValue::Vec3F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<3>(*left, *right, operator).map(ScalarValue::Vec3F)
		}
		(ScalarValue::Vec4F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<4>(*left, *right, operator).map(ScalarValue::Vec4F)
		}
		(ScalarValue::Mat4F(left), ScalarValue::F32(right)) => {
			apply_float_scalar_broadcast::<16>(*left, *right, operator).map(ScalarValue::Mat4F)
		}
		(ScalarValue::F32(left), ScalarValue::Vec2F(right)) => {
			apply_scalar_float_broadcast::<2>(*left, *right, operator).map(ScalarValue::Vec2F)
		}
		(ScalarValue::F32(left), ScalarValue::Vec3F(right)) => {
			apply_scalar_float_broadcast::<3>(*left, *right, operator).map(ScalarValue::Vec3F)
		}
		(ScalarValue::F32(left), ScalarValue::Vec4F(right)) => {
			apply_scalar_float_broadcast::<4>(*left, *right, operator).map(ScalarValue::Vec4F)
		}
		(ScalarValue::F32(left), ScalarValue::Mat4F(right)) => {
			apply_scalar_float_broadcast::<16>(*left, *right, operator).map(ScalarValue::Mat4F)
		}
		(left, right) => Err(VmError::TypeMismatch {
			expected: left.value_type().name().to_string(),
			found: right.value_type().name().to_string(),
		}),
	}
}

fn apply_integer_arithmetic<T>(left: T, right: T, operator: ArithmeticOperator) -> Result<T, VmError>
where
	T: Copy
		+ std::ops::Add<Output = T>
		+ std::ops::Sub<Output = T>
		+ std::ops::Mul<Output = T>
		+ std::ops::Div<Output = T>
		+ std::ops::Rem<Output = T>
		+ PartialEq
		+ Default,
{
	let zero = T::default();
	match operator {
		ArithmeticOperator::Add => Ok(left + right),
		ArithmeticOperator::Subtract => Ok(left - right),
		ArithmeticOperator::Multiply => Ok(left * right),
		ArithmeticOperator::Divide => {
			if right == zero {
				return Err(VmError::ArithmeticError {
					message: "Division by zero".to_string(),
				});
			}
			Ok(left / right)
		}
		ArithmeticOperator::Modulo => {
			if right == zero {
				return Err(VmError::ArithmeticError {
					message: "Modulo by zero".to_string(),
				});
			}
			Ok(left % right)
		}
	}
}

fn apply_float_arithmetic(left: f32, right: f32, operator: ArithmeticOperator) -> Result<f32, VmError> {
	match operator {
		ArithmeticOperator::Add => Ok(left + right),
		ArithmeticOperator::Subtract => Ok(left - right),
		ArithmeticOperator::Multiply => Ok(left * right),
		ArithmeticOperator::Divide => Ok(left / right),
		ArithmeticOperator::Modulo => Ok(left % right),
	}
}

fn apply_float_array_arithmetic<const N: usize>(
	left: [f32; N],
	right: [f32; N],
	operator: ArithmeticOperator,
) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	for index in 0..N {
		values[index] = apply_float_arithmetic(left[index], right[index], operator)?;
	}
	Ok(values)
}

fn apply_float_scalar_broadcast<const N: usize>(
	left: [f32; N],
	right: f32,
	operator: ArithmeticOperator,
) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	for index in 0..N {
		values[index] = apply_float_arithmetic(left[index], right, operator)?;
	}
	Ok(values)
}

fn apply_scalar_float_broadcast<const N: usize>(
	left: f32,
	right: [f32; N],
	operator: ArithmeticOperator,
) -> Result<[f32; N], VmError> {
	let mut values = [0.0; N];
	for index in 0..N {
		values[index] = apply_float_arithmetic(left, right[index], operator)?;
	}
	Ok(values)
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
	UnsupportedMainSignature {
		message: String,
	},
	UnsupportedType {
		type_name: String,
	},
	UnsupportedStatement {
		message: String,
	},
	UnsupportedAssignmentTarget {
		message: String,
	},
	UnsupportedExpression {
		message: String,
	},
	UnsupportedBufferLayout {
		message: String,
	},
	UnsupportedDescriptor {
		slot: DescriptorSlot,
		message: String,
	},
	DescriptorAccessDenied {
		slot: DescriptorSlot,
		access: &'static str,
	},
	DescriptorTypeMismatch {
		slot: DescriptorSlot,
		expected: &'static str,
		found: &'static str,
	},
	UnknownBufferMember {
		member: String,
	},
	UnboundDescriptor {
		slot: DescriptorSlot,
	},
	CallArgumentMismatch {
		expected: usize,
		found: usize,
	},
	BufferAccessOutOfBounds {
		offset: usize,
		size: usize,
		buffer_size: usize,
	},
	TextureAccessOutOfBounds {
		x: u32,
		y: u32,
		width: u32,
		height: u32,
	},
	InvalidTextureDimensions {
		width: u32,
		height: u32,
	},
	InvalidLiteral {
		value: String,
		value_type: String,
	},
	ArithmeticError {
		message: String,
	},
	TypeMismatch {
		expected: String,
		found: String,
	},
	UninitializedRegister {
		register: usize,
	},
	UninitializedLocal {
		local: usize,
	},
}

impl std::fmt::Display for VmError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			VmError::MissingMainFunction => {
				write!(
					f,
					"Missing main function. The most likely cause is that the lexed BESL program does not define `main`."
				)
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
			VmError::DescriptorTypeMismatch { slot, expected, found } => write!(
				f,
				"Descriptor type mismatch at set {} binding {}: expected `{}` but found `{}`. The most likely cause is that the host bound a different resource kind than the compiled BESL program requires.",
				slot.set(),
				slot.binding(),
				expected,
				found
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
			VmError::CallArgumentMismatch { expected, found } => write!(
				f,
				"Function call argument mismatch: expected {} arguments but found {}. The most likely cause is that the BESL function call does not match the declared parameter list.",
				expected, found
			),
			VmError::BufferAccessOutOfBounds {
				offset,
				size,
				buffer_size,
			} => write!(
				f,
				"Buffer access out of bounds at byte {} for {} bytes in a {} byte buffer. The most likely cause is that the bound buffer does not match the compiled BESL buffer layout.",
				offset, size, buffer_size
			),
			VmError::TextureAccessOutOfBounds { x, y, width, height } => write!(
				f,
				"Texture access out of bounds at ({}, {}) in a {}x{} texture. The most likely cause is that the BESL program fetched a texel outside the bound texture dimensions.",
				x, y, width, height
			),
			VmError::InvalidTextureDimensions { width, height } => write!(
				f,
				"Invalid texture dimensions {}x{}. The most likely cause is that the host created a texture with zero width or height.",
				width, height
			),
			VmError::InvalidLiteral { value, value_type } => write!(
				f,
				"Invalid literal `{}` for `{}`. The most likely cause is that the literal cannot be parsed as the target BESL scalar type.",
				value, value_type
			),
			VmError::ArithmeticError { message } => write!(
				f,
				"Invalid arithmetic operation. {}. The most likely cause is that the BESL program evaluated an unsupported numeric operation such as division or modulo by zero.",
				message
			),
			VmError::TypeMismatch { expected, found } => write!(
				f,
				"Type mismatch: expected `{}` but found `{}`. The most likely cause is that the BESL assignment mixes incompatible scalar types.",
				expected, found
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

	use super::{Buffer, DescriptorBindings, DescriptorSlot, ExecutableProgram, Texture, Value};

	fn read_f32s(buffer: &Buffer, count: usize) -> Vec<f32> {
		buffer
			.bytes()
			.chunks_exact(4)
			.take(count)
			.map(|chunk| f32::from_ne_bytes(chunk.try_into().expect("Expected four bytes")))
			.collect()
	}

	fn write_texture(texture: &mut Texture, texels: &[([u32; 2], [f32; 4])]) {
		for (coord, value) in texels {
			texture.write(*coord, *value).expect("Expected texture write to succeed");
		}
	}

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

	#[test]
	fn executable_program_reads_a_bound_buffer_inside_main() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = input.value;
			output.value = value;
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");

		root.add_child(
			Node::binding(
				"input",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type.clone()).into()],
				},
				0,
				7,
				true,
				false,
			)
			.into(),
		);

		root.add_child(
			Node::binding(
				"output",
				BindingTypes::Buffer {
					members: vec![Node::member("value", float_type).into()],
				},
				0,
				8,
				false,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let input_slot = DescriptorSlot::new(0, 7);
		let output_slot = DescriptorSlot::new(0, 8);
		let input_layout = executable
			.buffer_layout(input_slot)
			.expect("Expected input buffer layout")
			.clone();
		let output_layout = executable
			.buffer_layout(output_slot)
			.expect("Expected output buffer layout")
			.clone();
		let mut input = Buffer::new(input_layout);
		let mut output = Buffer::new(output_layout);
		input
			.write("value", Value::F32(9.25))
			.expect("Expected host buffer write to succeed");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(input_slot, &mut input);
			descriptors.bind_buffer(output_slot, &mut output);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(output.read_f32("value").expect("Expected f32 member"), 9.25);
	}

	#[test]
	fn executable_program_evaluates_addition_before_writing_to_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = 7.5;
			buff.value = value + 4.5;
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
				1,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let slot = DescriptorSlot::new(0, 1);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 12.0);
	}

	#[test]
	fn apply_arithmetic_supports_all_basic_scalar_operations() {
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Add,
				&super::ScalarValue::U32(2),
				&super::ScalarValue::U32(3)
			)
			.expect("Expected addition to succeed"),
			super::ScalarValue::U32(5)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Subtract,
				&super::ScalarValue::I32(9),
				&super::ScalarValue::I32(4)
			)
			.expect("Expected subtraction to succeed"),
			super::ScalarValue::I32(5)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Multiply,
				&super::ScalarValue::U16(6),
				&super::ScalarValue::U16(7)
			)
			.expect("Expected multiplication to succeed"),
			super::ScalarValue::U16(42)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Divide,
				&super::ScalarValue::F32(9.0),
				&super::ScalarValue::F32(2.0)
			)
			.expect("Expected division to succeed"),
			super::ScalarValue::F32(4.5)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Modulo,
				&super::ScalarValue::U8(20),
				&super::ScalarValue::U8(6)
			)
			.expect("Expected modulo to succeed"),
			super::ScalarValue::U8(2)
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Add,
				&super::ScalarValue::Vec3F([1.0, 2.0, 3.0]),
				&super::ScalarValue::Vec3F([4.0, 5.0, 6.0])
			)
			.expect("Expected vec3f addition to succeed"),
			super::ScalarValue::Vec3F([5.0, 7.0, 9.0])
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Multiply,
				&super::ScalarValue::Vec4F([1.0, 2.0, 3.0, 4.0]),
				&super::ScalarValue::F32(2.0)
			)
			.expect("Expected vec4f scalar broadcast to succeed"),
			super::ScalarValue::Vec4F([2.0, 4.0, 6.0, 8.0])
		);
		assert_eq!(
			super::apply_arithmetic(
				super::ArithmeticOperator::Add,
				&super::ScalarValue::Mat4F([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,]),
				&super::ScalarValue::F32(1.0)
			)
			.expect("Expected mat4f scalar broadcast to succeed"),
			super::ScalarValue::Mat4F([2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0,])
		);
	}

	#[test]
	fn executable_program_evaluates_vec3f_arithmetic_before_writing_to_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let lhs: vec3f = vec3f(1.0, 2.0, 3.0);
			let rhs: vec3f = vec3f(4.0, 5.0, 6.0);
			buff.value = lhs + rhs;
		}
		"#;

		let mut root = Node::root();
		let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec3f_type).into()],
				},
				0,
				2,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let slot = DescriptorSlot::new(0, 2);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_f32s(&buffer, 3), vec![5.0, 7.0, 9.0]);
	}

	#[test]
	fn executable_program_evaluates_vec4f_scalar_broadcast_arithmetic() {
		let script = r#"
		main: fn () -> void {
			let value: vec4f = vec4f(1.0, 2.0, 3.0, 4.0);
			buff.value = value * 2.0;
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec4f_type).into()],
				},
				0,
				3,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let slot = DescriptorSlot::new(0, 3);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_f32s(&buffer, 4), vec![2.0, 4.0, 6.0, 8.0]);
	}

	#[test]
	fn executable_program_evaluates_mat4f_arithmetic_before_writing_to_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let lhs: mat4f = mat4f(
				vec4f(1.0, 0.0, 0.0, 0.0),
				vec4f(0.0, 1.0, 0.0, 0.0),
				vec4f(0.0, 0.0, 1.0, 0.0),
				vec4f(0.0, 0.0, 0.0, 1.0)
			);
			let rhs: mat4f = mat4f(
				vec4f(1.0, 1.0, 1.0, 1.0),
				vec4f(1.0, 1.0, 1.0, 1.0),
				vec4f(1.0, 1.0, 1.0, 1.0),
				vec4f(1.0, 1.0, 1.0, 1.0)
			);
			buff.value = lhs + rhs;
		}
		"#;

		let mut root = Node::root();
		let mat4f_type = root.get_child("mat4f").expect("Expected mat4f");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", mat4f_type).into()],
				},
				0,
				4,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let slot = DescriptorSlot::new(0, 4);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(
			read_f32s(&buffer, 16),
			vec![2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0,]
		);
	}

	#[test]
	fn executable_program_calls_function_with_parameters_and_return_value() {
		let script = r#"
		add: fn (lhs: f32, rhs: f32) -> f32 {
			return lhs + rhs;
		}

		main: fn () -> void {
			buff.value = add(3.0, 4.5);
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
				5,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let slot = DescriptorSlot::new(0, 5);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(buffer.read_f32("value").expect("Expected f32 member"), 7.5);
	}

	#[test]
	fn executable_program_calls_function_and_returns_vec3f() {
		let script = r#"
		double: fn (value: vec3f) -> vec3f {
			return value * 2.0;
		}

		main: fn () -> void {
			buff.value = double(vec3f(1.0, 2.0, 3.0));
		}
		"#;

		let mut root = Node::root();
		let vec3f_type = root.get_child("vec3f").expect("Expected vec3f");

		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec3f_type).into()],
				},
				0,
				6,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let slot = DescriptorSlot::new(0, 6);
		let layout = executable.buffer_layout(slot).expect("Expected buffer layout").clone();
		let mut buffer = Buffer::new(layout);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_f32s(&buffer, 3), vec![2.0, 4.0, 6.0]);
	}

	#[test]
	fn executable_program_fetches_texture_texels_into_a_bound_buffer_member() {
		let script = r#"
		main: fn () -> void {
			let coord: vec2u = vec2u(1, 0);
			buff.value = fetch(texture, coord);
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

		root.add_child(
			Node::binding(
				"texture",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				7,
				true,
				false,
			)
			.into(),
		);
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec4f_type).into()],
				},
				0,
				8,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let texture_slot = DescriptorSlot::new(0, 7);
		let buffer_slot = DescriptorSlot::new(0, 8);
		let layout = executable.buffer_layout(buffer_slot).expect("Expected buffer layout").clone();
		let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
		let mut buffer = Buffer::new(layout);
		write_texture(
			&mut texture,
			&[
				([0, 0], [1.0, 0.0, 0.0, 1.0]),
				([1, 0], [0.0, 1.0, 0.0, 1.0]),
				([0, 1], [0.0, 0.0, 1.0, 1.0]),
				([1, 1], [1.0, 1.0, 1.0, 1.0]),
			],
		);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(texture_slot, &mut texture);
			descriptors.bind_buffer(buffer_slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_f32s(&buffer, 4), vec![0.0, 1.0, 0.0, 1.0]);
	}

	#[test]
	fn executable_program_samples_textures_inside_arithmetic_expressions() {
		let script = r#"
		main: fn () -> void {
			let color: vec4f = sample(texture_sampler, vec2f(0.5, 0.5));
			buff.value = color * 2.0;
		}
		"#;

		let mut root = Node::root();
		let vec4f_type = root.get_child("vec4f").expect("Expected vec4f");

		root.add_child(
			Node::binding(
				"texture_sampler",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				9,
				true,
				false,
			)
			.into(),
		);
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::member("value", vec4f_type).into()],
				},
				0,
				10,
				true,
				true,
			)
			.into(),
		);

		let program = compile_to_besl(script, Some(root)).expect("Expected lexed program");
		let executable = ExecutableProgram::compile(program).expect("Expected runnable program");

		let texture_slot = DescriptorSlot::new(0, 9);
		let buffer_slot = DescriptorSlot::new(0, 10);
		let layout = executable.buffer_layout(buffer_slot).expect("Expected buffer layout").clone();
		let mut texture = Texture::new(2, 2).expect("Expected texture allocation");
		let mut buffer = Buffer::new(layout);
		write_texture(
			&mut texture,
			&[
				([0, 0], [0.0, 0.0, 0.0, 1.0]),
				([1, 0], [1.0, 0.0, 0.0, 1.0]),
				([0, 1], [0.0, 1.0, 0.0, 1.0]),
				([1, 1], [1.0, 1.0, 0.0, 1.0]),
			],
		);

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(texture_slot, &mut texture);
			descriptors.bind_buffer(buffer_slot, &mut buffer);
			executable.run_main(&mut descriptors).expect("Expected execution to succeed");
		}

		assert_eq!(read_f32s(&buffer, 4), vec![1.0, 1.0, 0.0, 2.0]);
	}
}
