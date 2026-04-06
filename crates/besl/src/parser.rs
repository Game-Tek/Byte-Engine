//! The parser module contains all code related to the parsing of the BESL language and the generation of the JSPD.
//!
//! # Example beShader
//!
//! ```glsl
//! Light: struct {
//! 	position: vec3,
//! 	color: vec3,
//! }
//!
//! main: fn () -> void {
//! 	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
//! }
//! ```
//!
//! The `parse` function is the entry point.
//! The parser consumes an stream of tokens and creates nodes with Nodes.
//! All nodes which have cross references only do so by name.
//! Those relations are resolved later by the lexer which performs a grammar analysis.

use crate::tokenizer;

/// Generates a syntax tree from BESL source code tokens.
/// The syntax tree is just another representation of the source code.
/// It is missing the final transformation step, which is the lexing step.
pub type NodeReference<'a> = &'a Node<'a>;

/// Generates a syntax tree from BESL source code tokens.
/// The syntax tree is just another representation of the source code.
/// It is missing the final transformation step, which is the lexing step.
pub(super) fn parse<'i, 'a: 'i>(tokens: &'i tokenizer::Tokens<'a>) -> Result<Node<'a>, ParsingFailReasons> {
	let mut iterator = tokens.tokens.iter();

	let parsers = [parse_struct, parse_function, parse_macro, parse_const, parse_member];

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
use std::num::NonZeroUsize;

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

	pub fn member_expression(name: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Member { name }),
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

	pub fn main_function(statements: Vec<Node<'a>>) -> Node<'a> {
		make_function("main", Vec::new(), "void", statements)
	}

	pub fn binding(name: &'a str, r#type: Node<'a>, set: u32, descriptor: u32, read: bool, write: bool) -> Node<'a> {
		Node {
			node: Nodes::Binding {
				name,
				r#type: Box::new(r#type),
				set,
				descriptor,
				read,
				write,
				count: None,
			},
		}
	}

	pub fn binding_array(
		name: &'a str,
		r#type: Node<'a>,
		set: u32,
		descriptor: u32,
		read: bool,
		write: bool,
		count: u32,
	) -> Node<'a> {
		Node {
			node: Nodes::Binding {
				name,
				r#type: Box::new(r#type),
				set,
				descriptor,
				read,
				write,
				count: NonZeroUsize::new(count as usize),
			},
		}
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

	pub fn glsl(code: impl Into<Cow<'a, str>>, input: &'a [&'a str], output: &'a [&'a str]) -> Node<'a> {
		make_raw_code(Some(code.into()), None, input, output)
	}

	pub fn hlsl(code: impl Into<Cow<'a, str>>, input: &'a [&'a str], output: &'a [&'a str]) -> Node<'a> {
		make_raw_code(None, Some(code.into()), input, output)
	}

	pub fn raw_code(
		glsl: Option<Cow<'a, str>>,
		hlsl: Option<Cow<'a, str>>,
		input: &'a [&'a str],
		output: &'a [&'a str],
	) -> Node<'a> {
		make_raw_code(glsl, hlsl, input, output)
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
		Node {
			node: Nodes::Output {
				name,
				format,
				location,
				count: None,
			},
		}
	}

	pub fn output_array(name: &'a str, format: &'a str, location: u8, count: u32) -> Node<'a> {
		Node {
			node: Nodes::Output {
				name,
				format,
				location,
				count: NonZeroUsize::new(count as usize),
			},
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
			Nodes::Conditional { .. } => None,
			Nodes::Binding { name, .. } => Some(name),
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
			Nodes::Input { name, .. } | Nodes::Output { name, .. } => Some(name),
			Nodes::Const { name, .. } => Some(name),
			Nodes::Null => None,
		}
	}

	pub fn node_mut(&mut self) -> &mut Nodes<'a> {
		// TODO: maybe do not expose nodes
		&mut self.node
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

		match &mut self.node {
			Nodes::Scope { children, .. } => {
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
			_ => {}
		}
	}
}

#[derive(Clone, Debug)]
pub enum Nodes<'a> {
	/// A special kind of node. Mostly used for partially implemented features.
	Null,
	/// Like a Rust module. A logical division/grouping of code.
	Scope {
		/// The scope's name. Used in code when importing or declaring namespaces.
		name: &'a str,
		children: Vec<Node<'a>>,
	},
	/// A struct declaration. A struct is a collection of fields.
	Struct {
		name: &'a str,
		fields: Vec<Node<'a>>,
	},
	/// A member field. Usually used inside a struct.
	Member {
		name: &'a str,
		r#type: String,
	},
	/// A funcion declaration and definition node.
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
	/// A binding declaration. A binding is a resource that can be used in the shader.
	Binding {
		name: &'a str,
		r#type: Box<Node<'a>>,
		set: u32,
		descriptor: u32,
		read: bool,
		write: bool,
		count: Option<NonZeroUsize>,
	},
	/// A specialization constant. A specialization constant is a constant that can be set when creating a pipeline at runtime.
	Specialization {
		name: &'a str,
		r#type: &'a str,
	},
	/// A push constant block. A push constant is a small buffer that can have values pushed during rendering.
	PushConstant {
		members: Vec<Node<'a>>,
	},
	/// An abstract type. Usually used to define primitive types such as `f32`.
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
	Literal {
		name: &'a str,
		body: Box<Node<'a>>,
	},
	Parameter {
		name: &'a str,
		r#type: &'a str,
	},
	/// A module-level constant variable declaration. Used to define compile-time constant values.
	Const {
		name: &'a str,
		r#type: &'a str,
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
		name: &'a str,
	},
	Literal {
		value: &'a str,
	},
	Call {
		name: &'a str,
		parameters: Vec<Node<'a>>,
	},
	Operator {
		name: &'a str,
		left: Box<Node<'a>>,
		right: Box<Node<'a>>,
	},
	VariableDeclaration {
		name: &'a str,
		r#type: &'a str,
	},
	RawCode {
		glsl: Option<&'a str>,
		hlsl: Option<&'a str>,
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
}

#[derive(Clone, Debug)]
pub(super) enum Atoms<'a> {
	Keyword,
	Accessor,
	Member { name: &'a str },
	Literal { value: &'a str },
	FunctionCall { name: &'a str, parameters: Vec<Vec<Atoms<'a>>> },
	Operator { name: &'a str },
	VariableDeclaration { name: &'a str, r#type: &'a str },
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

fn make_raw_code<'a>(
	glsl: Option<Cow<'a, str>>,
	hlsl: Option<Cow<'a, str>>,
	input: &'a [&'a str],
	output: &'a [&'a str],
) -> Node<'a> {
	Node {
		node: Nodes::RawCode {
			glsl,
			hlsl,
			input,
			output,
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
			Atoms::Accessor => 4,
			Atoms::Member { .. } => 0,
			Atoms::Literal { .. } => 0,
			Atoms::FunctionCall { .. } => 0,
			Atoms::Operator { name } => match *name {
				"=" => 8,
				"|" => 7,
				"&" => 6,
				"<" => 5,
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

/// Type of the result of a parser.
type FeatureParserResult<'i, 'a> = Result<(Node<'a>, std::slice::Iter<'i, &'a str>), ParsingFailReasons>;

/// A parser is a function that tries to parse a sequence of tokens.
type FeatureParser<'i, 'a> = fn(std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a>;

type ExpressionParserResult<'i, 'a> = Result<(Vec<Atoms<'a>>, std::slice::Iter<'i, &'a str>), ParsingFailReasons>;
type ExpressionParser<'i, 'a> = fn(std::slice::Iter<'i, &'a str>, Vec<Atoms<'a>>) -> ExpressionParserResult<'i, 'a>;

/// Execute a list of parsers on a stream of tokens.
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

/// Tries to execute a list of parsers on a stream of tokens. But it's ok if none of them can handle the syntax.
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

/// Execute a list of parsers on a stream of tokens.
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

/// Tries to execute a list of parsers on a stream of tokens. But it's ok if none of them can handle the syntax.
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
							name: *name,
							left: Box::new(left),
							right: Box::new(right),
						}),
					}
				}
				Atoms::FunctionCall { name, parameters } => {
					let parameters = parameters.iter().map(|v| atoms_to_node(v)).collect::<Vec<_>>();
					Node {
						node: Nodes::Expression(Expressions::Call { name: *name, parameters }),
					}
				}
				Atoms::Literal { value } => Node {
					node: Nodes::Expression(Expressions::Literal { value: *value }),
				},
				Atoms::Member { name } => Node {
					node: Nodes::Expression(Expressions::Member { name: *name }),
				},
				_ => panic!("Unexpected atom in const expression"),
			}
		} else {
			panic!("No atoms in const expression");
		}
	}

	let value = atoms_to_node(&expressions);

	Ok((Node::constant(name, r#type, value), iterator))
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
			iterator.next();
			r#type.push_str("<");
			let next = iterator.next().ok_or(ParsingFailReasons::BadSyntax {
				message: format!("Expected to find type while parsing generic argument for member {}", name),
			})?;
			r#type.push_str(next.as_ref());
			iterator.next();
			r#type.push_str(">");
		}
	}

	let node = Node::member(name, &r#type);

	iterator.next().ok_or(ParsingFailReasons::BadSyntax {
		message: format!("Expected semicolon"),
	})?; // Skip semicolon

	Ok(((node), iterator))
}

fn parse_macro<'i, 'a: 'i>(iterator: std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a> {
	let mut iter = iterator;

	iter.next_str("#")?;
	iter.next_str("[")?;
	iter.next_identifier().map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find macro name after #[."),
		},
		_ => e,
	})?;
	iter.next_str("]").map_err(|e| match e {
		ParsingFailReasons::NotMine => ParsingFailReasons::BadSyntax {
			message: format!("Expected to find ] after macro name."),
		},
		_ => e,
	})?;

	Ok((make_scope("MACRO", vec![]).into(), iter))
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

		fields.push(make_member(v, &type_name).into());
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

	expressions.push(Atoms::VariableDeclaration {
		name: variable_name,
		r#type: variable_type,
	});

	let possible_following_expressions: Vec<ExpressionParser<'i, 'a>> = vec![parse_operator];

	let expressions = execute_expression_parsers(&possible_following_expressions, iterator, expressions)?;

	Ok(expressions)
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
	let (expressions, mut iterator) = execute_expression_parsers(&[parse_rvalue], iterator, expressions)?;
	iterator.next_str("]")?;

	let lexers = vec![parse_operator, parse_accessor, parse_index_accessor];
	try_execute_expression_parsers(&lexers, iterator.clone(), expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

fn is_number_literal(s: &str) -> bool {
	s.chars().all(|c| c.is_digit(10) || c == '.')
}

fn parse_literal<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let value = iterator.next_is(is_number_literal)?;

	expressions.push(Atoms::Literal { value });

	let possible_following_expressions = vec![parse_operator, parse_accessor, parse_index_accessor];

	try_execute_expression_parsers(&possible_following_expressions, iterator.clone(), expressions.clone())
		.unwrap_or(Ok((expressions, iterator)))
}

fn parse_rvalue<'i, 'a: 'i>(
	iterator: std::slice::Iter<'i, &'a str>,
	expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let parsers = vec![parse_function_call, parse_literal, parse_variable];

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
				|| v == "<<" || v == ">>"
				|| v == "&" || v == "|"
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
		}
		.into();
	}

	let max_precedence_item = atoms.iter().enumerate().max_by_key(|(_, v)| v.precedence());

	if let Some((i, e)) = max_precedence_item {
		match e {
			Atoms::Keyword => Node {
				node: Nodes::Expression(Expressions::Return { value: None }),
			},
			Atoms::Operator { name } => {
				let left = expression_atoms_to_node(&atoms[..i]);
				let right = expression_atoms_to_node(&atoms[i + 1..]);

				Node {
					node: Nodes::Expression(Expressions::Operator {
						name: *name,
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
			Atoms::FunctionCall { name, parameters } => {
				let parameters = parameters.iter().map(|v| expression_atoms_to_node(v)).collect::<Vec<_>>();

				Node {
					node: Nodes::Expression(Expressions::Call { name: *name, parameters }),
				}
			}
			Atoms::Literal { value } => Node {
				node: Nodes::Expression(Expressions::Literal { value: *value }),
			},
			Atoms::Member { name } => Node {
				node: Nodes::Expression(Expressions::Member { name: *name }),
			},
			Atoms::VariableDeclaration { name, r#type } => Node {
				node: Nodes::Expression(Expressions::VariableDeclaration {
					name: *name,
					r#type: *r#type,
				}),
			},
		}
		.into()
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

fn parse_function_call<'i, 'a: 'i>(
	mut iterator: std::slice::Iter<'i, &'a str>,
	mut expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let function_name = iterator.next_identifier()?;
	iterator.next_str("(")?;

	let mut parameters = vec![];

	loop {
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

	let parsers = vec![parse_keywords, parse_var_decl, parse_function_call, parse_variable];

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
			if **iterator.clone().peekable().peek().unwrap() == "}" {
				iterator.next();
				break;
			} else {
				let token = iterator.clone().peekable().peek().copied().copied().unwrap_or("<eof>");
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
		match &self.node {
			Nodes::Scope { children, .. } => {
				for child in children {
					match child.node {
						Nodes::Scope {
							name: child_name,
							children: _,
						} => {
							if child_name == index {
								return child;
							}
						}
						Nodes::Struct {
							name: child_name,
							fields: _,
						} => {
							if child_name == index {
								return child;
							}
						}
						Nodes::Member {
							name: child_name,
							r#type: _,
						} => {
							if child_name == index {
								return child;
							}
						}
						Nodes::Function { name: child_name, .. } => {
							if child_name == index {
								return child;
							}
						}
						Nodes::Const { name: child_name, .. } => {
							if child_name == index {
								return child;
							}
						}
						_ => {}
					}
				}
			}
			Nodes::Struct { fields, .. } => {
				for field in fields {
					if let Nodes::Member { name: child_name, .. } = field.node {
						if child_name == index {
							return &field;
						}
					}
				}
			}
			_ => {
				panic!("Cannot search  in these");
			}
		}

		panic!("Not found");
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

				if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) = var_decl.node {
					assert_eq!(name, "position");
					assert_eq!(r#type, "vec4f");
				} else {
					panic!("Not an variable declaration");
				}

				if let Nodes::Expression(Expressions::Call { name, parameters }) = &function_call.node {
					assert_eq!(*name, "vec4");
					assert_eq!(parameters.len(), 4);

					let x_param = &parameters[0];

					if let Nodes::Expression(Expressions::Literal { value }) = x_param.node {
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

					if let Nodes::Expression(Expressions::Call { name, .. }) = vec4.node {
						assert_eq!(name, "vec4");
					} else {
						panic!("Not a function call");
					}

					if let Nodes::Expression(Expressions::Literal { value }) = literal.node {
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
						if let Nodes::Expression(Expressions::Member { name }) = position.node {
							assert_eq!(name, "position");
						} else {
							panic!("Not a member");
						}

						if let Nodes::Expression(Expressions::Member { name }) = y.node {
							assert_eq!(name, "y");
						} else {
							panic!("Not a member");
						}
					} else {
						panic!("Not an accessor");
					}

					if let Nodes::Expression(Expressions::Literal { value }) = literal.node {
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

					if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) = left.node {
						assert_eq!(name, "n");
						assert_eq!(r#type, "f32");
					} else {
						panic!("Not a variable declaration");
					}

					if let Nodes::Expression(Expressions::Accessor { left, right }) = &right.node {
						if let Nodes::Expression(Expressions::Call { name, parameters }) = &left.node {
							assert_eq!(*name, "intrinsic");
							assert_eq!(parameters.len(), 1);

							if let Nodes::Expression(Expressions::Literal { value }) = parameters[0].node {
								assert_eq!(value, "0");
							} else {
								panic!("Not a literal");
							}
						} else {
							panic!("Not a function call");
						}

						if let Nodes::Expression(Expressions::Member { name }) = right.node {
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
					assert!(matches!(left.node, Nodes::Expression(Expressions::Member { name }) if name == "values"));
					assert!(matches!(right.node, Nodes::Expression(Expressions::Literal { value }) if value == "1"));
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
	fn test_parse_const() {
		let source = "
PI: const f32 = 3.14;
";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(&tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let const_node = &node["PI"];

			if let Nodes::Const { name, r#type, value } = &const_node.node {
				assert_eq!(*name, "PI");
				assert_eq!(*r#type, "f32");

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

		if let Nodes::Const { name, r#type, value } = &const_node.node {
			assert_eq!(*name, "TAU");
			assert_eq!(*r#type, "f32");

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
}
