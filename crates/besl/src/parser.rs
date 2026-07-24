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

use crate::tokenizer;

/// A shared syntax node in a parsed BESL tree.
pub type NodeReference<'a> = &'a Node<'a>;

/// The `TypeName` enum preserves type structure while the parser still borrows source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeName<'a> {
	Named(&'a str),
	Array { element: Box<TypeName<'a>>, count: u32 },
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
pub(super) fn parse<'i, 'a: 'i>(tokens: &'i tokenizer::Tokens<'a>) -> Result<Node<'a>, ParsingFailReasons> {
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

	pub fn function(name: &'a str, params: Vec<Node<'a>>, return_type: &'a str, statements: Vec<Node<'a>>) -> Node<'a> {
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
		Self::binding_with_count(name, r#type, slot, read, write, None)
	}

	fn binding_with_count(
		name: &'a str,
		r#type: Node<'a>,
		slot: u32,
		read: bool,
		write: bool,
		count: Option<NonZeroUsize>,
	) -> Node<'a> {
		Node {
			node: Nodes::Binding {
				name,
				r#type: Box::new(r#type),
				slot,
				read,
				write,
				count,
			},
		}
	}

	pub fn binding_array(name: &'a str, r#type: Node<'a>, slot: u32, read: bool, write: bool, count: u32) -> Node<'a> {
		let count = NonZeroUsize::new(count as usize).expect(
			"Invalid binding array count. The most likely cause is that a resource array was declared with zero elements.",
		);
		Self::binding_with_count(name, r#type, slot, read, write, Some(count))
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

	pub fn variable_declaration(name: &'a str, r#type: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::VariableDeclaration {
				name,
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

	pub fn let_assignment(name: &'a str, r#type: &'a str, value: Node<'a>) -> Node<'a> {
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

	pub fn workgroup(name: &'a str, format: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Workgroup { name, format },
		}
	}

	pub fn intrinsic(name: &'a str, parameters: Node<'a>, body: Node<'a>, r#return: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Intrinsic {
				name,
				elements: vec![parameters, body],
				r#return,
			},
		}
	}

	pub fn null() -> Node<'a> {
		Node { node: Nodes::Null }
	}

	pub fn parameter(name: &'a str, r#type: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Parameter { name, r#type },
		}
	}

	pub fn constant(name: &'a str, r#type: &'a str, value: Node<'a>) -> Node<'a> {
		Self::constant_with_type(name, TypeName::Named(r#type), value)
	}

	/// Builds a constant node while preserving the parsed type structure.
	fn constant_with_type(name: &'a str, r#type: TypeName<'a>, value: Node<'a>) -> Node<'a> {
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
		return_type: &'a str,
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
	},
	Literal {
		name: &'a str,
		body: Box<Node<'a>>,
	},
	Parameter {
		name: &'a str,
		r#type: &'a str,
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
		name: &'a str,
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
}

#[derive(Clone, Debug)]
pub(super) enum Atoms<'a> {
	Keyword,
	Continue,
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

fn make_scope<'a>(name: &'a str, children: Vec<Node<'a>>) -> Node<'a> {
	Node {
		node: Nodes::Scope { name, children },
	}
}

fn make_member<'a>(name: &'a str, r#type: &'_ str) -> Node<'a> {
	Node {
		node: Nodes::Member {
			name,
			r#type: r#type.to_string(),
		},
	}
}

fn make_struct<'a>(name: &'a str, children: Vec<Node<'a>>) -> Node<'a> {
	Node {
		node: Nodes::Struct { name, fields: children },
	}
}

fn make_function<'a>(name: &'a str, params: Vec<Node<'a>>, return_type: &'a str, statements: Vec<Node<'a>>) -> Node<'a> {
	Node {
		node: Nodes::Function {
			name,
			params,
			return_type,
			statements,
		},
	}
}

trait Precedence {
	fn precedence(&self) -> u8;
}

impl Precedence for Atoms<'_> {
	fn precedence(&self) -> u8 {
		match self {
			Atoms::Keyword => 0,
			Atoms::Continue => 0,
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
type FeatureParserResult<'i, 'a> = Result<(Node<'a>, std::slice::Iter<'i, &'a str>), ParsingFailReasons>;

/// A function that tries to parse a token sequence.
type FeatureParser<'i, 'a> = fn(std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a>;

type ExpressionParserResult<'i, 'a> = Result<(Vec<Atoms<'a>>, std::slice::Iter<'i, &'a str>), ParsingFailReasons>;
type ExpressionParser<'i, 'a> = fn(std::slice::Iter<'i, &'a str>, Vec<Atoms<'a>>) -> ExpressionParserResult<'i, 'a>;

/// Runs parsers in order until one accepts the token stream.
fn execute_parsers<'i, 'a: 'i>(
	parsers: &[FeatureParser<'i, 'a>],
	mut iterator: std::slice::Iter<'i, &'a str>,
) -> FeatureParserResult<'i, 'a> {
	let mut error = None;

	for parser in parsers {
		match parser(iterator.clone()) {
			Ok(r) => return Ok(r),
			Err(ParsingFailReasons::NotMine) => {}
			Err(other) => {
				if error.is_none() {
					error = Some(other);
				}
			}
		}
	}

	if let Some(error) = error {
		return Err(error);
	}

	Err(ParsingFailReasons::BadSyntax {
		message: format!(
			"Tried several parsers none could handle the syntax for statement: {}",
			iterator.next().unwrap()
		),
	}) // No parser could handle this syntax.
}

/// Runs parsers in order and permits every parser to decline the syntax.
fn try_execute_parsers<'i, 'a: 'i>(
	parsers: &[FeatureParser<'i, 'a>],
	iterator: std::slice::Iter<'i, &'a str>,
) -> Option<FeatureParserResult<'i, 'a>> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone()) {
			return Some(Ok(r));
		}
	}

	None
}

/// Runs parsers in order until one accepts the token stream.
fn execute_expression_parsers<'i, 'a: 'i>(
	parsers: &[ExpressionParser<'i, 'a>],
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let mut error = None;

	for parser in parsers {
		match parser(iterator.clone(), expressions.clone()) {
			Ok(r) => return Ok(r),
			Err(ParsingFailReasons::NotMine) => {}
			Err(other) => {
				if error.is_none() {
					error = Some(other);
				}
			}
		}
	}

	if let Some(error) = error {
		return Err(error);
	}

	Err(ParsingFailReasons::BadSyntax {
		message: format!(
			"Tried several parsers none could handle the syntax for statement: {}",
			iterator.next().unwrap()
		),
	}) // No parser could handle this syntax.
}

/// Runs parsers in order and permits every parser to decline the syntax.
fn try_execute_expression_parsers<'i, 'a: 'i>(
	parsers: &[ExpressionParser<'i, 'a>],
	iterator: std::slice::Iter<'i, &'a str>,
	expressions: Vec<Atoms<'a>>,
) -> Option<ExpressionParserResult<'i, 'a>> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), expressions.clone()) {
			return Some(Ok(r));
		}
	}

	None
}

fn is_identifier_char(c: char) -> bool {
	// TODO: validate number at end of identifier
	c.is_alphanumeric() || c == '_'
}

fn is_identifier(s: &str) -> bool {
	if s == "struct" || s == "fn" || s == "let" || s == "return" || s == "const" {
		// should not be a keyword
		return false;
	}
	s.chars().all(is_identifier_char)
}

fn parse_const<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("const")?;

	let r#type = iterator.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find a type for const {}.", name),
		},
		_ => e,
	})?;
	let (r#type, mut iterator) = parse_type_name(iterator, r#type)?;

	iterator.next_str("=").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find = after type for const {}.", name),
		},
		_ => e,
	})?;

	let parsers = vec![parse_function_call, parse_literal, parse_variable];
	let (expressions, new_iterator) = execute_expression_parsers(&parsers, iterator, Vec::new())?;
	iterator = new_iterator;

	iterator.next_str(";").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find ; after const {} value.", name),
		},
		_ => e,
	})?;

	fn atoms_to_node<'a>(atoms: &[Atoms<'a>]) -> Node<'a> {
		let max_precedence_item = atoms.iter().enumerate().max_by_key(|(_, v)| v.precedence());

		if let Some((i, e)) = max_precedence_item {
			match e {
				Atoms::Operator { name } => {
					let left = atoms_to_node(&atoms[..i]);
					let right = atoms_to_node(&atoms[i + 1..]);
					Node {
						node: Nodes::Expression(Expressions::Operator {
							name,
							left: Box::new(left),
							right: Box::new(right),
						}),
					}
				}
				Atoms::FunctionCall { name, parameters } => {
					let parameters = parameters.iter().map(|v| atoms_to_node(v)).collect::<Vec<_>>();
					Node {
						node: Nodes::Expression(Expressions::Call {
							name: name.clone(),
							parameters,
						}),
					}
				}
				Atoms::Literal { value } => Node {
					node: Nodes::Expression(Expressions::Literal { value: (*value).into() }),
				},
				Atoms::Member { name } => Node {
					node: Nodes::Expression(Expressions::Member { name: (*name).into() }),
				},
				_ => panic!("Unexpected atom in const expression"),
			}
		} else {
			panic!("No atoms in const expression");
		}
	}

	let value = atoms_to_node(&expressions);

	Ok((Node::constant_with_type(name, r#type, value), iterator))
}

/// Parses a flat resource descriptor and preserves its source type name for semantic resolution.
fn parse_descriptor<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("descriptor")?;

	let syntax_error = |message: String| ParsingFailReasons::BadSyntax { message };
	iterator.next_str("<").map_err(|_| {
		syntax_error(format!(
			"Expected < after descriptor in resource {}. The most likely cause is that the descriptor arguments are missing.",
			name
		))
	})?;
	let resource_type = iterator.next_identifier().map_err(|_| {
		syntax_error(format!(
			"Expected a resource type in descriptor {}. The most likely cause is that the first descriptor argument is missing.",
			name
		))
	})?;
	let format = if iterator.clone().next().copied() == Some("<") {
		iterator.next();
		let format = iterator.next_identifier().map_err(|_| {
			syntax_error(format!(
				"Expected a storage image format in descriptor {}. The most likely cause is that the StorageImage format argument is missing.",
				name
			))
		})?;
		iterator.next_str(">").map_err(|_| {
			syntax_error(format!(
				"Expected > after storage image format in descriptor {}. The most likely cause is that the resource type arguments are malformed.",
				name
			))
		})?;
		if resource_type != "StorageImage" {
			return Err(syntax_error(format!(
				"Resource type {} cannot declare format `{}` in descriptor {}. The most likely cause is that a storage image format was attached to a non-StorageImage resource.",
				resource_type, format, name
			)));
		}
		Some(format)
	} else {
		None
	};
	iterator.next_str(",").map_err(|_| {
		syntax_error(format!(
			"Expected , after resource type in descriptor {}. The most likely cause is that the descriptor arguments are malformed.",
			name
		))
	})?;

	let slot = iterator
		.next()
		.ok_or_else(|| {
			syntax_error(format!(
				"Expected a slot in descriptor {}. The most likely cause is that the second descriptor argument is missing.",
				name
			))
		})?
		.parse::<u32>()
		.map_err(|_| {
			syntax_error(format!(
				"Invalid slot in descriptor {}. The most likely cause is that the slot is not a u32 literal.",
				name
			))
		})?;
	iterator.next_str(",").map_err(|_| {
		syntax_error(format!(
			"Expected , after slot in descriptor {}. The most likely cause is that the descriptor arguments are malformed.",
			name
		))
	})?;

	let access = iterator.next().ok_or_else(|| {
		syntax_error(format!(
			"Expected an access mode in descriptor {}. The most likely cause is that the third descriptor argument is missing.",
			name
		))
	})?;
	let (read, write) = match *access {
		"read" => (true, false),
		"write" => (false, true),
		"read_write" => (true, true),
		_ => {
			return Err(syntax_error(format!(
				"Invalid access mode `{}` in descriptor {}. The most likely cause is that the access is not read, write, or read_write.",
				access, name
			)));
		}
	};

	let count = if iterator.clone().next().copied() == Some(",") {
		iterator.next();
		let count = iterator
			.next()
			.ok_or_else(|| {
				syntax_error(format!(
					"Expected a resource count in descriptor {}. The most likely cause is that the fourth descriptor argument is missing.",
					name
				))
			})?
			.parse::<u32>()
			.map_err(|_| {
				syntax_error(format!(
					"Invalid resource count in descriptor {}. The most likely cause is that the count is not a u32 literal.",
					name
				))
			})?;
		Some(NonZeroU32::new(count).ok_or_else(|| {
			syntax_error(format!(
				"Invalid resource count in descriptor {}. The most likely cause is that the resource array was declared with zero elements.",
				name
			))
		})?)
	} else {
		None
	};

	iterator.next_str(">").map_err(|_| {
		syntax_error(format!(
			"Expected > after descriptor {} arguments. The most likely cause is that the descriptor declaration is incomplete.",
			name
		))
	})?;
	iterator.next_str(";").map_err(|_| {
		syntax_error(format!(
			"Expected ; after descriptor {}. The most likely cause is that the declaration terminator is missing.",
			name
		))
	})?;

	Ok((
		Node {
			node: Nodes::Descriptor {
				name,
				resource_type,
				format,
				slot,
				read,
				write,
				count,
			},
		},
		iterator,
	))
}

/// Parses stage-interface storage declared directly in BESL source.
fn parse_shader_interface_declaration<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	let declaration = iterator.next().copied().ok_or(ParsingFailReasons::StreamEndedPrematurely)?;
	if !matches!(declaration, "input" | "output" | "task_payload" | "workgroup") {
		return Err(ParsingFailReasons::NotMine);
	}

	let syntax_error = |message: String| ParsingFailReasons::BadSyntax { message };
	iterator.next_str("<").map_err(|_| {
		syntax_error(format!(
			"Expected < after {declaration} in {name}. The most likely cause is that the declaration arguments are missing."
		))
	})?;
	let format = iterator.next_identifier().map_err(|_| {
		syntax_error(format!(
			"Expected a type in {declaration} {name}. The most likely cause is that the first declaration argument is missing."
		))
	})?;

	let node = match declaration {
		"input" | "output" => {
			iterator.next_str(",").map_err(|_| {
				syntax_error(format!(
					"Expected , after the type in {declaration} {name}. The most likely cause is that the location is missing."
				))
			})?;
			let location = iterator
				.next()
				.ok_or_else(|| {
					syntax_error(format!(
						"Expected a location in {declaration} {name}. The most likely cause is that the second declaration argument is missing."
					))
				})?
				.parse::<u8>()
				.map_err(|_| {
					syntax_error(format!(
						"Invalid location in {declaration} {name}. The most likely cause is that the location is not a u8 literal."
					))
				})?;

			if declaration == "input" {
				Node::input(name, format, location)
			} else if iterator.clone().next().copied() == Some(",") {
				iterator.next();
				let count = iterator
					.next()
					.ok_or_else(|| {
						syntax_error(format!(
							"Expected an element count in output {name}. The most likely cause is that the third declaration argument is missing."
						))
					})?
					.parse::<u32>()
					.map_err(|_| {
						syntax_error(format!(
							"Invalid element count in output {name}. The most likely cause is that the count is not a u32 literal."
						))
					})?;
				if count == 0 {
					return Err(syntax_error(format!(
						"Invalid element count in output {name}. The most likely cause is that an output array was declared with zero elements."
					)));
				}
				Node::output_array(name, format, location, count)
			} else {
				Node::output(name, format, location)
			}
		}
		"task_payload" => {
			iterator.next_str(",").map_err(|_| {
				syntax_error(format!(
					"Expected , after the type in task_payload {name}. The most likely cause is that the element count is missing."
				))
			})?;
			let count = iterator
				.next()
				.ok_or_else(|| {
					syntax_error(format!(
						"Expected an element count in task_payload {name}. The most likely cause is that the second declaration argument is missing."
					))
				})?
				.parse::<u32>()
				.map_err(|_| {
					syntax_error(format!(
						"Invalid element count in task_payload {name}. The most likely cause is that the count is not a u32 literal."
					))
				})?;
			if count == 0 {
				return Err(syntax_error(format!(
					"Invalid element count in task_payload {name}. The most likely cause is that a task-payload array was declared with zero elements."
				)));
			}
			Node::task_payload(name, format, count)
		}
		"workgroup" => Node::workgroup(name, format),
		_ => unreachable!("Shader interface declaration was validated above."),
	};

	iterator.next_str(">").map_err(|_| {
		syntax_error(format!(
			"Expected > after {declaration} {name} arguments. The most likely cause is that the declaration is incomplete."
		))
	})?;
	iterator.next_str(";").map_err(|_| {
		syntax_error(format!(
			"Expected ; after {declaration} {name}. The most likely cause is that the declaration terminator is missing."
		))
	})?;

	Ok((node, iterator))
}

/// Parses the single push-constant block exposed to shader source as `push_constant`.
fn parse_push_constant<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("push_constant")?;
	iterator.next_str(":")?;
	iterator.next_str("push_constant")?;
	iterator.next_str("{").map_err(|_| ParsingFailReasons::BadSyntax {
		message: "Expected { after push_constant declaration.".to_string(),
	})?;

	let mut members = Vec::new();
	loop {
		let Some(token) = iterator.next().copied() else {
			return Err(ParsingFailReasons::BadSyntax {
				message: "Push-constant declaration is missing a closing }.".to_string(),
			});
		};
		if token == "}" {
			break;
		}
		if token == "," {
			continue;
		}

		let member_name = token;
		iterator.next_str(":").map_err(|_| ParsingFailReasons::BadSyntax {
			message: format!("Expected : after push-constant member {member_name}."),
		})?;
		let member_type = iterator.next_identifier().map_err(|_| ParsingFailReasons::BadSyntax {
			message: format!("Expected a type after push-constant member {member_name}."),
		})?;
		members.push(make_member(member_name, member_type));
	}

	Ok((Node::push_constant(members), iterator))
}

fn parse_member<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	let mut r#type = iterator
		.next_identifier()
		.map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected to find type while parsing member {}.", name),
			},
			_ => e,
		})?
		.to_string();

	if let Some(&&n) = iterator.clone().peekable().peek() {
		if n == "<" {
			if r#type == "descriptor" {
				return Err(ParsingFailReasons::BadSyntax {
					message: format!(
						"Invalid descriptor declaration for {name}. The most likely cause is that required slot or access arguments are missing."
					),
				});
			}
			iterator.next();
			r#type.push('<');
			let next = iterator.next().ok_or(ParsingFailReasons::BadSyntax {
				message: format!("Expected to find type while parsing generic argument for member {}", name),
			})?;
			r#type.push_str(next.as_ref());
			iterator.next();
			r#type.push('>');
		}
	}

	let node = Node::member(name, &r#type);

	iterator.next().ok_or(ParsingFailReasons::BadSyntax {
		message: "Expected semicolon".to_string(),
	})?; // Skip semicolon

	Ok(((node), iterator))
}

fn parse_macro<'i, 'a: 'i>(iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let mut iter = iterator;

	iter.next_str("#")?;
	iter.next_str("[")?;
	iter.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: "Expected to find macro name after #[.".to_string(),
		},
		_ => e,
	})?;
	iter.next_str("]").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: "Expected to find ] after macro name.".to_string(),
		},
		_ => e,
	})?;

	Ok((make_scope("MACRO", vec![]), iter))
}

fn parse_struct<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	iterator.next_str("struct")?;
	iterator.next_str("{").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find {{ after struct {} declaration.", name),
		},
		_ => e,
	})?;

	let mut fields = vec![];

	while let Some(&v) = iterator.next() {
		if v == "}" {
			break;
		} else if v == "," {
			continue;
		}

		iterator.next_str(":").map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected to find : after name for member {} in struct {}", v, name),
			},
			_ => e,
		})?;

		let type_name = iterator.next_identifier().map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected to find a type name after : for member {} in struct {}", v, name),
			},
			_ => e,
		})?;

		// See if is array type
		let type_name = if iterator.clone().peekable().peek().map(|v| v.as_ref()) == Some("[") {
			iterator.next();
			let count = iterator
				.next()
				.and_then(|v| v.parse::<u32>().ok())
				.ok_or(ParsingFailReasons::BadSyntax {
					message: format!("Expected to find a number after [ for member {} in struct {}", v, name),
				})?;
			iterator.next().unwrap();
			format!("{}[{}]", type_name, count)
		} else {
			type_name.to_string()
		};

		fields.push(make_member(v, &type_name));
	}

	let node = Node::r#struct(name, fields);

	Ok((node, iterator))
}

fn parse_var_decl<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("let")?;
	let variable_name = iterator.next_identifier()?;
	iterator.next_str(":")?;
	let variable_type = iterator.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find a type for variable {}", variable_name),
		},
		_ => e,
	})?;
	let (variable_type, iterator) = parse_type_name(iterator, variable_type)?;

	expressions.push(Atoms::VariableDeclaration {
		name: variable_name,
		r#type: variable_type,
	});

	let possible_following_expressions: Vec<ExpressionParser<'i, 'a>> = vec![parse_operator];

	let expressions = execute_expression_parsers(&possible_following_expressions, iterator, expressions)?;

	Ok(expressions)
}

/// Parses a source-backed type name and all of its array suffixes.
fn parse_type_name<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	base_type: &'a str,
) -> Result<(TypeName<'a>, std::slice::Iter<'i, &'a str>), ParsingFailReasons> {
	let mut type_name = TypeName::Named(base_type);

	while iterator.clone().peekable().peek().map(|token| token.as_ref()) == Some("[") {
		iterator.next_str("[")?;
		let count = iterator
			.next_is(|token| token.chars().all(|c| c.is_ascii_digit()))?
			.parse::<u32>()
			.map_err(|_| ParsingFailReasons::BadSyntax {
				message: format!("Invalid array count for type {}", type_name),
			})?;
		iterator.next_str("]")?;

		type_name = TypeName::Array {
			element: Box::new(type_name),
			count,
		};
	}

	Ok((type_name, iterator))
}

fn parse_keywords<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("return")?;

	expressions.push(Atoms::Keyword);

	if **iterator
		.clone()
		.peekable()
		.peek()
		.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
		== ";"
	{
		return Ok((expressions, iterator));
	}

	try_execute_expression_parsers(&[parse_rvalue], iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, iterator)))
}

fn parse_continue<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("continue")?;
	expressions.push(Atoms::Continue);
	Ok((expressions, iterator))
}

fn parse_variable<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;

	expressions.push(Atoms::Member { name });

	let lexers = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&lexers, iterator.clone(), expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

fn parse_accessor<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let _ = iterator.next_str(".")?;

	expressions.push(Atoms::Accessor);

	let lexers: Vec<ExpressionParser<'i, 'a>> = vec![parse_variable];

	execute_expression_parsers(&lexers, iterator, expressions)
}

fn parse_index_accessor<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let _ = iterator.next_str("[")?;
	expressions.push(Atoms::Accessor);
	let (inner_expressions, mut iterator) = execute_expression_parsers(&[parse_rvalue], iterator, Vec::new())?;
	expressions.push(Atoms::GroupedExpression(inner_expressions));
	iterator.next_str("]")?;

	let lexers = vec![parse_operator, parse_accessor, parse_index_accessor];
	try_execute_expression_parsers(&lexers, iterator.clone(), expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

fn is_literal(s: &str) -> bool {
	matches!(s, "true" | "false") || s.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn parse_literal<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let value = iterator.next_is(is_literal)?;

	expressions.push(Atoms::Literal { value });

	let possible_following_expressions = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&possible_following_expressions, iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, iterator)))
}

/// Parses a parenthesized sub-expression like `(a + b)`.
fn parse_grouped_expression<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	iterator.next_str("(")?;

	// Parse the inner expression
	let (inner_expressions, mut inner_iterator) = execute_expression_parsers(&[parse_rvalue], iterator, Vec::new())?;

	inner_iterator.next_str(")").map_err(|_| ParsingFailReasons::BadSyntax {
		message: "Expected closing ')' for grouped expression".to_string(),
	})?;

	// Keep grouped expressions intact so later lowering can preserve precedence.
	expressions.push(Atoms::GroupedExpression(inner_expressions));

	// Check for following expressions (operators, accessors, etc.)
	let possible_following_expressions = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&possible_following_expressions, inner_iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, inner_iterator)))
}

fn parse_rvalue<'i, 'a: 'i>(
	iterator: std::slice::Iter<'i, &'a str>,
	expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let parsers = vec![parse_function_call, parse_grouped_expression, parse_literal, parse_variable];

	execute_expression_parsers(&parsers, iterator.clone(), expressions)
}

fn parse_operator<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let operator =
		iterator.next_is(|v| {
			v == "*"
				|| v == "+" || v == "-"
				|| v == "/" || v == "%"
				|| v == "=" || v == "<"
				|| v == ">" || v == "=="
				|| v == "!=" || v == "<="
				|| v == ">=" || v == "&&"
				|| v == "||" || v == "<<"
				|| v == ">>" || v == "&"
				|| v == "|"
		})?;

	expressions.push(Atoms::Operator { name: operator });

	let possible_following_expressions: Vec<ExpressionParser<'i, 'a>> = vec![parse_rvalue];

	execute_expression_parsers(&possible_following_expressions, iterator, expressions)
}

fn expression_atoms_to_node<'a>(atoms: &[Atoms<'a>]) -> Node<'a> {
	if matches!(atoms.first(), Some(Atoms::Keyword)) {
		return Node {
			node: Nodes::Expression(Expressions::Return {
				value: atoms
					.get(1..)
					.filter(|remaining| !remaining.is_empty())
					.map(|remaining| Box::new(expression_atoms_to_node(remaining))),
			}),
		};
	}

	if matches!(atoms.first(), Some(Atoms::Continue)) {
		return Node {
			node: Nodes::Expression(Expressions::Continue),
		};
	}

	let max_precedence_item = atoms.iter().enumerate().max_by_key(|(_, v)| v.precedence());

	if let Some((i, e)) = max_precedence_item {
		match e {
			Atoms::Keyword => Node {
				node: Nodes::Expression(Expressions::Return { value: None }),
			},
			Atoms::Continue => Node {
				node: Nodes::Expression(Expressions::Continue),
			},
			Atoms::Operator { name } => {
				let left = expression_atoms_to_node(&atoms[..i]);
				let right = expression_atoms_to_node(&atoms[i + 1..]);

				Node {
					node: Nodes::Expression(Expressions::Operator {
						name,
						left: Box::new(left),
						right: Box::new(right),
					}),
				}
			}
			Atoms::Accessor => {
				let left = expression_atoms_to_node(&atoms[..i]);
				let right = expression_atoms_to_node(&atoms[i + 1..]);

				Node {
					node: Nodes::Expression(Expressions::Accessor {
						left: Box::new(left),
						right: Box::new(right),
					}),
				}
			}
			Atoms::GroupedExpression(inner) => Node::sentence(vec![expression_atoms_to_node(inner)]),
			Atoms::FunctionCall { name, parameters } => {
				let parameters = parameters.iter().map(|v| expression_atoms_to_node(v)).collect::<Vec<_>>();

				Node {
					node: Nodes::Expression(Expressions::Call {
						name: name.clone(),
						parameters,
					}),
				}
			}
			Atoms::Literal { value } => Node {
				node: Nodes::Expression(Expressions::Literal { value: (*value).into() }),
			},
			Atoms::Member { name } => Node {
				node: Nodes::Expression(Expressions::Member { name: (*name).into() }),
			},
			Atoms::VariableDeclaration { name, r#type } => Node {
				node: Nodes::Expression(Expressions::VariableDeclaration {
					name,
					r#type: r#type.clone(),
				}),
			},
		}
	} else {
		panic!("No max precedence item");
	}
}

fn parse_conditional<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("if")?;
	iterator.next_str("(")?;

	let (condition_atoms, mut iterator) = execute_expression_parsers(&[parse_rvalue], iterator, Vec::new())?;
	let condition = expression_atoms_to_node(&condition_atoms);

	iterator.next_str(")")?;
	iterator.next_str("{")?;

	let mut statements = vec![];
	loop {
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== "}"
		{
			iterator.next();
			break;
		}

		let (statement, new_iterator) = parse_statement(iterator)?;
		statements.push(statement);
		iterator = new_iterator;
	}

	Ok((Node::conditional(condition, statements), iterator))
}

fn parse_for_loop<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	iterator.next_str("for")?;
	iterator.next_str("(")?;

	let statement_parsers = vec![
		parse_keywords,
		parse_continue,
		parse_var_decl,
		parse_function_call,
		parse_variable,
	];
	let (initializer_atoms, mut iterator) = execute_expression_parsers(&statement_parsers, iterator, Vec::new())?;
	let initializer = expression_atoms_to_node(&initializer_atoms);

	iterator.next_str(";")?;

	let (condition_atoms, mut iterator) = execute_expression_parsers(&[parse_rvalue], iterator, Vec::new())?;
	let condition = expression_atoms_to_node(&condition_atoms);

	iterator.next_str(";")?;

	let (update_atoms, mut iterator) = execute_expression_parsers(&statement_parsers, iterator, Vec::new())?;
	let update = expression_atoms_to_node(&update_atoms);

	iterator.next_str(")")?;
	iterator.next_str("{")?;

	let mut statements = vec![];
	loop {
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== "}"
		{
			iterator.next();
			break;
		}

		let (statement, new_iterator) = parse_statement(iterator)?;
		statements.push(statement);
		iterator = new_iterator;
	}

	Ok((Node::for_loop(initializer, condition, update, statements), iterator))
}

fn parse_function_call<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let function_name = iterator.next_identifier()?;
	let (function_name, mut iterator) = parse_type_name(iterator, function_name)?;
	iterator.next_str("(")?;

	let mut parameters = vec![];

	loop {
		let iter_before = iterator.clone();

		if let Some(a) = try_execute_expression_parsers(&[parse_rvalue], iterator.clone(), Vec::new()) {
			let (expressions, new_iterator) = a?;
			parameters.push(expressions);
			iterator = new_iterator;
		}

		// Check if iter is comma
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ","
		{
			iterator.next();
		}

		// check if iter is close brace
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ")"
		{
			iterator.next();
			break;
		}

		// Safety: if no progress was made, break to avoid infinite loop
		if iterator.len() == iter_before.len() {
			let token = iterator.clone().peekable().peek().copied().copied().unwrap_or("<eof>");
			return Err(ParsingFailReasons::BadSyntax {
				message: format!("Unexpected token '{}' in function call {}", token, function_name),
			});
		}
	}

	expressions.push(Atoms::FunctionCall {
		name: function_name,
		parameters,
	});

	let possible_following_expressions = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&possible_following_expressions, iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, iterator)))
}

fn parse_statement<'i, 'a: 'i>(iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	if let Some(result) = try_execute_parsers(&[parse_conditional], iterator.clone()) {
		return result;
	}

	if let Some(result) = try_execute_parsers(&[parse_for_loop], iterator.clone()) {
		return result;
	}

	let parsers = vec![
		parse_keywords,
		parse_continue,
		parse_var_decl,
		parse_function_call,
		parse_variable,
	];

	let (expressions, mut iterator) = execute_expression_parsers(&parsers, iterator, Vec::new())?;

	iterator.next_str(";")?; // Skip semicolon

	Ok((expression_atoms_to_node(&expressions), iterator))
}

fn parse_function<'i, 'a: 'i>(mut iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let name = iterator.next_identifier()?;

	iterator.next_str(":")?;
	iterator.next_str("fn")?;
	iterator.next_str("(")?;

	let mut params = Vec::new();
	loop {
		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ")"
		{
			iterator.next();
			break;
		}

		let param_name = iterator.next_identifier().map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected a parameter name for function {}.", name),
			},
			_ => e,
		})?;
		iterator.next_str(":")?;
		let param_type = iterator.next_identifier().map_err(|e| match e {
			ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
				message: format!("Expected a parameter type for function {}.", name),
			},
			_ => e,
		})?;
		params.push(Node::parameter(param_name, param_type));

		if **iterator
			.clone()
			.peekable()
			.peek()
			.ok_or(ParsingFailReasons::StreamEndedPrematurely)?
			== ","
		{
			iterator.next();
		}
	}
	iterator.next_str("->")?;

	let return_type = iterator.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected a return type for function {} declaration.", name),
		},
		_ => e,
	})?;

	iterator.next_str("{").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected a {{ after function {} declaration.", name),
		},
		_ => e,
	})?;

	let mut statements = vec![];

	loop {
		if let Some(Ok((expression, new_iterator))) = try_execute_parsers(&[parse_statement], iterator.clone()) {
			iterator = new_iterator;

			statements.push(expression);
		} else {
			// A failed statement parser at EOF means the function body was truncated.
			let Some(token) = iterator.clone().next().copied() else {
				return Err(ParsingFailReasons::BadSyntax {
					message: format!(
						"Function `{}` is missing a closing `}}`. The source most likely ended before the function body was complete.",
						name
					),
				});
			};

			if token == "}" {
				iterator.next();
				break;
			} else {
				return Err(ParsingFailReasons::BadSyntax {
					message: format!("Expected a }} after function {} declaration, found `{}`.", name, token),
				});
			}
		}

		// check if iter is close brace
		if **iterator.clone().peekable().peek().ok_or(ParsingFailReasons::BadSyntax {
			message: "Expected a '}' after function body".to_string(),
		})? == "}"
		{
			iterator.next();
			break;
		}
	}

	let node = Node::function(name, params, return_type, statements);

	Ok((node, iterator))
}

use std::ops::Index;

impl<'a> Index<&str> for Node<'a> {
	type Output = Node<'a>;

	fn index(&self, index: &str) -> &Self::Output {
		let child = match &self.node {
			Nodes::Scope { children, .. } => children.iter().find(|child| {
				matches!(
					child.node(),
					Nodes::Scope { .. }
						| Nodes::Struct { .. }
						| Nodes::Member { .. }
						| Nodes::Function { .. }
						| Nodes::Descriptor { .. }
						| Nodes::Input { .. }
						| Nodes::Output { .. }
						| Nodes::TaskPayload { .. }
						| Nodes::Workgroup { .. }
						| Nodes::Const { .. }
				) && child.name() == Some(index)
			}),
			Nodes::Struct { fields, .. } => fields
				.iter()
				.find(|field| matches!(field.node(), Nodes::Member { .. }) && field.name() == Some(index)),
			_ => panic!("Cannot search  in these"),
		};

		child.unwrap_or_else(|| panic!("Not found"))
	}
}

trait ParserIterator<'a> {
	fn next_is(&mut self, f: impl Fn(&'a str) -> bool) -> Result<&'a str, ParsingFailReasons>;
	fn next_str(&mut self, expected: &'a str) -> Result<&'a str, ParsingFailReasons>;
	fn next_identifier(&mut self) -> Result<&'a str, ParsingFailReasons>;
}

impl<'i, 'a> ParserIterator<'a> for std::slice::Iter<'i, &'a str> {
	fn next_is(&mut self, f: impl Fn(&'a str) -> bool) -> Result<&'a str, ParsingFailReasons> {
		let token = self.next().ok_or(ParsingFailReasons::StreamEndedPrematurely)?;
		if f(token) {
			Ok(token)
		} else {
			Err(ParsingFailReasons::NotMine)
		}
	}

	fn next_str(&mut self, expected: &'a str) -> Result<&'a str, ParsingFailReasons> {
		self.next_is(|v| v == expected)
	}

	fn next_identifier(&mut self) -> Result<&'a str, ParsingFailReasons> {
		self.next_is(is_identifier)
	}
}

#[derive(Clone)]
pub struct ProgramState {
	// pub(super) types: HashMap<String, NodeReference>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::tokenizer::tokenize;

	#[test]
	#[should_panic(expected = "Invalid binding array count")]
	fn binding_array_rejects_zero_elements() {
		Node::binding_array("textures", Node::combined_image_sampler(), 0, true, false, 0);
	}

	#[test]
	fn parse_stage_interface_and_task_storage_declarations() {
		let tokens = tokenize(
			r#"
				instance_index: input<u32, 0>;
				primitive_index: output<u32, 1>;
				meshlet_indices: output<u32, 2, 126>;
				visible_meshlets: task_payload<u32, 32>;
				visible_count: workgroup<atomicu32>;
			"#,
		)
		.expect("stage-interface source should tokenize");
		let root = parse(&tokens).expect("stage-interface source should parse");

		assert!(matches!(
			root["instance_index"].node(),
			Nodes::Input {
				format: "u32",
				location: 0,
				..
			}
		));
		assert!(matches!(
			root["primitive_index"].node(),
			Nodes::Output {
				format: "u32",
				location: 1,
				count: None,
				..
			}
		));
		assert!(matches!(
			root["meshlet_indices"].node(),
			Nodes::Output {
				format: "u32",
				location: 2,
				count: Some(count),
				..
			} if count.get() == 126
		));
		assert!(matches!(
			root["visible_meshlets"].node(),
			Nodes::TaskPayload {
				format: "u32",
				count,
				..
			} if count.get() == 32
		));
		assert!(matches!(
			root["visible_count"].node(),
			Nodes::Workgroup { format: "atomicu32", .. }
		));
	}

	#[test]
	fn stage_interface_declarations_reject_invalid_locations_and_counts() {
		for source in [
			"value: input<u32, 256>;",
			"value: output<u32, 0, 0>;",
			"value: task_payload<u32, 0>;",
			"value: workgroup<u32>",
		] {
			let tokens = tokenize(source).expect("invalid declaration should still tokenize");
			assert!(parse(&tokens).is_err(), "expected `{source}` to be rejected");
		}
	}

	#[test]
	fn parse_resource_descriptors_with_flat_slots_access_and_count() {
		let tokens = tokenize(
			r#"
				source: descriptor<Texture2D, 3, read>;
				result: descriptor<StorageImage<rgba16f>, 7, write, 4>;
				unformatted_result: descriptor<StorageImage, 8, write>;
				data: descriptor<Data, 11, read_write>;
				textures: descriptor<Texture2DArray, 20, read, 16>;
			"#,
		)
		.expect("descriptor source should tokenize");
		let root = parse(&tokens).expect("descriptor source should parse");

		let Nodes::Descriptor {
			resource_type,
			slot,
			read,
			write,
			count,
			..
		} = root["source"].node()
		else {
			panic!("expected source descriptor");
		};
		assert_eq!(*resource_type, "Texture2D");
		assert_eq!(*slot, 3);
		assert!(*read);
		assert!(!*write);
		assert_eq!(*count, None);

		assert!(matches!(
			root["result"].node(),
			Nodes::Descriptor {
				format: Some("rgba16f"),
				slot: 7,
				read: false,
				write: true,
				count: Some(count),
				..
			} if count.get() == 4
		));
		assert!(matches!(
			root["unformatted_result"].node(),
			Nodes::Descriptor {
				format: None,
				slot: 8,
				..
			}
		));
		assert!(matches!(
			root["data"].node(),
			Nodes::Descriptor {
				resource_type: "Data",
				slot: 11,
				read: true,
				write: true,
				..
			}
		));
		assert!(matches!(
			root["textures"].node(),
			Nodes::Descriptor { resource_type: "Texture2DArray", slot: 20, count: Some(count), .. }
				if count.get() == 16
		));
	}

	#[test]
	fn parse_source_push_constant_block() {
		let tokens = tokenize(
			r#"
				push_constant: push_constant {
					source_vertex_base: u32,
					destination_vertex_base: u32,
					vertex_count: u32,
				}
			"#,
		)
		.expect("push-constant source should tokenize");
		let root = parse(&tokens).expect("push-constant source should parse");
		let Nodes::Scope { children, .. } = root.node() else {
			panic!("expected root scope");
		};
		assert!(matches!(
			children.as_slice(),
			[Node {
				node: Nodes::PushConstant { members },
				..
			}] if members.len() == 3
		));
	}

	#[test]
	fn descriptor_rejects_invalid_access_count_and_arguments() {
		for source in [
			"texture: descriptor<Texture2D, 0, execute>;",
			"textures: descriptor<Texture2D, 0, read, 0>;",
			"texture: descriptor<Texture2D>;",
		] {
			let tokens = tokenize(source).expect("descriptor source should tokenize");
			assert!(parse(&tokens).is_err(), "malformed descriptor should be rejected: {source}");
		}
	}

	#[test]
	fn descriptor_rejects_formats_on_non_storage_image_resources() {
		for source in [
			"texture: descriptor<Texture2D<rgba16f>, 0, read>;",
			"data: descriptor<Data<rgba16f>, 0, read>;",
		] {
			let tokens = tokenize(source).expect("formatted descriptor source should tokenize");
			assert!(
				parse(&tokens).is_err(),
				"non-storage image descriptor format should be rejected: {source}"
			);
		}
	}

	fn assert_named_type(type_name: &TypeName<'_>, expected: &str) {
		assert!(matches!(type_name, TypeName::Named(name) if *name == expected));
	}

	fn print_tree(node: &Node) {
		match &node.node {
			Nodes::Scope { name, children } => {
				println!("{}", name,);
				for child in children {
					print_tree(child);
				}
			}
			Nodes::Struct { name, fields } => {
				println!("{}", name,);
				for field in fields {
					print_tree(field);
				}
			}
			_ => {}
		}
	}

	fn assert_struct(node: &Node) {
		if let Nodes::Struct { name, fields } = &node.node {
			assert_eq!(*name, "Light");
			assert_eq!(fields.len(), 2);

			let position = &fields[0];

			if let Nodes::Member { name, r#type } = &position.node {
				assert_eq!(*name, "position");
				assert_eq!(r#type, "vec3f");
			} else {
				panic!("Not a member");
			}

			let color = &fields[1];

			if let Nodes::Member { name, r#type } = &color.node {
				assert_eq!(*name, "color");
				assert_eq!(r#type, "vec3f");
			} else {
				panic!("Not a member");
			}
		} else {
			panic!("Not a struct");
		}
	}

	#[test]
	fn test_parse_struct() {
		let source = "
Light: struct {
	array: u32[3],
	position: vec3f,
	color: vec3f
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		// program.types.get("Light").expect("Failed to get Light type");

		if let Nodes::Struct { name, .. } = node.node {
			assert_eq!(name, "root");
			assert_struct(&node["Light"]);
		}
	}

	fn assert_function(node: &Node) {
		if let Nodes::Function {
			name,
			params,
			return_type,
			statements,
			..
		} = &node.node
		{
			assert_eq!(*name, "main");
			assert_eq!(params.len(), 0);
			assert_eq!(*return_type, "void");
			assert_eq!(statements.len(), 2);

			let statement = &statements[0];

			if let Nodes::Expression(Expressions::Operator {
				name,
				left: var_decl,
				right: function_call,
			}) = &statement.node
			{
				assert_eq!(*name, "=");

				if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type, .. }) = &var_decl.node {
					assert_eq!(*name, "position");
					assert_named_type(r#type, "vec4f");
				} else {
					panic!("Not an variable declaration");
				}

				if let Nodes::Expression(Expressions::Call { name, parameters, .. }) = &function_call.node {
					assert_named_type(name, "vec4");
					assert_eq!(parameters.len(), 4);

					let x_param = &parameters[0];

					if let Nodes::Expression(Expressions::Literal { value }) = &x_param.node {
						assert_eq!(value, "0.0");
					} else {
						panic!("Not a literal");
					}
				} else {
					panic!("Not a function call");
				}
			} else {
				panic!("Not an assignment");
			}
		} else {
			panic!("Not a function");
		}
	}

	#[test]
	fn test_parse_function() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { name, .. } = node.node {
			assert_eq!(name, "root");
			assert_function(&node["main"]);
		} else {
			panic!("Not root node")
		}
	}

	#[test]
	fn test_parse_function_with_parameters_and_return_value() {
		let source = "
		add: fn (lhs: f32, rhs: f32) -> f32 {
			return lhs + rhs;
		}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		let function = &node["add"];
		if let Nodes::Function {
			name,
			params,
			return_type,
			statements,
			..
		} = &function.node
		{
			assert_eq!(*name, "add");
			assert_eq!(params.len(), 2);
			assert_eq!(*return_type, "f32");
			assert_eq!(statements.len(), 1);

			if let Nodes::Parameter { name, r#type } = &params[0].node {
				assert_eq!(*name, "lhs");
				assert_eq!(*r#type, "f32");
			} else {
				panic!("Expected parameter");
			}

			if let Nodes::Expression(Expressions::Return { value }) = &statements[0].node {
				let value = value.as_ref().expect("Expected return value");
				if let Nodes::Expression(Expressions::Operator { name, .. }) = &value.node {
					assert_eq!(*name, "+");
				} else {
					panic!("Expected return operator");
				}
			} else {
				panic!("Expected return statement");
			}
		} else {
			panic!("Expected function");
		}
	}

	#[test]
	fn parse_operators() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0) * 2.0;
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];

		if let Nodes::Function {
			name,
			statements,
			return_type,
			params,
			..
		} = &main_node.node
		{
			assert_eq!(*name, "main");
			assert_eq!(statements.len(), 2);
			assert_eq!(*return_type, "void");
			assert_eq!(params.len(), 0);

			assert_eq!(statements.len(), 2);

			let statement0 = &statements[0];

			if let Nodes::Expression(Expressions::Operator {
				name,
				left: var_decl,
				right: multiply,
			}) = &statement0.node
			{
				assert_eq!(*name, "=");

				if let Nodes::Expression(Expressions::VariableDeclaration { .. }) = var_decl.node {
				} else {
					panic!("Not a variable declaration");
				}

				if let Nodes::Expression(Expressions::Operator {
					name,
					left: vec4,
					right: literal,
				}) = &multiply.node
				{
					assert_eq!(*name, "*");

					if let Nodes::Expression(Expressions::Call { name, .. }) = &vec4.node {
						assert_named_type(name, "vec4");
					} else {
						panic!("Not a function call");
					}

					if let Nodes::Expression(Expressions::Literal { value }) = &literal.node {
						assert_eq!(value, "2.0");
					} else {
						panic!("Not a literal");
					}
				} else {
					panic!("Not an operator");
				}
			} else {
				panic!("Not an expression");
			}
		} else {
			panic!("Not a feature");
		}
	}

	#[test]
	fn builder_creates_assignment_expression() {
		let node = Node::assignment(Node::member_expression("albedo"), Node::literal_expression("1.0"));

		let Nodes::Expression(Expressions::Operator { name, left, right }) = node.node else {
			panic!("Expected assignment operator");
		};

		assert_eq!(name, "=");
		assert!(matches!(left.node, Nodes::Expression(Expressions::Member { name }) if name == "albedo"));
		assert!(matches!(right.node, Nodes::Expression(Expressions::Literal { value }) if value == "1.0"));
	}

	#[test]
	fn builder_creates_call_expression() {
		let node = Node::call(
			"vec4f",
			vec![
				Node::literal_expression("1.0"),
				Node::literal_expression("0.0"),
				Node::literal_expression("0.0"),
				Node::literal_expression("1.0"),
			],
		);

		let Nodes::Expression(Expressions::Call { name, parameters, .. }) = node.node else {
			panic!("Expected call expression");
		};

		assert_named_type(&name, "vec4f");
		assert_eq!(parameters.len(), 4);
	}

	#[test]
	fn builder_creates_variable_declaration_assignment() {
		let node = Node::let_assignment("roughness", "f32", Node::literal_expression("0.5"));

		let Nodes::Expression(Expressions::Operator { name, left, right }) = node.node else {
			panic!("Expected assignment operator");
		};

		assert_eq!(name, "=");
		assert!(matches!(
			left.node,
			Nodes::Expression(Expressions::VariableDeclaration { name, r#type, .. })
				if name == "roughness" && matches!(r#type, TypeName::Named("f32")),
		));
		assert!(matches!(right.node, Nodes::Expression(Expressions::Literal { value }) if value == "0.5"));
	}

	#[test]
	fn builder_program_lexes() {
		let program = Node::root_with_children(vec![Node::main_function(vec![Node::let_assignment(
			"albedo",
			"vec4f",
			Node::call(
				"vec4f",
				vec![
					Node::literal_expression("1.0"),
					Node::literal_expression("0.0"),
					Node::literal_expression("0.0"),
					Node::literal_expression("1.0"),
				],
			),
		)])]);

		crate::lex(program).expect("builder generated program should lex");
	}

	#[test]
	fn parse_accessor() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0) * 2.0;
	position.y = 2.0;
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(&tokens).expect("Failed to parse");

		print_tree(&node);

		if let Nodes::Scope { children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, statements, .. } = &main_node.node {
				assert_eq!(*name, "main");
				assert_eq!(statements.len(), 3);

				let statement1 = &statements[1];

				if let Nodes::Expression(Expressions::Operator {
					name,
					left: accessor,
					right: literal,
				}) = &statement1.node
				{
					assert_eq!(*name, "=");

					if let Nodes::Expression(Expressions::Accessor {
						left: position,
						right: y,
					}) = &accessor.node
					{
						if let Nodes::Expression(Expressions::Member { name }) = &position.node {
							assert_eq!(name, "position");
						} else {
							panic!("Not a member");
						}

						if let Nodes::Expression(Expressions::Member { name }) = &y.node {
							assert_eq!(name, "y");
						} else {
							panic!("Not a member");
						}
					} else {
						panic!("Not an accessor");
					}

					if let Nodes::Expression(Expressions::Literal { value }) = &literal.node {
						assert_eq!(value, "2.0");
					} else {
						panic!("Not a literal");
					}
				} else {
					panic!("Not an operator");
				}
			} else {
				panic!("Not a function");
			}
		} else {
			panic!("Not root node")
		}
	}

	#[test]
	fn test_parse_struct_and_function() {
		let source = "
Light: struct {
	position: vec3f,
	color: vec3f
}

#[vertex]
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { .. } = &node.node {
			assert_struct(&node["Light"]);
			assert_function(&node["main"]);
		} else {
			panic!("Not root node")
		}
	}

	#[test]
	fn test_parse_member() {
		let source = "color: In<vec4f>;";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { .. } = &node.node {
			let member_node = &node["color"];

			if let Nodes::Member { name, r#type } = &member_node.node {
				assert_eq!(*name, "color");
				assert_eq!(r#type, "In<vec4f>");
			} else {
				panic!("Not a feature");
			}
		}
	}

	#[test]
	fn test_parse_multiple_functions() {
		let source = "
used: fn () -> void {}
not_used: fn () -> void {}

main: fn () -> void {
	used();
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = node.node {
			assert_eq!(children.len(), 3);
		}
	}

	#[test]
	fn fragment_shader() {
		let source = r#"
		main: fn () -> void {
			let albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = node.node {
			assert_eq!(children.len(), 1);
		}
	}

	#[test]
	fn test_parse_accessor_and_assignment() {
		let source = "
main: fn () -> void {
	let n: f32 = intrinsic(0).y;
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, statements, .. } = &main_node.node {
				assert_eq!(*name, "main");
				assert_eq!(statements.len(), 1);

				let statement = &statements[0];

				if let Nodes::Expression(Expressions::Operator { name, left, right }) = &statement.node {
					assert_eq!(*name, "=");

					if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type, .. }) = &left.node {
						assert_eq!(*name, "n");
						assert_named_type(r#type, "f32");
					} else {
						panic!("Not a variable declaration");
					}

					if let Nodes::Expression(Expressions::Accessor { left, right }) = &right.node {
						if let Nodes::Expression(Expressions::Call { name, parameters, .. }) = &left.node {
							assert_named_type(name, "intrinsic");
							assert_eq!(parameters.len(), 1);

							if let Nodes::Expression(Expressions::Literal { value }) = &parameters[0].node {
								assert_eq!(value, "0");
							} else {
								panic!("Not a literal");
							}
						} else {
							panic!("Not a function call");
						}

						if let Nodes::Expression(Expressions::Member { name }) = &right.node {
							assert_eq!(name, "y");
						} else {
							panic!("Not a member");
						}
					} else {
						panic!("Not an accessor");
					}
				} else {
					panic!("Not an operator");
				}
			} else {
				panic!("Not a function");
			}
		} else {
			panic!("Not root node")
		}
	}

	#[test]
	fn parse_array_index_accessor() {
		let source = "
main: fn () -> void {
	let n: u32 = values[1];
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		if let Nodes::Function { statements, .. } = &main_node.node {
			let statement = &statements[0];
			if let Nodes::Expression(Expressions::Operator { right, .. }) = &statement.node {
				if let Nodes::Expression(Expressions::Accessor { left, right }) = &right.node {
					assert!(matches!(&left.node, Nodes::Expression(Expressions::Member { name }) if name == "values"));
					assert!(matches!(
						right.node,
						Nodes::Expression(Expressions::Expression(ref elements))
							if elements.len() == 1
								&& matches!(&elements[0].node, Nodes::Expression(Expressions::Literal { value }) if value == "1")
					));
				} else {
					panic!("Not an accessor");
				}
			} else {
				panic!("Not an operator");
			}
		} else {
			panic!("Not a function");
		}
	}

	#[test]
	fn parse_comparison_and_continue() {
		let source = r#"
		main: fn () -> void {
			for (let i: u32 = 0; i <= 4; i = i + 1) {
				if (i >= 2) {
					continue;
				}
			}
		}
		"#;

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");
		let main_node = &node["main"];

		let Nodes::Function { statements, .. } = &main_node.node else {
			panic!("Expected function");
		};

		let Nodes::ForLoop {
			condition, statements, ..
		} = &statements[0].node
		else {
			panic!("Expected for loop");
		};

		assert!(matches!(
			&condition.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if *name == "<="
		));

		let Nodes::Conditional { condition, statements } = &statements[0].node else {
			panic!("Expected conditional");
		};

		assert!(matches!(
			&condition.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if *name == ">="
		));
		assert!(matches!(statements[0].node, Nodes::Expression(Expressions::Continue)));
	}

	#[test]
	fn test_parse_const() {
		let source = "
PI: const f32 = 3.14;
";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let const_node = &node["PI"];

			if let Nodes::Const { name, r#type, value, .. } = &const_node.node {
				assert_eq!(*name, "PI");
				assert_named_type(r#type, "f32");

				if let Nodes::Expression(Expressions::Literal { value }) = &value.node {
					assert_eq!(*value, "3.14");
				} else {
					panic!("Expected a literal value, got: {:?}", value.node);
				}
			} else {
				panic!("Expected a const node, got: {:?}", const_node.node);
			}
		} else {
			panic!("Not root node");
		}
	}

	#[test]
	fn test_parse_const_with_expression() {
		let source = "
TAU: const f32 = 3.14 * 2.0;
";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let const_node = &node["TAU"];

		if let Nodes::Const { name, r#type, value, .. } = &const_node.node {
			assert_eq!(*name, "TAU");
			assert_named_type(r#type, "f32");

			if let Nodes::Expression(Expressions::Operator { name, .. }) = &value.node {
				assert_eq!(*name, "*");
			} else {
				panic!("Expected an operator expression, got: {:?}", value.node);
			}
		} else {
			panic!("Expected a const node");
		}
	}

	#[test]
	fn test_parse_const_array() {
		let source = "
		WEIGHTS: const f32 [ 3 ] = f32 [ 3 ](0.5, 0.25, 0.125);
";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let const_node = &node["WEIGHTS"];

		if let Nodes::Const { name, r#type, value } = &const_node.node {
			assert_eq!(*name, "WEIGHTS");
			assert_eq!(
				r#type,
				&TypeName::Array {
					element: Box::new(TypeName::Named("f32")),
					count: 3,
				}
			);

			if let Nodes::Expression(Expressions::Call { name, parameters }) = &value.node {
				assert_eq!(
					name,
					&TypeName::Array {
						element: Box::new(TypeName::Named("f32")),
						count: 3,
					}
				);
				assert_eq!(parameters.len(), 3);
			} else {
				panic!("Expected an array constructor call, got: {:?}", value.node);
			}
		} else {
			panic!("Expected a const node");
		}
	}

	#[test]
	fn parse_nested_array_type_without_flattening() {
		let tokens = tokenize("f32 [ 3 ] [ 4 ]").expect("Failed to tokenize");
		let mut tokens = tokens.tokens.iter();
		let base_type = tokens.next().expect("Expected a base type");
		let (type_name, mut iterator) = parse_type_name(tokens, base_type).expect("Failed to parse type");

		assert_eq!(
			type_name,
			TypeName::Array {
				element: Box::new(TypeName::Array {
					element: Box::new(TypeName::Named("f32")),
					count: 3,
				}),
				count: 4,
			}
		);
		assert!(iterator.next().is_none());
	}

	#[test]
	fn parse_conditional_block() {
		let source = "
main: fn () -> void {
	let n: u32 = 0;
	if (n < 1) {
		n = 2;
	}
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		if let Nodes::Function { statements, .. } = &main_node.node {
			assert_eq!(statements.len(), 2);

			let conditional = &statements[1];
			if let Nodes::Conditional { condition, statements } = &conditional.node {
				assert_eq!(statements.len(), 1);

				assert!(matches!(
					condition.node,
					Nodes::Expression(Expressions::Operator { name, .. }) if name == "<"
				));

				assert!(matches!(
					statements[0].node,
					Nodes::Expression(Expressions::Operator { name, .. }) if name == "="
				));
			} else {
				panic!("Expected conditional block");
			}
		} else {
			panic!("Expected main function");
		}
	}

	#[test]
	fn parse_for_loop_block() {
		let source = "
main: fn () -> void {
	let sum: u32 = 0;
	for (let i: u32 = 0; i < 4; i = i + 1) {
		sum = sum + i;
	}
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		let Nodes::Function { statements, .. } = &main_node.node else {
			panic!("Expected main function");
		};

		assert_eq!(statements.len(), 2);

		let for_loop = &statements[1];
		let Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} = &for_loop.node
		else {
			panic!("Expected for loop block");
		};

		assert!(matches!(
			initializer.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "="
		));
		assert!(matches!(
			condition.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "<"
		));
		assert!(matches!(
			update.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "="
		));
		assert_eq!(statements.len(), 1);
	}

	#[test]
	fn parse_bitwise_expression() {
		let source = "
main: fn () -> void {
	let packed: u32 = 1 << 8 | 2 & 255;
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		let main_node = &node["main"];
		let Nodes::Function { statements, .. } = &main_node.node else {
			panic!("Expected main function");
		};

		let Nodes::Expression(Expressions::Operator { name, right, .. }) = &statements[0].node else {
			panic!("Expected assignment expression");
		};
		assert_eq!(*name, "=");

		let Nodes::Expression(Expressions::Operator { name, left, right }) = &right.node else {
			panic!("Expected bitwise or expression");
		};
		assert_eq!(*name, "|");

		assert!(matches!(
			left.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "<<"
		));
		assert!(matches!(
			right.node,
			Nodes::Expression(Expressions::Operator { name, .. }) if name == "&"
		));
	}

	#[test]
	fn parse_compute_vertex_position() {
		let source = r#"
compute_vertex_position: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> vec4f {
	let vertex_index: u32 = compute_vertex_index(mesh, meshlet, primitive_index);
	return vec4f(
		vertex_positions.positions[vertex_index].x,
		vertex_positions.positions[vertex_index].y,
		vertex_positions.positions[vertex_index].z,
		1.0
	);
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");
		let func = &node["compute_vertex_position"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_compute_triangle() {
		let source = r#"
compute_triangle: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> vec3u {
	return vec3u(
		primitive_indices.primitive_indices[(mesh.base_triangle_index + u16_to_u32(meshlet.triangle_offset) + primitive_index) * 3 + 0],
		primitive_indices.primitive_indices[(mesh.base_triangle_index + u16_to_u32(meshlet.triangle_offset) + primitive_index) * 3 + 1],
		primitive_indices.primitive_indices[(mesh.base_triangle_index + u16_to_u32(meshlet.triangle_offset) + primitive_index) * 3 + 2]
	);
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		println!("Tokens: {:?}", tokens.tokens);
		let node = parse(&tokens).expect("Failed to parse");
		let func = &node["compute_triangle"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_grouping_parentheses() {
		// Minimal repro: grouping parentheses inside a function call
		let source = r#"
main: fn () -> void {
	foo((a + b) * 3);
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		println!("Tokens: {:?}", tokens.tokens);
		let node = parse(&tokens).expect("Failed to parse");
		let func = &node["main"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_conditional_comparing_a_push_constant_member() {
		let source = r#"
main: fn () -> void {
	let local_vertex_index: u32 = thread_id().x;
	if (local_vertex_index >= push_constant.vertex_count) {
		return;
	}
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse push-constant comparison");
		let func = &node["main"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_grouped_arithmetic_inside_a_conditional() {
		let source = r#"
main: fn () -> void {
	if (total_weight > 0.00000001) {
		let column0: vec4f = (
			matrix0.column0 * weights.x
			+ matrix1.column0 * weights.y
		) * inverse_total_weight;
	}
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse grouped conditional arithmetic");
		let func = &node["main"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn parse_process_meshlet() {
		let source = r#"
process_meshlet: fn (instance_index: u32, matrix: mat4f) -> void {
	let mesh: Mesh = meshes.meshes[instance_index];
	let meshlet_index: u32 = threadgroup_position() + mesh.base_meshlet_index;
	let meshlet: Meshlet = meshlets.meshlets[meshlet_index];
	let primitive_index: u32 = thread_idx();

	set_mesh_output_counts(u8_to_u32(meshlet.primitive_count), u8_to_u32(meshlet.triangle_count));

	if (primitive_index < u8_to_u32(meshlet.primitive_count)) {
		set_mesh_vertex_position(
			primitive_index,
			matrix * mesh.model * compute_vertex_position(mesh, meshlet, primitive_index)
		);
	}

	if (primitive_index < u8_to_u32(meshlet.triangle_count)) {
		set_mesh_triangle(primitive_index, compute_triangle(mesh, meshlet, primitive_index));
		out_instance_index[primitive_index] = instance_index;
		out_primitive_index[primitive_index] = meshlet_index << 8 | primitive_index & 255;
	}
}
"#;
		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");
		let func = &node["process_meshlet"];
		assert!(matches!(&func.node, Nodes::Function { .. }));
	}

	#[test]
	fn truncated_function_returns_an_error() {
		let tokens = tokenize("main: fn () -> void {").expect("Failed to tokenize");

		assert!(matches!(parse(&tokens), Err(ParsingFailReasons::BadSyntax { .. })));
	}
}
