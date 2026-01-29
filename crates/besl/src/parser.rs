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
pub(super) fn parse<'a>(tokens: tokenizer::Tokens<'a>) -> Result<Node<'a>, ParsingFailReasons> {
	let mut iterator = tokens.tokens.iter();

	let parsers = [
		parse_struct,
		parse_function,
		parse_macro,
		parse_member,
	];

	let mut children: Vec<Node> = Vec::with_capacity(64);

	loop {
		let (expression, iter) = execute_parsers(parsers.as_slice(), iterator,)?;

		children.push(expression);

		iterator = iter;

		if iterator.len() == 0 {
			break;
		}
	}

	Ok(make_scope("root", children),)
}

use std::num::NonZeroUsize;

#[derive(Clone, Debug)]
pub struct Node<'a> {
	pub(crate) node: Nodes<'a>,
}

impl <'a> Node<'a> {
	pub fn root() -> Node<'a> {
		make_scope("root", Vec::new())
	}

	pub fn root_with_children(children: Vec<Node>) -> Node<'a> {
		make_scope("root", children)
	}

	pub fn scope(name: &str, children: Vec<Node>) -> Node<'a> {
		make_scope(name, children)
	}

	pub fn r#struct(name: &str, fields: Vec<Node>) -> Node<'a> {
		make_struct(name, fields)
	}

	pub fn member(name: &str, r#type: &str) -> Node<'a> {
		make_member(name, r#type)
	}

	pub fn member_expression(name: &str) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Member { name }),
		}
	}

	pub fn function(name: &str, params: Vec<Node>, return_type: &str, statements: Vec<Node>) -> Node<'a> {
		make_function(name, params, return_type, statements)
	}

	pub fn main_function(statements: Vec<Node>) -> Node<'a> {
		make_function("main", Vec::new(), "void", statements)
	}

	pub fn binding(name: &str, r#type: Node, set: u32, descriptor: u32, read: bool, write: bool) -> Node<'a> {
		Node {
			node: Nodes::Binding {
				name,
				r#type,
				set,
				descriptor,
				read,
				write,
				count: None,
			},
		}
	}

	pub fn binding_array(name: &str, r#type: Node, set: u32, descriptor: u32, read: bool, write: bool, count: u32) -> Node<'a> {
		Node {
			node: Nodes::Binding {
				name,
				r#type,
				set,
				descriptor,
				read,
				write,
				count: NonZeroUsize::new(count as usize),
			},
		}
	}

	pub fn specialization(name: &str, r#type: &str) -> Node<'a> {
		Node {
			node: Nodes::Specialization {
				name,
				r#type,
			},
		}
	}

	pub fn buffer(name: &str, members: Vec<Node>) -> Node<'a> {
		Node {
			node: Nodes::Type {
				name,
				members,
			},
		}
	}

	pub fn image(format: &str) -> Node {
		Node {
			node: Nodes::Image {
				format,
			},
		}
	}

	pub fn push_constant(members: Vec<Node>) -> Node {
		Node {
			node: Nodes::PushConstant {
				members,
			},
		}
	}

	pub fn combined_image_sampler() -> Node<'a> {
		Node {
			node: Nodes::CombinedImageSampler {
				format: "",
			},
		}
	}

	pub fn combined_array_image_sampler() -> Node<'a> {
		Node {
			node: Nodes::CombinedImageSampler {
				format: "ArrayTexture2D",
			},
		}
	}

	pub fn r#macro(name: &str, body: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Macro {
				name,
				body,
			}),
		}
	}

	pub fn sentence(expressions: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Expression(expressions)),
		}
	}

	pub fn glsl(code: &str, input: &[&str], output: Vec<&str>) -> Node<'a> {
		make_raw_code(Some(code), None, input, output)
	}

	pub fn hlsl(code: &str, input: &[&str], output: Vec<&str>) -> Node<'a> {
		make_raw_code(None, Some(code), input, output)
	}

	pub fn raw_code(glsl: Option<&str>, hlsl: Option<&str>, input: &[&str], output: Vec<&str>) -> Node<'a> {
		make_raw_code(glsl.map(intern), hlsl.map(intern), input, output)
	}

	pub fn literal(name: &str, body: Node) -> Node<'a> {
		Node {
			node: Nodes::Literal {
				name,
				body,
			},
		}
	}

	pub fn input(name: &str, format: &str, location: u8) -> Node<'a> {
		Node {
			node: Nodes::Input {
				name,
				format,
				location,
			},
		}
	}

	pub fn output(name: &str, format: &str, location: u8) -> Node<'a> {
		Node {
			node: Nodes::Output {
				name,
				format,
				location,
			},
		}
	}

	pub fn intrinsic(name: &str, parameters: Node<'a>, body: Node<'a>, r#return: &str) -> Node<'a> {
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

	pub fn parameter(name: &str, r#type: &str) -> Node<'a> {
		Node {
			node: Nodes::Parameter {
				name,
				r#type,
			},
		}
	}

	pub fn name(&self) -> Option<&'a str> {
		match &self.node {
			Nodes::Scope { name, .. } => Some(name),
			Nodes::Struct { name, .. } => Some(name),
			Nodes::Member { name, .. } => Some(name),
			Nodes::Function { name, .. } => Some(name),
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
			Nodes::Null => None,
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
		r#type: &'a str,
	},
	/// A funcion declaration and definition node.
	Function {
		name: &'a str,
		params: Vec<Node<'a>>,
		return_type: &'a str,
		statements: Vec<Node<'a>>,
	},
	/// A binding declaration. A binding is a resource that can be used in the shader.
	Binding {
		name: &'a str,
		r#type: Node<'a>,
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
		glsl: Option<&'a str>,
		hlsl: Option<&'a str>,
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
	},
	Literal {
		name: &'a str,
		body: Node<'a>,
	},
	Parameter {
		name: &'a str,
		r#type: &'a str,
	},
}

#[derive(Clone, Debug)]
pub enum Expressions<'a> {
	Expression(Vec<Node<'a>>),
	Accessor{ left: Node<'a>, right: Node<'a>, },
	Member{ name: &'a str },
	Literal{ value: &'a str, },
	Call{ name: &'a str, parameters: Vec<Node<'a>> },
	Operator{ name: &'a str, left: Node<'a>, right: Node<'a>, },
	VariableDeclaration{ name: &'a str, r#type: &'a str, },
	RawCode{ glsl: Option<&'a str>, hlsl: Option<&'a str>, input: &'a [&'a str], output: &'a [&'a str], },
	Macro{ name: &'a str, body: Node<'a>, },
	Return,
}

#[derive(Clone, Debug)]
pub(super) enum Atoms<'a> {
	Keyword,
	Accessor,
	Member{ name: &'a str },
	Literal{ value: &'a str, },
	FunctionCall{ name: &'a str, parameters: Vec<Vec<Atoms<'a>>> },
	Operator{ name: &'a str, },
	VariableDeclaration{ name: &'a str, r#type: &'a str, },
}

#[derive(Debug)]
pub(super) enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax{ message: String, },
	StreamEndedPrematurely,
}

fn make_scope<'a>(name: &str, children: Vec<Node<'a>>) -> Node<'a> {
	Node {
		node: Nodes::Scope {
			name,
			children,
		},
	}
}

fn make_member<'a>(name: &str, r#type: &str) -> Node<'a> {
	Node {
		node: Nodes::Member {
			name,
			r#type,
		},
	}
}

fn make_struct<'a>(name: &str, children: Vec<Node<'a>>) -> Node<'a> {
	Node {
		node: Nodes::Struct {
			name,
			fields: children,
		},
	}
}

fn make_function<'a>(name: &str, params: Vec<Node<'a>>, return_type: &str, statements: Vec<Node<'a>>) -> Node<'a> {
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
			Atoms::Accessor => 4,
			Atoms::Member{ .. } => 0,
			Atoms::Literal{ .. } => 0,
			Atoms::FunctionCall{ .. } => 0,
			Atoms::Operator{ name } => {
				match *name {
					"=" => 5,
					"+" => 3,
					"-" => 3,
					"*" => 2,
					"/" => 2,
					_ => 0,
				}
			},
			Atoms::VariableDeclaration{ .. } => 0,
		}
	}
}

/// Type of the result of a parser.
type FeatureParserResult<'a> = Result<(Node<'a>, std::slice::Iter<'a, &'a str>), ParsingFailReasons>;

/// A parser is a function that tries to parse a sequence of tokens.
type FeatureParser<'a> = fn(std::slice::Iter<'a, &'a str>) -> FeatureParserResult<'a>;

type ExpressionParserResult<'a> = Result<(Vec<Atoms<'a>>, std::slice::Iter<'a, &'a str>), ParsingFailReasons>;
type ExpressionParser<'a> = fn(std::slice::Iter<'a, &'a str>, Vec<Atoms<'a>>) -> ExpressionParserResult<'a>;

/// Execute a list of parsers on a stream of tokens.
fn execute_parsers<'a>(parsers: &[FeatureParser<'a>], mut iterator: std::slice::Iter<'a, &'a str>) -> FeatureParserResult<'a> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(),) {
			return Ok(r);
		}
	}

	Err(ParsingFailReasons::BadSyntax{ message: format!("Tried several parsers none could handle the syntax for statement: {}", iterator.next().unwrap()) }) // No parser could handle this syntax.
}

/// Tries to execute a list of parsers on a stream of tokens. But it's ok if none of them can handle the syntax.
fn try_execute_parsers<'a>(parsers: &[FeatureParser<'a>], iterator: std::slice::Iter<'a, &'a str>,) -> Option<FeatureParserResult<'a>> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(),) {
			return Some(Ok(r));
		}
	}

	None
}

/// Execute a list of parsers on a stream of tokens.
fn execute_expression_parsers<'a>(parsers: &[ExpressionParser<'a>], mut iterator: std::slice::Iter<'a, &'a str>, expressions: Vec<Atoms<'a>>) -> ExpressionParserResult<'a> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), expressions.clone()) {
			return Ok(r);
		}
	}

	Err(ParsingFailReasons::BadSyntax{ message: format!("Tried several parsers none could handle the syntax for statement: {}", iterator.next().unwrap()) }) // No parser could handle this syntax.
}

/// Tries to execute a list of parsers on a stream of tokens. But it's ok if none of them can handle the syntax.
fn try_execute_expression_parsers<'a>(parsers: &[ExpressionParser<'a>], iterator: std::slice::Iter<'a, &'a str>, expressions: Vec<Atoms<'a>>,) -> Option<ExpressionParserResult<'a>> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), expressions.clone()) {
			return Some(Ok(r));
		}
	}

	None
}

fn is_identifier(c: char) -> bool { // TODO: validate number at end of identifier
	c.is_alphanumeric() || c == '_'
}

fn parse_member<'a>(mut iterator: std::slice::Iter<'a, &'a str>,) -> FeatureParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|&v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	let mut r#type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find type while parsing member {}.", name) })?.to_string();

	if let Some(&&n) = iterator.clone().peekable().peek() {
		if n == "<"	{
			iterator.next();
			r#type.push_str("<");
			let next = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find type while parsing generic argument for member {}", name) })?;
			r#type.push_str(next.as_ref());
			iterator.next();
			r#type.push_str(">");
		}
	}

	let node = Node::member(name.as_ref(), &r#type);

	iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected semicolon") })?; // Skip semicolon

	Ok(((node), iterator))
}

fn parse_macro<'a>(iterator: std::slice::Iter<'a, &'a str>,) -> FeatureParserResult<'a> {
	let mut iter = iterator;

	iter.next().and_then(|&v| if v == "#" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().and_then(|&v| if v == "[" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find macro name.") })?;
	iter.next().and_then(|&v| if v == "]" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find ] after macro.") })?;

	Ok((make_scope("MACRO", vec![]).into(), iter))
}

fn parse_struct<'a>(mut iterator: std::slice::Iter<'a, &'a str>,) -> FeatureParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(char::is_alphanumeric) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|&v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|&v| if v == "struct" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|&v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find {{ after struct {} declaration", name.as_ref()) })?;

	let mut fields = vec![];

	while let Some(&v) = iterator.next() {
		if v == "}" {
			break;
		} else if v == "," {
			continue;
		}

		let colon = iterator.next().unwrap();

		if *colon != ":" { return Err(ParsingFailReasons::BadSyntax{ message: format!("Expected to find : after name for member {} in struct {}", v, name) }); }

		let type_name = iterator.next().unwrap();

		if !type_name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::BadSyntax{ message: format!("Expected to find a type name after : for member {} in struct {}", v.as_ref(), name.as_ref()) }); }

		// See if is array type
		let type_name = if iterator.clone().peekable().peek().map(|v| v.as_ref()) == Some("[") {
			iterator.next();
			let count = iterator.next().and_then(|v| v.parse::<u32>().ok()).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find a number after [ for member {} in struct {}", v.as_ref(), name.as_ref()) })?;
			iterator.next().unwrap();
			format!("{}[{}]", type_name, count)
		} else {
			type_name.to_string()
		};

		fields.push(make_member(v.as_ref(), &type_name).into());
	}

	let node = Node::r#struct(name.as_ref(), fields);

	Ok((node, iterator))
}

fn parse_var_decl<'a>(mut iterator: std::slice::Iter<'a, &'a str>, mut expressions: Vec<Atoms<'a>>,) -> ExpressionParserResult<'a> {
	let _ = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|&v| if v == "let" { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	let variable_name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|&v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	let variable_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find a type for variable {}", variable_name) }).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	expressions.push(Atoms::VariableDeclaration{ name: variable_name.clone(), r#type: variable_type.clone() });

	let possible_following_expressions: Vec<ExpressionParser> = vec![
		parse_operator,
	];

	let expressions = execute_expression_parsers(&possible_following_expressions, iterator, expressions)?;

	Ok(expressions)
}

fn parse_keywords<'a>(mut iterator: std::slice::Iter<'a, &'a str>, mut expressions: Vec<Atoms<'a>>) -> ExpressionParserResult<'a> {
	iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|&v| if v == "return" { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	expressions.push(Atoms::Keyword);

	// let lexers = vec![
	// 	parse_operator,
	// 	parse_accessor,
	// ];

	// try_execute_expression_parsers(&lexers, iterator.clone(), expressions.clone()).unwrap_or(Ok((expressions, iterator)));

	Ok((expressions, iterator))
}

fn parse_variable<'a>(mut iterator: std::slice::Iter<'a, &'a str>, mut expressions: Vec<Atoms<'a>>) -> ExpressionParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|&v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	expressions.push(Atoms::Member{ name: name.clone() });

	let lexers = vec![
		parse_operator,
		parse_accessor,
	];

	try_execute_expression_parsers(&lexers, iterator.clone(), expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

fn parse_accessor<'a>(mut iterator: std::slice::Iter<'a, &'a str>, mut expressions: Vec<Atoms<'a>>,) -> ExpressionParserResult<'a> {
	let _ = iterator.next().and_then(|&v| if v == "." { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	expressions.push(Atoms::Accessor);

	let lexers: Vec<ExpressionParser> = vec![
		parse_variable,
	];

	execute_expression_parsers(&lexers, iterator, expressions)
}

fn parse_literal<'a>(mut iterator: std::slice::Iter<'a, &'a str>, mut expressions: Vec<Atoms<'a>>,) -> ExpressionParserResult<'a> {
	let value = iterator.next().and_then(|&v| if v == "0" || v == "2.0" || v == "1.0" || v == "0.0" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?; // TODO: do real literal parsing

	expressions.push(Atoms::Literal{ value: value.clone() });

	Ok((expressions, iterator))
}

fn parse_rvalue<'a>(iterator: std::slice::Iter<'a, &'a str>, expressions: Vec<Atoms<'a>>,) -> ExpressionParserResult<'a> {
	let parsers = vec![
		parse_function_call,
		parse_literal,
		parse_variable,
	];

	execute_expression_parsers(&parsers, iterator.clone(), expressions)
}

fn parse_operator<'a>(mut iterator: std::slice::Iter<'a, &'a str>, mut expressions: Vec<Atoms<'a>>,) -> ExpressionParserResult<'a> {
	let operator = iterator.next().and_then(|&v| if v == "*" || v == "+" || v == "-" || v == "/" || v == "=" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	expressions.push(Atoms::Operator{ name: operator.clone() });

	let possible_following_expressions: Vec<ExpressionParser> = vec![
		parse_rvalue,
	];

	execute_expression_parsers(&possible_following_expressions, iterator, expressions)
}

fn parse_function_call<'a>(mut iterator: std::slice::Iter<'a, &'a str>, mut expressions: Vec<Atoms<'a>>) -> ExpressionParserResult<'a> {
	let function_name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|&v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let mut parameters = vec![];

	loop {
		if let Some(a) = try_execute_expression_parsers(&[parse_rvalue], iterator.clone(), Vec::new()) {
			let (expressions, new_iterator) = a?;
			parameters.push(expressions);
			iterator = new_iterator;
		}

		// Check if iter is comma
		if iterator.clone().peekable().peek().ok_or(ParsingFailReasons::StreamEndedPrematurely)? == "," { iterator.next(); }

		// check if iter is close brace
		if iterator.clone().peekable().peek().ok_or(ParsingFailReasons::StreamEndedPrematurely)? == ")" { iterator.next(); break; }
	}

	expressions.push(Atoms::FunctionCall{ name: function_name.clone(), parameters });

	let possible_following_expressions = vec![
		parse_operator,
		parse_accessor,
	];

	try_execute_expression_parsers(&possible_following_expressions, iterator.clone(), expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

fn parse_statement<'a>(iterator: std::slice::Iter<'a, &'a str>,) -> FeatureParserResult<'a> {
	let parsers = vec![
		parse_keywords,
		parse_var_decl,
		parse_function_call,
		parse_variable,
	];

	let (expressions, mut iterator) = execute_expression_parsers(&parsers, iterator, Vec::new())?;

	iterator.next().and_then(|&v| if v == ";" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected semicolon") })?; // Skip semicolon

	fn dandc<'a>(atoms: &[Atoms<'a>]) -> Node<'a> {
		let max_precedence_item = atoms.iter().enumerate().max_by_key(|(_, v)| v.precedence());

		if let Some((i, e)) = max_precedence_item {
			match e {
				Atoms::Keyword => { Node { node: Nodes::Expression(Expressions::Return) } }
				Atoms::Operator { name } => {
					let left = dandc(&atoms[..i]);
					let right = dandc(&atoms[i + 1..]);

					Node {
						node: Nodes::Expression(Expressions::Operator{ name: name.clone(), left, right }),
					}
				}
				Atoms::Accessor => {
					let left = dandc(&atoms[..i]);
					let right = dandc(&atoms[i + 1..]);

					Node {
						node: Nodes::Expression(Expressions::Accessor{ left, right }),
					}
				}
				Atoms::FunctionCall { name, parameters } => {
					let parameters = parameters.iter().map(|v| dandc(v)).collect::<Vec<_>>();

					Node { node: Nodes::Expression(Expressions::Call { name: name.clone(), parameters }), }
				}
				Atoms::Literal { value } => { Node { node: Nodes::Expression(Expressions::Literal { value: value.clone() },) } }
				Atoms::Member { name } => { Node { node: Nodes::Expression(Expressions::Member { name: name.clone() },) } }
				Atoms::VariableDeclaration { name, r#type } => { Node { node: Nodes::Expression(Expressions::VariableDeclaration { name: name.clone(), r#type: r#type.clone() },) } }
			}.into()
		} else {
			panic!("No max precedence item");
		}
	}

	Ok((dandc(&expressions), iterator))
}

fn parse_function<'a>(mut iterator: std::slice::Iter<'a, &'a str>,) -> FeatureParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	iterator.next().and_then(|&v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|&v| if v == "fn" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|&v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|&v| if v == ")" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|&v| if v == "->" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let return_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected a return type for function {} declaration.", name) })?;

	iterator.next().and_then(|&v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected a {{ after function {} declaration.", name) })?;

	let mut statements = vec![];

	loop {
		if let Some(Ok((expression, new_iterator))) = try_execute_parsers(&[parse_statement], iterator.clone(),) {
			iterator = new_iterator;

			statements.push(expression);
		} else {
			if iterator.clone().peekable().peek().unwrap() == "}" {
				iterator.next();
				break;
			} else {
				return Err(ParsingFailReasons::BadSyntax{ message: format!("Expected a }} after function {} declaration.", name) });
			}
		}

		// check if iter is close brace
		if iterator.clone().peekable().peek().ok_or(ParsingFailReasons::BadSyntax { message: "Expected a '}' after function body".to_string() })? == "}" {
			iterator.next();
			break;
		}
	}

	let node = Node::function(name, vec![], return_type, statements);

	Ok((node, iterator))
}

use std::ops::{Index};

impl <'a> Index<&str> for Node<'a> {
    type Output = Node<'a>;

    fn index(&self, index: &str) -> &Self::Output {
		match &self.node {
			Nodes::Scope { children, .. } => {
				for child in children {
					match child.node {
						Nodes::Scope { name: child_name, children: _ } => { if child_name == index { return child; } }
						Nodes::Struct { name: child_name, fields: _ } => { if child_name == index { return child; } }
						Nodes::Member { name: child_name, r#type: _ } => { if child_name == index { return child; } }
						Nodes::Function { name: child_name, .. } => { if child_name == index { return child; } }
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
			_ => { panic!("Cannot search  in these"); }
		}

		panic!("Not found");
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
		if let Nodes::Struct { name, fields } = node.node {
			assert_eq!(name, "Light");
			assert_eq!(fields.len(), 2);

			let position = &fields[0];

			if let Nodes::Member { name, r#type } = position.node {
				assert_eq!(name, "position");
				assert_eq!(r#type, "vec3f");
			} else { panic!("Not a member"); }

			let color = &fields[1];

			if let Nodes::Member { name, r#type } = color.node {
				assert_eq!(name, "color");
				assert_eq!(r#type, "vec3f");
			} else { panic!("Not a member"); }
		} else { panic!("Not a struct"); }
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
		let node = parse(tokens).expect("Failed to parse");

		// program.types.get("Light").expect("Failed to get Light type");

		if let Nodes::Struct { name, .. } = node.node {
			assert_eq!(name, "root");
			assert_struct(&node["Light"]);
		}
	}

	fn assert_function(node: &Node) {
		if let Nodes::Function { name, params, return_type, statements, .. } = node.node {
			assert_eq!(name, "main");
			assert_eq!(params.len(), 0);
			assert_eq!(return_type, "void");
			assert_eq!(statements.len(), 2);

			let statement = &statements[0];

			if let Nodes::Expression(Expressions::Operator { name, left: var_decl, right: function_call }) = statement.node {
				assert_eq!(name, "=");

				if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) = var_decl.node {
					assert_eq!(name, "position");
					assert_eq!(r#type, "vec4f");
				} else { panic!("Not an variable declaration"); }

				if let Nodes::Expression(Expressions::Call { name, parameters, }) = function_call.node {
					assert_eq!(name, "vec4");
					assert_eq!(parameters.len(), 4);

					let x_param = &parameters[0];

					if let Nodes::Expression(Expressions::Literal { value }) = x_param.node {
						assert_eq!(value, "0.0");
					} else { panic!("Not a literal"); }
				} else { panic!("Not a function call"); }
			} else { panic!("Not an assignment");}
		} else { panic!("Not a function"); }
	}

	#[test]
	fn test_parse_function() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope{ name, .. } = node.node {
			assert_eq!(name, "root");
			assert_function(&node["main"]);
		} else { panic!("Not root node") }
	}


	#[test]
	fn parse_operators() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4(0.0, 0.0, 0.0, 1.0) * 2.0;
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let node = parse(tokens).expect("Failed to parse");

		let main_node = &node["main"];

		if let Nodes::Function { name, statements, return_type, params, .. } = main_node.node {
			assert_eq!(name, "main");
			assert_eq!(statements.len(), 2);
			assert_eq!(return_type, "void");
			assert_eq!(params.len(), 0);

			assert_eq!(statements.len(), 2);

			let statement0 = &statements[0];

			if let Nodes::Expression(Expressions::Operator { name, left: var_decl, right: multiply }) = statement0.node {
				assert_eq!(name, "=");

				if let Nodes::Expression(Expressions::VariableDeclaration { .. }) = var_decl.node {
				} else { panic!("Not a variable declaration"); }

				if let Nodes::Expression(Expressions::Operator { name, left: vec4, right: literal }) = multiply.node {
					assert_eq!(name, "*");

					if let Nodes::Expression(Expressions::Call { name, .. }) = vec4.node {
						assert_eq!(name, "vec4");
					} else { panic!("Not a function call"); }

					if let Nodes::Expression(Expressions::Literal { value, }) = literal.node {
						assert_eq!(value, "2.0");
					} else { panic!("Not a literal"); }
				} else { panic!("Not an operator"); }
			} else { panic!("Not an expression"); }
		} else { panic!("Not a feature"); }
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
		let node = parse(tokens).expect("Failed to parse");

		print_tree(&node);

		if let Nodes::Scope{ children, .. } = node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, statements, .. } = main_node.node {
				assert_eq!(name, "main");
				assert_eq!(statements.len(), 3);

				let statement1 = &statements[1];

				if let Nodes::Expression(Expressions::Operator { name, left: accessor, right: literal }) = statement1.node {
					assert_eq!(name, "=");

					if let Nodes::Expression(Expressions::Accessor{ left: position, right: y }) = accessor.node {
						if let Nodes::Expression(Expressions::Member { name }) = position.node {
							assert_eq!(name, "position");
						} else { panic!("Not a member"); }

						if let Nodes::Expression(Expressions::Member { name }) = y.node {
							assert_eq!(name, "y");
						} else { panic!("Not a member"); }
					} else { panic!("Not an accessor"); }

					if let Nodes::Expression(Expressions::Literal { value, }) = literal.node {
						assert_eq!(value, "2.0");
					} else { panic!("Not a literal"); }
				} else { panic!("Not an operator"); }
			} else { panic!("Not a function"); }
		} else { panic!("Not root node") }
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
		let node = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { .. } = &node.node {
			assert_struct(&node["Light"]);
			assert_function(&node["main"]);
		} else { panic!("Not root node") }
	}

	#[test]
	fn test_parse_member() {
		let source = "color: In<vec4f>;";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let node = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { .. } = &node.node {
			let member_node = &node["color"];

			if let Nodes::Member{ name, r#type } = member_node.node {
				assert_eq!(name, "color");
				assert_eq!(r#type, "In<vec4f>");
			} else { panic!("Not a feature"); }
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
		let node = parse(tokens).expect("Failed to parse");

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
		let node = parse(tokens).expect("Failed to parse");

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
		let node = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, statements, .. } = main_node.node {
				assert_eq!(name, "main");
				assert_eq!(statements.len(), 1);

				let statement = &statements[0];

				if let Nodes::Expression(Expressions::Operator { name, left, right }) = statement.node {
					assert_eq!(name, "=");

					if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) = left.node {
						assert_eq!(name, "n");
						assert_eq!(r#type, "f32");
					} else { panic!("Not a variable declaration"); }

					if let Nodes::Expression(Expressions::Accessor { left, right }) = right.node {
						if let Nodes::Expression(Expressions::Call { name, parameters }) = left.node {
							assert_eq!(name, "intrinsic");
							assert_eq!(parameters.len(), 1);

							if let Nodes::Expression(Expressions::Literal { value }) = parameters[0].node {
								assert_eq!(value, "0");
							} else { panic!("Not a literal"); }
						} else { panic!("Not a function call"); }

						if let Nodes::Expression(Expressions::Member { name }) = right.node {
							assert_eq!(name, "y");
						} else { panic!("Not a member"); }
					} else { panic!("Not an accessor"); }
				} else { panic!("Not an operator"); }
			} else { panic!("Not a function"); }
		} else { panic!("Not root node") }
	}
}
