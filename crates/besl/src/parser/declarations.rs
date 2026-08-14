//! Parses BESL tokens into syntax nodes that preserve the source structure.
//!
//! # Example shader
//!
//! ```glsl
//! Light: struct {
//!     position: vec3,
//!     color: vec3,
//! }
//!
//! main: fn () -> void {
//!     gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
//! }
//! ```
//!
//! Use [`crate::parse`] as the entry point. The parser records cross-references by name.
//! The [`crate::lexer`] module resolves those names later.

use super::expressions::{
	execute_parsers, parse_const, parse_descriptor, parse_function, parse_macro, parse_member, parse_push_constant,
	parse_shader_interface_declaration, parse_struct,
};
use crate::{lexer::BufferMemoryClass, tokenizer};

/// A shared syntax node in a parsed BESL tree.
pub type NodeReference<'a> = &'a Node<'a>;

/// The `TypeName` enum preserves type structure while the parser still borrows source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeName<'a> {
	Named(&'a str),
	Array { element: Box<TypeName<'a>>, count: u32 },
}

impl<'a> From<&'a str> for TypeName<'a> {
	fn from(name: &'a str) -> Self {
		Self::Named(name)
	}
}

impl std::fmt::Display for TypeName<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Named(name) => f.write_str(name),
			Self::Array { element, count } => write!(f, "{element}[{count}]"),
		}
	}
}

/// A weak syntax-node reference used to avoid ownership cycles.
pub(crate) fn parse<'i, 'a: 'i>(tokens: &'i tokenizer::Tokens<'a>) -> Result<Node<'a>, ParsingFailReasons> {
	let mut iterator = tokens.tokens.iter();

	let parsers = [
		parse_push_constant,
		parse_struct,
		parse_function,
		parse_macro,
		parse_const,
		parse_descriptor,
		parse_shader_interface_declaration,
		parse_member,
	];

	let mut children: Vec<Node<'a>> = Vec::with_capacity(64);

	loop {
		let (expression, iter) = execute_parsers(parsers.as_slice(), iterator)?;

		children.push(expression);

		iterator = iter;

		if iterator.len() == 0 {
			break;
		}
	}

	Ok(make_scope("root", children))
}

use std::borrow::Cow;
use std::num::{NonZeroU32, NonZeroUsize};

#[derive(Clone, Debug)]
pub struct Node<'a> {
	pub(crate) node: Nodes<'a>,
}

impl<'a> Node<'a> {
	pub fn root() -> Node<'a> {
		make_scope("root", Vec::new())
	}

	pub fn root_with_children(children: Vec<Node<'a>>) -> Node<'a> {
		make_scope("root", children)
	}

	pub fn scope(name: &'a str, children: Vec<Node<'a>>) -> Node<'a> {
		make_scope(name, children)
	}

	pub fn r#struct(name: &'a str, fields: Vec<Node<'a>>) -> Node<'a> {
		make_struct(name, fields)
	}

	pub fn member(name: &'a str, r#type: &'_ str) -> Node<'a> {
		make_member(name, r#type)
	}

	pub fn member_expression(name: impl Into<Cow<'a, str>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Member { name: name.into() }),
		}
	}

	pub fn function(
		name: &'a str,
		params: Vec<Node<'a>>,
		return_type: impl Into<TypeName<'a>>,
		statements: Vec<Node<'a>>,
	) -> Node<'a> {
		make_function(name, params, return_type, statements)
	}

	pub fn conditional(condition: Node<'a>, statements: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Conditional {
				condition: Box::new(condition),
				statements,
			},
		}
	}

	pub fn for_loop(initializer: Node<'a>, condition: Node<'a>, update: Node<'a>, statements: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::ForLoop {
				initializer: Box::new(initializer),
				condition: Box::new(condition),
				update: Box::new(update),
				statements,
			},
		}
	}

	pub fn main_function(statements: Vec<Node<'a>>) -> Node<'a> {
		make_function("main", Vec::new(), "void", statements)
	}

	pub fn binding(name: &'a str, r#type: Node<'a>, slot: u32, read: bool, write: bool) -> Node<'a> {
		Self::binding_with_count(name, r#type, slot, read, write, None, None)
	}

	/// Builds a buffer binding whose memory class is independent from its read and write access.
	pub fn binding_in_memory(
		name: &'a str,
		r#type: Node<'a>,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: BufferMemoryClass,
	) -> Node<'a> {
		Self::binding_with_count(name, r#type, slot, read, write, Some(memory_class), None)
	}

	/// Builds a buffer binding that stores thread-varying data in device memory.
	pub fn device_buffer_binding(name: &'a str, r#type: Node<'a>, slot: u32, read: bool, write: bool) -> Node<'a> {
		Self::binding_in_memory(name, r#type, slot, read, write, BufferMemoryClass::Device)
	}

	/// Builds a buffer binding that stores dispatch-shared values in constant memory.
	pub fn constant_buffer_binding(name: &'a str, r#type: Node<'a>, slot: u32, read: bool, write: bool) -> Node<'a> {
		Self::binding_in_memory(name, r#type, slot, read, write, BufferMemoryClass::Constant)
	}

	fn binding_with_count(
		name: &'a str,
		r#type: Node<'a>,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: Option<BufferMemoryClass>,
		count: Option<NonZeroUsize>,
	) -> Node<'a> {
		Node {
			node: Nodes::Binding {
				name,
				r#type: Box::new(r#type),
				slot,
				read,
				write,
				memory_class,
				count,
			},
		}
	}

	pub fn binding_array(name: &'a str, r#type: Node<'a>, slot: u32, read: bool, write: bool, count: u32) -> Node<'a> {
		let count = NonZeroUsize::new(count as usize).expect(
			"Invalid binding array count. The most likely cause is that a resource array was declared with zero elements.",
		);
		Self::binding_with_count(name, r#type, slot, read, write, None, Some(count))
	}

	pub fn specialization(name: &'a str, r#type: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Specialization { name, r#type },
		}
	}

	pub fn buffer(name: &'a str, members: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Type { name, members },
		}
	}

	pub fn image(format: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Image { format },
		}
	}

	pub fn push_constant(members: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::PushConstant { members },
		}
	}

	pub fn combined_image_sampler() -> Node<'a> {
		Node {
			node: Nodes::CombinedImageSampler { format: "" },
		}
	}

	pub fn combined_array_image_sampler() -> Node<'a> {
		Node {
			node: Nodes::CombinedImageSampler {
				format: "ArrayTexture2D",
			},
		}
	}

	pub fn combined_cube_image_sampler() -> Node<'a> {
		Node {
			node: Nodes::CombinedImageSampler { format: "TextureCube" },
		}
	}

	pub fn combined_cube_array_image_sampler() -> Node<'a> {
		Node {
			node: Nodes::CombinedImageSampler {
				format: "TextureCubeArray",
			},
		}
	}

	pub fn r#macro(name: &'a str, body: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Macro {
				name,
				body: Box::new(body),
			}),
		}
	}

	pub fn sentence(expressions: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Expression(expressions)),
		}
	}

	pub fn expression(elements: Vec<Node<'a>>) -> Node<'a> {
		Self::sentence(elements)
	}

	pub fn accessor(left: Node<'a>, right: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Accessor {
				left: Box::new(left),
				right: Box::new(right),
			}),
		}
	}

	pub fn call(name: &'a str, parameters: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Call {
				name: TypeName::Named(name),
				parameters,
			}),
		}
	}

	pub fn operator(name: &'a str, left: Node<'a>, right: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Operator {
				name,
				left: Box::new(left),
				right: Box::new(right),
			}),
		}
	}

	pub fn assignment(left: Node<'a>, right: Node<'a>) -> Node<'a> {
		Self::operator("=", left, right)
	}

	/// Builds a typed local declaration.
	///
	/// Generated programs may own their local names, while parsed programs continue to borrow source text.
	pub fn variable_declaration(name: impl Into<Cow<'a, str>>, r#type: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::VariableDeclaration {
				name: name.into(),
				r#type: TypeName::Named(r#type),
			}),
		}
	}

	pub fn literal_expression(value: impl Into<Cow<'a, str>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Literal { value: value.into() }),
		}
	}

	pub fn return_value(value: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Return {
				value: Some(Box::new(value)),
			}),
		}
	}

	pub fn return_void() -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Return { value: None }),
		}
	}

	pub fn let_assignment(name: impl Into<Cow<'a, str>>, r#type: &'a str, value: Node<'a>) -> Node<'a> {
		Self::assignment(Self::variable_declaration(name, r#type), value)
	}

	pub fn member_assignment(name: &'a str, value: Node<'a>) -> Node<'a> {
		Self::assignment(Self::member_expression(name), value)
	}

	pub fn glsl(code: impl Into<Cow<'a, str>>, input: &'a [&'a str], output: &'a [&'a str]) -> Node<'a> {
		Self::raw_code(Some(code.into()), None, None, input, output)
	}

	pub fn hlsl(code: impl Into<Cow<'a, str>>, input: &'a [&'a str], output: &'a [&'a str]) -> Node<'a> {
		Self::raw_code(None, Some(code.into()), None, input, output)
	}

	pub fn msl(code: impl Into<Cow<'a, str>>, input: &'a [&'a str], output: &'a [&'a str]) -> Node<'a> {
		Self::raw_code(None, None, Some(code.into()), input, output)
	}

	/// Builds parser raw code with explicit backend sources and interface names.
	pub fn raw_code(
		glsl: Option<Cow<'a, str>>,
		hlsl: Option<Cow<'a, str>>,
		msl: Option<Cow<'a, str>>,
		input: &'a [&'a str],
		output: &'a [&'a str],
	) -> Node<'a> {
		Node {
			node: Nodes::RawCode {
				glsl,
				hlsl,
				msl,
				input,
				output,
			},
		}
	}

	pub fn literal(name: &'a str, body: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Literal {
				name,
				body: Box::new(body),
			},
		}
	}

	pub fn input(name: &'a str, format: &'a str, location: u8) -> Node<'a> {
		Node {
			node: Nodes::Input { name, format, location },
		}
	}

	pub fn output(name: &'a str, format: &'a str, location: u8) -> Node<'a> {
		Self::output_with_count(name, format, location, None)
	}

	fn output_with_count(name: &'a str, format: &'a str, location: u8, count: Option<NonZeroUsize>) -> Node<'a> {
		Node {
			node: Nodes::Output {
				name,
				format,
				location,
				count,
			},
		}
	}

	pub fn output_array(name: &'a str, format: &'a str, location: u8, count: u32) -> Node<'a> {
		Self::output_with_count(name, format, location, NonZeroUsize::new(count as usize))
	}

	pub fn task_payload(name: &'a str, format: &'a str, count: u32) -> Node<'a> {
		let count = NonZeroUsize::new(count as usize).expect(
			"Invalid task-payload count. The most likely cause is that a task-payload array was declared with zero elements.",
		);
		Node {
			node: Nodes::TaskPayload { name, format, count },
		}
	}

	pub fn workgroup(name: &'a str, format: &'a str, count: Option<NonZeroUsize>) -> Node<'a> {
		Node {
			node: Nodes::Workgroup { name, format, count },
		}
	}

	pub fn intrinsic(name: &'a str, parameters: Node<'a>, body: Node<'a>, r#return: &'a str) -> Node<'a> {
		Self::intrinsic_with_parameters(name, vec![parameters], body, r#return)
	}

	/// Builds an intrinsic whose portable signature has more than one parameter.
	pub fn intrinsic_with_parameters(name: &'a str, parameters: Vec<Node<'a>>, body: Node<'a>, r#return: &'a str) -> Node<'a> {
		let mut elements = parameters;
		elements.push(body);
		Node {
			node: Nodes::Intrinsic {
				name,
				elements,
				r#return,
			},
		}
	}

	pub fn null() -> Node<'a> {
		Node { node: Nodes::Null }
	}

	pub fn parameter(name: &'a str, r#type: impl Into<TypeName<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Parameter {
				name,
				r#type: r#type.into(),
			},
		}
	}

	pub fn constant(name: &'a str, r#type: &'a str, value: Node<'a>) -> Node<'a> {
		Self::constant_with_type(name, TypeName::Named(r#type), value)
	}

	/// Builds a constant node while preserving the parsed type structure.
	pub(super) fn constant_with_type(name: &'a str, r#type: TypeName<'a>, value: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Const {
				name,
				r#type,
				value: Box::new(value),
			},
		}
	}

	pub fn name(&self) -> Option<&'a str> {
		match &self.node {
			Nodes::Scope { name, .. } => Some(name),
			Nodes::Struct { name, .. } => Some(name),
			Nodes::Member { name, .. } => Some(name),
			Nodes::Function { name, .. } => Some(name),
			Nodes::Conditional { .. } | Nodes::ForLoop { .. } => None,
			Nodes::Binding { name, .. } => Some(name),
			Nodes::Descriptor { name, .. } => Some(name),
			Nodes::Specialization { name, .. } => Some(name),
			Nodes::Type { name, .. } => Some(name),
			Nodes::Image { .. } => None,
			Nodes::CombinedImageSampler { .. } => None,
			Nodes::Expression(_) => None,
			Nodes::RawCode { .. } => None,
			Nodes::Intrinsic { name, .. } => Some(name),
			Nodes::Literal { name, .. } => Some(name),
			Nodes::Parameter { name, .. } => Some(name),
			Nodes::PushConstant { .. } => None,
			Nodes::Input { name, .. }
			| Nodes::Output { name, .. }
			| Nodes::TaskPayload { name, .. }
			| Nodes::Workgroup { name, .. } => Some(name),
			Nodes::Const { name, .. } => Some(name),
			Nodes::Null => None,
		}
	}

	pub fn node_mut(&mut self) -> &mut Nodes<'a> {
		// TODO: maybe do not expose nodes
		&mut self.node
	}

	pub fn node(&self) -> &Nodes<'a> {
		&self.node
	}

	pub fn get_mut(&mut self, name: &str) -> Option<&mut Node<'a>> {
		match &mut self.node {
			Nodes::Scope { children, .. } => children.iter_mut().find(|n| n.name() == Some(name)),
			_ => None,
		}
	}

	pub fn add(&mut self, children: Vec<Node<'a>>) {
		match &mut self.node {
			Nodes::Scope { children: c, .. } => {
				// Extend from the beginning of the vector
				c.extend(children);
			}
			_ => {
				println!("Tried to add children to a non-scope node.");
			}
		}
	}

	pub(crate) fn sort(&mut self) {
		// Place main function node at the end

		if let Nodes::Scope { children, .. } = &mut self.node {
			// Only sort scopes
			// Place main function node at the end
			children.sort_by(|a, b| {
				if a.name() == Some("main") {
					std::cmp::Ordering::Greater
				} else if b.name() == Some("main") {
					std::cmp::Ordering::Less
				} else {
					std::cmp::Ordering::Equal
				}
			});
			children.iter_mut().for_each(|n| n.sort()); // Recursively sort children
		}
	}
}

#[derive(Clone, Debug)]
pub enum Nodes<'a> {
	/// A placeholder for syntax that does not yet have a specialized node.
	Null,
	/// A named group of BESL declarations, similar to a Rust module.
	Scope {
		/// The name used for imports and namespaces.
		name: &'a str,
		children: Vec<Node<'a>>,
	},
	/// A struct declaration and its fields.
	Struct {
		name: &'a str,
		fields: Vec<Node<'a>>,
	},
	/// A field declared in a struct.
	Member {
		name: &'a str,
		r#type: String,
	},
	/// A function declaration and body.
	Function {
		name: &'a str,
		params: Vec<Node<'a>>,
		return_type: TypeName<'a>,
		statements: Vec<Node<'a>>,
	},
	Conditional {
		condition: Box<Node<'a>>,
		statements: Vec<Node<'a>>,
	},
	ForLoop {
		initializer: Box<Node<'a>>,
		condition: Box<Node<'a>>,
		update: Box<Node<'a>>,
		statements: Vec<Node<'a>>,
	},
	/// A shader resource binding declaration.
	Binding {
		name: &'a str,
		r#type: Box<Node<'a>>,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: Option<BufferMemoryClass>,
		count: Option<NonZeroUsize>,
	},
	/// A flat resource descriptor declared directly in BESL source.
	Descriptor {
		name: &'a str,
		resource_type: &'a str,
		format: Option<&'a str>,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: Option<&'a str>,
		count: Option<NonZeroU32>,
	},
	/// A constant selected when the application creates a pipeline.
	Specialization {
		name: &'a str,
		r#type: &'a str,
	},
	/// A small constant buffer updated during rendering.
	PushConstant {
		members: Vec<Node<'a>>,
	},
	/// An abstract type declaration, such as the declaration for `f32`.
	Type {
		name: &'a str,
		members: Vec<Node<'a>>,
	},
	Image {
		format: &'a str,
	},
	CombinedImageSampler {
		format: &'a str,
	},
	Expression(Expressions<'a>),
	RawCode {
		glsl: Option<Cow<'a, str>>,
		hlsl: Option<Cow<'a, str>>,
		msl: Option<Cow<'a, str>>,
		input: &'a [&'a str],
		output: &'a [&'a str],
	},
	Intrinsic {
		name: &'a str,
		elements: Vec<Node<'a>>,
		r#return: &'a str,
	},
	Input {
		name: &'a str,
		format: &'a str,
		location: u8,
	},
	Output {
		name: &'a str,
		format: &'a str,
		location: u8,
		count: Option<NonZeroUsize>,
	},
	/// An array carried from a task shader invocation group to the mesh work it emits.
	TaskPayload {
		name: &'a str,
		format: &'a str,
		count: NonZeroUsize,
	},
	/// Storage shared by all invocations in one task or compute workgroup.
	Workgroup {
		name: &'a str,
		format: &'a str,
		count: Option<NonZeroUsize>,
	},
	Literal {
		name: &'a str,
		body: Box<Node<'a>>,
	},
	Parameter {
		name: &'a str,
		r#type: TypeName<'a>,
	},
	/// A named module-level value known at compile time.
	Const {
		name: &'a str,
		r#type: TypeName<'a>,
		value: Box<Node<'a>>,
	},
}

#[derive(Clone, Debug)]
pub enum Expressions<'a> {
	Expression(Vec<Node<'a>>),
	Accessor {
		left: Box<Node<'a>>,
		right: Box<Node<'a>>,
	},
	Member {
		name: Cow<'a, str>,
	},
	Literal {
		value: Cow<'a, str>,
	},
	Call {
		name: TypeName<'a>,
		parameters: Vec<Node<'a>>,
	},
	Operator {
		name: &'a str,
		left: Box<Node<'a>>,
		right: Box<Node<'a>>,
	},
	VariableDeclaration {
		name: Cow<'a, str>,
		r#type: TypeName<'a>,
	},
	RawCode {
		glsl: Option<&'a str>,
		hlsl: Option<&'a str>,
		msl: Option<&'a str>,
		input: &'a [&'a str],
		output: &'a [&'a str],
	},
	Macro {
		name: &'a str,
		body: Box<Node<'a>>,
	},
	Return {
		value: Option<Box<Node<'a>>>,
	},
	Continue,
	Discard,
}

#[derive(Clone, Debug)]
pub(super) enum Atoms<'a> {
	Keyword,
	Continue,
	Discard,
	Accessor,
	GroupedExpression(Vec<Atoms<'a>>),
	Member {
		name: &'a str,
	},
	Literal {
		value: &'a str,
	},
	FunctionCall {
		name: TypeName<'a>,
		parameters: Vec<Vec<Atoms<'a>>>,
	},
	Operator {
		name: &'a str,
	},
	VariableDeclaration {
		name: &'a str,
		r#type: TypeName<'a>,
	},
}

#[derive(Debug)]
pub enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax {
		message: String,
	},
	StreamEndedPrematurely,
}

impl std::fmt::Display for ParsingFailReasons {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ParsingFailReasons::NotMine => write!(f, "Parser cannot handle this syntax."),
			ParsingFailReasons::BadSyntax { message } => write!(f, "Bad syntax: {}", message),
			ParsingFailReasons::StreamEndedPrematurely => {
				write!(f, "Token stream ended prematurely.")
			}
		}
	}
}

pub(super) fn make_scope<'a>(name: &'a str, children: Vec<Node<'a>>) -> Node<'a> {
	Node {
		node: Nodes::Scope { name, children },
	}
}

pub(super) fn make_member<'a>(name: &'a str, r#type: &'_ str) -> Node<'a> {
	Node {
		node: Nodes::Member {
			name,
			r#type: r#type.to_string(),
		},
	}
}

pub(super) fn make_struct<'a>(name: &'a str, children: Vec<Node<'a>>) -> Node<'a> {
	Node {
		node: Nodes::Struct { name, fields: children },
	}
}

pub(super) fn make_function<'a>(
	name: &'a str,
	params: Vec<Node<'a>>,
	return_type: impl Into<TypeName<'a>>,
	statements: Vec<Node<'a>>,
) -> Node<'a> {
	Node {
		node: Nodes::Function {
			name,
			params,
			return_type: return_type.into(),
			statements,
		},
	}
}

pub(super) trait Precedence {
	fn precedence(&self) -> u8;
}

impl Precedence for Atoms<'_> {
	fn precedence(&self) -> u8 {
		match self {
			Atoms::Keyword => 0,
			Atoms::Continue => 0,
			Atoms::Discard => 0,
			Atoms::Accessor => 1,
			Atoms::GroupedExpression { .. } => 0,
			Atoms::Member { .. } => 0,
			Atoms::Literal { .. } => 0,
			Atoms::FunctionCall { .. } => 0,
			Atoms::Operator { name } => match *name {
				"=" => 8,
				"||" => 7,
				"&&" => 6,
				"|" => 7,
				"&" => 6,
				"==" => 5,
				"!=" => 5,
				"<" => 5,
				">" => 5,
				"<=" => 5,
				">=" => 5,
				"<<" => 4,
				">>" => 4,
				"+" => 3,
				"-" => 3,
				"*" => 2,
				"/" => 2,
				"%" => 2,
				_ => 0,
			},
			Atoms::VariableDeclaration { .. } => 0,
		}
	}
}

/// The result type returned by a syntax parser.
pub(super) type FeatureParserResult<'i, 'a> = Result<(Node<'a>, std::slice::Iter<'i, &'a str>), ParsingFailReasons>;

/// A function that tries to parse a token sequence.
pub(super) type FeatureParser<'i, 'a> = fn(std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a>;

pub(super) type ExpressionParserResult<'i, 'a> = Result<(Vec<Atoms<'a>>, std::slice::Iter<'i, &'a str>), ParsingFailReasons>;
pub(super) type ExpressionParser<'i, 'a> = fn(std::slice::Iter<'i, &'a str>, Vec<Atoms<'a>>) -> ExpressionParserResult<'i, 'a>;
