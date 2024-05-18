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

/// Parse consumes an stream of tokens and return a JSPD describing the shader.
pub(super) fn parse(tokens: Vec<String>) -> Result<(Node, ProgramState), ParsingFailReasons> {
	let mut program_state = ProgramState::new();

	let mut iterator = tokens.iter();

	let parsers = [
		parse_struct,
		parse_function,
		parse_macro,
		parse_member,
	];

	let mut children = vec![];

	loop {
		let ((expression, program), iter) = execute_parsers(parsers.as_slice(), iterator, &program_state)?;

		program_state = program; // Update program state

		children.push(expression);

		iterator = iter;

		if iterator.len() == 0 {
			break;
		}
	}

	Ok((make_scope("root", children), program_state))
}

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Node {
	pub(crate) node: Nodes,
}

impl Node {
	pub fn node_mut(&mut self) -> &mut Nodes {
		&mut self.node
	}

	pub fn root() -> Node {
		Node {
			node: Nodes::Scope {
				name: "root".to_string(),
				children: vec![],
			},
		}
	}

	pub fn root_with_children(children: Vec<Node>) -> Node {
		Node {
			node: Nodes::Scope {
				name: "root".to_string(),
				children,
			},
		}
	}

	pub fn scope(name: &str, children: Vec<Node>) -> Node {
		make_scope(name, children)
	}

	pub fn r#struct(name: &str, fields: Vec<Node>) -> Node {
		make_struct(name, fields)
	}

	pub fn member(name: &str, r#type: &str) -> Node {
		make_member(name, r#type)
	}

	pub fn member_expression(name: &str) -> Node {
		Node {
			node: Nodes::Expression(Expressions::Member { name: name.to_string() }),
		}
	}

	pub fn function(name: &str, params: Vec<Node>, return_type: &str, statements: Vec<Node>) -> Node {
		make_function(name, params, return_type, statements)
	}

	pub fn binding(name: &str, r#type: Node, set: u32, descriptor: u32, read: bool, write: bool) -> Node {
		Node {
			node: Nodes::Binding {
				name: name.to_string(),
				r#type: r#type.into(),
				set,
				descriptor,
				read,
				write,
				count: None,
			},
		}
	}

	pub fn binding_array(name: &str, r#type: Node, set: u32, descriptor: u32, read: bool, write: bool, count: u32) -> Node {
		Node {
			node: Nodes::Binding {
				name: name.to_string(),
				r#type: r#type.into(),
				set,
				descriptor,
				read,
				write,
				count: NonZeroUsize::new(count as usize),
			},
		}
	}

	pub fn specialization(name: &str, r#type: &str) -> Node {
		Node {
			node: Nodes::Specialization {
				name: name.to_string(),
				r#type: r#type.to_string(),
			},
		}
	}

	pub fn buffer(name: &str, members: Vec<Node>) -> Node {
		Node {
			node: Nodes::Type {
				name: name.to_string(),
				members,
			},
		}
	}

	pub fn image(format: &str) -> Node {
		Node {
			node: Nodes::Image {
				format: format.to_string(),
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

	pub fn combined_image_sampler() -> Node {
		Node {
			node: Nodes::CombinedImageSampler {
				format: "".to_string(),
			},
		}
	}

	pub fn r#macro(name: &str, body: Node) -> Node {
		Node {
			node: Nodes::Expression(Expressions::Macro {
				name: name.to_string(),
				body: body.into(),
			}),
		}
	}

	pub fn sentence(expressions: Vec<Node>) -> Node {
		Node {
			node: Nodes::Expression(Expressions::Expression(expressions)),
		}
	}

	pub fn glsl(code: &str, input: Vec<String>, output: Vec<String>) -> Node {
		Node {
			node: Nodes::GLSL {
				code: code.to_string(),
				input,
				output,
			},
		}
	}

	pub fn literal(name: &str, body: Node) -> Node {
		Node {
			node: Nodes::Literal {
				name: name.to_string(),
				body: body.into(),
			},
		}
	}

	pub fn intrinsic(name: &str, parameters: Node, body: Node, r#return: &str) -> Node {
		Node {
			node: Nodes::Intrinsic {
				name: name.to_string(),
				elements: vec![parameters, body],
				r#return: r#return.to_string(),
			},
		}
	}

	pub fn null() -> Node {
		Node {
			node: Nodes::Null,
		}
	}

	pub fn parameter(name: &str, r#type: &str) -> Node {
		Node {
			node: Nodes::Parameter {
				name: name.to_string(),
				r#type: r#type.to_string(),
			},
		}
	}

	pub fn add(&mut self, children: Vec<Node>) {
		match &mut self.node {
			Nodes::Scope { children: c, .. } => {
				// Extend from the beginning of the vector
				c.extend(children);
			},
			_ => { println!("Tried to add children to a non-scope node."); },
		}
	}

	pub fn get_mut(&mut self, name: &str) -> Option<&mut Node> {
		match &mut self.node {
			Nodes::Scope { children, .. } => {
				children.iter_mut().find(|n| n.name() == Some(name))
			},
			_ => None,
		}
	}

	pub fn name(&self) -> Option<&str> {
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
			Nodes::GLSL { .. } => None,
			Nodes::Intrinsic { name, .. } => Some(name),
			Nodes::Literal { name, .. } => Some(name),
			Nodes::Parameter { name, .. } => Some(name),
			Nodes::PushConstant { .. } => None,
			Nodes::Null => None,
		}
	}
}

#[derive(Clone, Debug)]
pub enum Nodes {
	Null,
	Scope {
		name: String,
		children: Vec<Node>,
	},
	Struct {
		name: String,
		fields: Vec<Node>
	},
	Member {
		name: String,
		r#type: String,
	},
	Function {
		name: String,
		params: Vec<Node>,
		return_type: String,
		statements: Vec<Node>,
	},
	Binding {
		name: String,
		r#type: Box<Node>,
		set: u32,
		descriptor: u32,
		read: bool,
		write: bool,
		count: Option<NonZeroUsize>,
	},
	Specialization {
		name: String,
		r#type: String,
	},
	PushConstant {
		members: Vec<Node>,
	},
	Type {
		name: String,
		members: Vec<Node>,
	},
	Image {
		format: String,
	},
	CombinedImageSampler {
		format: String,
	},
	Expression(Expressions),
	GLSL {
		code: String,
		input: Vec<String>,
		output: Vec<String>,
	},
	Intrinsic {
		name: String,
		elements: Vec<Node>,
		r#return: String,
	},
	Literal {
		name: String,
		body: Box<Node>,
	},
	Parameter {
		name: String,
		r#type: String,
	},
}

#[derive(Clone, Debug)]
pub enum Expressions {
	Expression(Vec<Node>),
	Accessor{ left: Box<Node>, right: Box<Node>, },
	Member{ name: String },
	Literal{ value: String, },
	Call{ name: String, parameters: Vec<Node> },
	Operator{ name: String, left: Box<Node>, right: Box<Node>, },
	VariableDeclaration{ name: String, r#type: String, },
	GLSL{ code: String, input: Vec<String>, output: Vec<String>, },
	Macro{ name: String, body: Box<Node> },
	Return,
}

#[derive(Clone, Debug)]
pub(super) enum Atoms {
	Keyword,
	Accessor,
	Member{ name: String },
	Literal{ value: String, },
	FunctionCall{ name: String, parameters: Vec<Vec<Atoms>> },
	Operator{ name: String, },
	VariableDeclaration{ name: String, r#type: String, },
}

#[derive(Clone, Debug)]
enum BindingTypes {
	Buffer {
		name: String,
		members: Vec<Node>,
	},
	Image {
		format: String,
	},
	CombinedImageSampler {
		format: String,
	},
}

#[derive(Debug)]
pub(super) enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax{ message: String, },
	StreamEndedPrematurely,
}

fn make_scope(name:&str, children: Vec<Node>) -> Node {
	Node {
		node: Nodes::Scope {
			name: name.to_string(),
			children,
		},
	}
}

fn make_member(name: &str, r#type: &str) -> Node {
	Node {
		node: Nodes::Member {
			name: name.to_string(),
			r#type: r#type.to_string(),
		},
	}
}

fn make_no_member_struct(name: &str) -> Node {
	make_struct(name, vec![])
}

fn make_struct(name: &str, children: Vec<Node>) -> Node {
	Node {
		node: Nodes::Struct {
			name: name.to_string(),
			fields: children.clone(),
		},
	}
}

fn make_function(name: &str, params: Vec<Node>, return_type: &str, statements: Vec<Node>,) -> Node {
	Node {
		node: Nodes::Function {
			name: name.to_string(),
			params,
			return_type: return_type.to_string(),
			statements,
		},
	}
}

trait Precedence {
	fn precedence(&self) -> u8;
}

impl Precedence for Atoms {
	fn precedence(&self) -> u8 {
		match self {
			Atoms::Keyword => 0,
			Atoms::Accessor => 4,
			Atoms::Member{ .. } => 0,
			Atoms::Literal{ .. } => 0,
			Atoms::FunctionCall{ .. } => 0,
			Atoms::Operator{ name } => {
				match name.as_str() {
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
type FeatureParserResult<'a> = Result<((Node, ProgramState), std::slice::Iter<'a, String>), ParsingFailReasons>;

/// A parser is a function that tries to parse a sequence of tokens.
type FeatureParser<'a> = fn(std::slice::Iter<'a, String>, &ProgramState) -> FeatureParserResult<'a>;

type ExpressionParserResult<'a> = Result<(Vec<Atoms>, std::slice::Iter<'a, String>), ParsingFailReasons>;
type ExpressionParser<'a> = fn(std::slice::Iter<'a, String>, &ProgramState, Vec<Atoms>) -> ExpressionParserResult<'a>;

/// Execute a list of parsers on a stream of tokens.
fn execute_parsers<'a>(parsers: &[FeatureParser<'a>], mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> FeatureParserResult<'a> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), program) {
			return Ok(r);
		}
	}

	Err(ParsingFailReasons::BadSyntax{ message: format!("Tried several parsers none could handle the syntax for statement: {}", iterator.next().unwrap()) }) // No parser could handle this syntax.
}

/// Tries to execute a list of parsers on a stream of tokens. But it's ok if none of them can handle the syntax.
fn try_execute_parsers<'a>(parsers: &[FeatureParser<'a>], iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> Option<FeatureParserResult<'a>> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), program) {
			return Some(Ok(r));
		}
	}

	None
}

/// Execute a list of parsers on a stream of tokens.
fn execute_expression_parsers<'a>(parsers: &[ExpressionParser<'a>], mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, expressions: Vec<Atoms>) -> ExpressionParserResult<'a> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), program, expressions.clone()) {
			return Ok(r);
		}
	}

	Err(ParsingFailReasons::BadSyntax{ message: format!("Tried several parsers none could handle the syntax for statement: {}", iterator.next().unwrap()) }) // No parser could handle this syntax.
}

/// Tries to execute a list of parsers on a stream of tokens. But it's ok if none of them can handle the syntax.
fn try_execute_expression_parsers<'a>(parsers: &[ExpressionParser<'a>], iterator: std::slice::Iter<'a, String>, program: &ProgramState, expressions: Vec<Atoms>,) -> Option<ExpressionParserResult<'a>> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), program, expressions.clone()) {
			return Some(Ok(r));
		}
	}

	None
}

fn is_identifier(c: char) -> bool { // TODO: validate number at end of identifier
	c.is_alphanumeric() || c == '_'
}

fn parse_member<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> FeatureParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	let mut r#type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find type while parsing member {}.", name) })?.clone();

	if let Some(n) = iterator.clone().peekable().peek() {
		if n.as_str() == "<"	{
			iterator.next();
			r#type.push_str("<");
			let next = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find type while parsing generic argument for member {}", name) })?;
			r#type.push_str(next);
			iterator.next();
			r#type.push_str(">");
		}
	}

	let node = Node::member(name, &r#type);

	iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected semicolon") })?; // Skip semicolon

	Ok(((node, program.clone()), iterator))
}

fn parse_macro<'a>(iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> FeatureParserResult<'a> {
	let mut iter = iterator;

	iter.next().and_then(|v| if v == "#" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().and_then(|v| if v == "[" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find macro name.") })?;
	iter.next().and_then(|v| if v == "]" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find ] after macro.") })?;

	Ok(((make_scope("MACRO", vec![]).into(), program.clone()), iter))
}

fn parse_struct<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> FeatureParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(char::is_alphanumeric) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "struct" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find {{ after struct {} declaration", name) })?;

	let mut fields = vec![];

	while let Some(v) = iterator.next() {
		if v == "}" {
			break;
		} else if v == "," {
			continue;
		}

		let colon = iterator.next().unwrap();

		if colon != ":" { return Err(ParsingFailReasons::BadSyntax{ message: format!("Expected to find : after name for member {} in struct {}", v, name) }); }

		let type_name = iterator.next().unwrap();

		if !type_name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::BadSyntax{ message: format!("Expected to find a type name after : for member {} in struct {}", v, name) }); }

		// See if is array type
		let type_name = if iterator.clone().peekable().peek().unwrap().as_str() == "[" {
			iterator.next();
			let count = iterator.next().and_then(|v| v.parse::<u32>().ok()).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find a number after [ for member {} in struct {}", v, name) })?;
			iterator.next().unwrap();
			format!("{}[{}]", type_name, count)
		} else {
			type_name.clone()
		};

		fields.push(make_member(v, &type_name).into());
	}

	let node = Node::r#struct(name, fields);

	let mut program = program.clone();

	// program.types.insert(name.clone(), node.clone());

	Ok(((node, program.clone()), iterator))
}

fn parse_var_decl<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let _ = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v == "let" { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	let variable_name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	let variable_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find a type for variable {}", variable_name) }).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	expressions.push(Atoms::VariableDeclaration{ name: variable_name.clone(), r#type: variable_type.clone() });

	let possible_following_expressions: Vec<ExpressionParser> = vec![
		parse_operator,
	];

	let expressions = execute_expression_parsers(&possible_following_expressions, iterator, program, expressions)?;

	Ok(expressions)
}

fn parse_keywords<'a>(mut iterator: std::slice::Iter<'a, String>, _: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v == "return" { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	expressions.push(Atoms::Keyword);

	// let lexers = vec![
	// 	parse_operator,
	// 	parse_accessor,
	// ];

	// try_execute_expression_parsers(&lexers, iterator.clone(), program, expressions.clone()).unwrap_or(Ok((expressions, iterator)));

	Ok((expressions, iterator))
}

fn parse_variable<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	expressions.push(Atoms::Member{ name: name.clone() });

	let lexers = vec![
		parse_operator,
		parse_accessor,
	];

	try_execute_expression_parsers(&lexers, iterator.clone(), program, expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

fn parse_accessor<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let _ = iterator.next().and_then(|v| if v == "." { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	expressions.push(Atoms::Accessor);

	let lexers: Vec<ExpressionParser> = vec![
		parse_variable,
	];

	execute_expression_parsers(&lexers, iterator, program, expressions)
}

fn parse_literal<'a>(mut iterator: std::slice::Iter<'a, String>, _: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let value = iterator.next().and_then(|v| if v == "0" || v == "2.0" || v == "1.0" || v == "0.0" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?; // TODO: do real literal parsing

	expressions.push(Atoms::Literal{ value: value.clone() });

	Ok((expressions, iterator))
}

fn parse_rvalue<'a>(iterator: std::slice::Iter<'a, String>, program: &ProgramState, expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let parsers = vec![
		parse_function_call,
		parse_literal,
		parse_variable,
	];

	execute_expression_parsers(&parsers, iterator.clone(), program, expressions)
}

fn parse_operator<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let operator = iterator.next().and_then(|v| if v == "*" || v == "+" || v == "-" || v == "/" || v == "=" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	expressions.push(Atoms::Operator{ name: operator.clone() });

	let possible_following_expressions: Vec<ExpressionParser> = vec![
		parse_rvalue,
	];

	execute_expression_parsers(&possible_following_expressions, iterator, program, expressions)
}

fn parse_function_call<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let function_name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let mut parameters = vec![];

	loop {
		if let Some(a) = try_execute_expression_parsers(&[parse_rvalue], iterator.clone(), program, Vec::new()) {
			let (expressions, new_iterator) = a?;
			parameters.push(expressions);
			iterator = new_iterator;
		}

		// Check if iter is comma
		if iterator.clone().peekable().peek().ok_or(ParsingFailReasons::StreamEndedPrematurely)?.as_str() == "," { iterator.next(); }

		// check if iter is close brace
		if iterator.clone().peekable().peek().ok_or(ParsingFailReasons::StreamEndedPrematurely)?.as_str() == ")" { iterator.next(); break; }
	}

	expressions.push(Atoms::FunctionCall{ name: function_name.clone(), parameters });

	let possible_following_expressions = vec![
		parse_operator,
		parse_accessor,
	];

	try_execute_expression_parsers(&possible_following_expressions, iterator.clone(), program, expressions.clone()).unwrap_or(Ok((expressions, iterator)))
}

fn parse_statement<'a>(iterator: std::slice::Iter<'a, String>, program: &ProgramState,) -> FeatureParserResult<'a> {
	let parsers = vec![
		parse_keywords,
		parse_var_decl,
		parse_function_call,
		parse_variable,
	];

	let (expressions, mut iterator) = execute_expression_parsers(&parsers, iterator, program, Vec::new())?;

	iterator.next().and_then(|v| if v == ";" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected semicolon") })?; // Skip semicolon

	fn dandc<'a>(atoms: &[Atoms]) -> Node {
		let max_precedence_item = atoms.iter().enumerate().max_by_key(|(_, v)| v.precedence());

		if let Some((i, e)) = max_precedence_item {
			match e {
				Atoms::Keyword => { Node { node: Nodes::Expression(Expressions::Return) } }
				Atoms::Operator { name } => {		
					let left = dandc(&atoms[..i]);
					let right = dandc(&atoms[i + 1..]);
		
					Node {
						node: Nodes::Expression(Expressions::Operator{ name: name.clone(), left: left.into(), right: right.into() }),
					}
				}
				Atoms::Accessor => {
					let left = dandc(&atoms[..i]);
					let right = dandc(&atoms[i + 1..]);

					Node {
						node: Nodes::Expression(Expressions::Accessor{ left: left.into(), right: right.into() },),
					}
				}
				Atoms::FunctionCall { name, parameters } => {
					let parameters = parameters.iter().map(|v| dandc(v)).collect::<Vec<_>>();

					Node { node: Nodes::Expression(Expressions::Call { name: name.clone(), parameters },), }
				}
				Atoms::Literal { value } => { Node { node: Nodes::Expression(Expressions::Literal { value: value.clone() },) } }
				Atoms::Member { name } => { Node { node: Nodes::Expression(Expressions::Member { name: name.clone() },) } }
				Atoms::VariableDeclaration { name, r#type } => { Node { node: Nodes::Expression(Expressions::VariableDeclaration { name: name.clone(), r#type: r#type.clone() },) } }
			}.into()
		} else {
			panic!("No max precedence item");
		}
	}

	Ok(((dandc(&expressions), program.clone()), iterator))
}

fn parse_function<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> FeatureParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(is_identifier) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "fn" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ")" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "->" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let return_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected a return type for function {} declaration.", name) })?;

	iterator.next().and_then(|v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected a {{ after function {} declaration.", name) })?;

	let mut statements = vec![];

	loop {
		if let Some(Ok(((expression, _), new_iterator))) = try_execute_parsers(&[parse_statement], iterator.clone(), program) {
			iterator = new_iterator;
	
			statements.push(expression);
		} else {
			if iterator.clone().peekable().peek().unwrap().as_str() == "}" {
				iterator.next();
				break;
			} else {
				return Err(ParsingFailReasons::BadSyntax{ message: format!("Expected a }} after function {} declaration.", name) });
			}
		}

		// check if iter is close brace
		if iterator.clone().peekable().peek().ok_or(ParsingFailReasons::BadSyntax { message: "Expected a '}' after function body".to_string() })?.as_str() == "}" {
			iterator.next();
			break;
		}
	}

	let mut program = program.clone();

	let node = Node::function(name, vec![], return_type, statements);

	Ok(((node, program.clone()), iterator))
}

use std::ops::Index;

impl Index<&str> for Node {
    type Output = Node;

    fn index(&self, index: &str) -> &Self::Output {
		match &self.node {
			Nodes::Scope { children, .. } => {
				for child in children {
					match &child.node {
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
					if let Nodes::Member { name: child_name, .. } = &field.node {
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

impl ProgramState {
	pub fn new() -> Self {
		let mut types = HashMap::new();

		let void = Node::r#struct("void", Vec::new());
		let u8 = Node::r#struct("u8", Vec::new());
		let u16 = Node::r#struct("u16", Vec::new());
		let u32 = Node::r#struct("u32", Vec::new());
		let f32 = Node::r#struct("f32", Vec::new());
		let in_type = Node::r#struct("In", Vec::new()); // Input type
		let out_type = Node::r#struct("Out", Vec::new()); // Output type
		let push_constant_type = Node::r#struct("PushConstant", Vec::new()); // Output type
		let vec2f = Node::r#struct("vec2f", vec![Node::member("x", "f32"), Node::member("y", "f32")]);
		let vec2u16 = Node::r#struct("vec2u16", vec![Node::member("x", "u16"), Node::member("y", "u16")]);
		let vec3f = Node::r#struct("vec3f", vec![Node::member("x", "f32"), Node::member("y", "f32"), Node::member("z", "f32")]);
		let vec4f = Node::r#struct("vec4f", vec![Node::member("x", "f32"), Node::member("y", "f32"), Node::member("z", "f32"), Node::member("w", "f32")]);
		let mat4f = Node::r#struct("mat4f", vec![Node::member("x", "f32"), Node::member("y", "f32"), Node::member("z", "f32"), Node::member("w", "f32")]);
	
		types.insert("void".to_string(), void);
		types.insert("u8".to_string(), u8);
		types.insert("u16".to_string(), u16);
		types.insert("u32".to_string(), u32);
		types.insert("f32".to_string(), f32);
		types.insert("In".to_string(), in_type);
		types.insert("Out".to_string(), out_type);
		types.insert("PushConstant".to_string(), push_constant_type);
		types.insert("vec2f".to_string(), vec2f);
		types.insert("vec2u16".to_string(), vec2u16);
		types.insert("vec3f".to_string(), vec3f);
		types.insert("vec4f".to_string(), vec4f);
		types.insert("mat4f".to_string(), mat4f);

		Self {}
	}
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
			assert_eq!(name, "Light");
			assert_eq!(fields.len(), 2);

			let position = &fields[0];

			if let Nodes::Member { name, r#type } = &position.node {
				assert_eq!(name, "position");
				assert_eq!(r#type, "vec3f");
			} else { panic!("Not a member"); }

			let color = &fields[1];

			if let Nodes::Member { name, r#type } = &color.node {
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
		let (node, program) = parse(tokens).expect("Failed to parse");

		// program.types.get("Light").expect("Failed to get Light type");

		if let Nodes::Struct { name, .. } = &node.node {
			assert_eq!(name, "root");
			assert_struct(&node["Light"]);
		}
	}

	fn assert_function(node: &Node) {
		if let Nodes::Function { name, params, return_type, statements, .. } = &node.node {
			assert_eq!(name, "main");
			assert_eq!(params.len(), 0);
			assert_eq!(return_type, "void");
			assert_eq!(statements.len(), 2);

			let statement = &statements[0];

			if let Nodes::Expression(Expressions::Operator { name, left: var_decl, right: function_call }) = &statement.node {
				assert_eq!(name, "=");

				if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) = &var_decl.node {
					assert_eq!(name, "position");
					assert_eq!(r#type, "vec4f");
				} else { panic!("Not an variable declaration"); }

				if let Nodes::Expression(Expressions::Call { name, parameters, }) = &function_call.node {
					assert_eq!(name, "vec4");
					assert_eq!(parameters.len(), 4);

					let x_param = &parameters[0];

					if let Nodes::Expression(Expressions::Literal { value }) = &x_param.node {
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
		let (node, _program) = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope{ name, .. } = &node.node {
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
		let (node, _program) = parse(tokens).expect("Failed to parse");

		let main_node = &node["main"];

		if let Nodes::Function { name, statements, return_type, params, .. } = &main_node.node {
			assert_eq!(name, "main");
			assert_eq!(statements.len(), 2);
			assert_eq!(return_type, "void");
			assert_eq!(params.len(), 0);

			assert_eq!(statements.len(), 2);

			let statement0 = &statements[0];

			if let Nodes::Expression(Expressions::Operator { name, left: var_decl, right: multiply }) = &statement0.node {
				assert_eq!(name, "=");

				if let Nodes::Expression(Expressions::VariableDeclaration { .. }) = &var_decl.node {
				} else { panic!("Not a variable declaration"); }

				if let Nodes::Expression(Expressions::Operator { name, left: vec4, right: literal }) = &multiply.node {
					assert_eq!(name, "*");

					if let Nodes::Expression(Expressions::Call { name, .. }) = &vec4.node {
						assert_eq!(name, "vec4");
					} else { panic!("Not a function call"); }

					if let Nodes::Expression(Expressions::Literal { value, }) = &literal.node {
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
		let (node, _program) = parse(tokens).expect("Failed to parse");

		print_tree(&node);

		if let Nodes::Scope{ children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, statements, .. } = &main_node.node {
				assert_eq!(name, "main");
				assert_eq!(statements.len(), 3);

				let statement1 = &statements[1];

				if let Nodes::Expression(Expressions::Operator { name, left: accessor, right: literal }) = &statement1.node {
					assert_eq!(name, "=");

					if let Nodes::Expression(Expressions::Accessor{ left: position, right: y }) = &accessor.node {
						if let Nodes::Expression(Expressions::Member { name }) = &position.node {
							assert_eq!(name, "position");
						} else { panic!("Not a member"); }

						if let Nodes::Expression(Expressions::Member { name }) = &y.node {
							assert_eq!(name, "y");
						} else { panic!("Not a member"); }
					} else { panic!("Not an accessor"); }

					if let Nodes::Expression(Expressions::Literal { value, }) = &literal.node {
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
		let (node, _) = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { .. } = &node.node {
			assert_struct(&node["Light"]);
			assert_function(&node["main"]);
		} else { panic!("Not root node") }
	}

	#[test]
	fn test_parse_member() {
		let source = "color: In<vec4f>;";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let (node, _) = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { .. } = &node.node {
			let member_node = &node["color"];

			if let Nodes::Member{ name, r#type } = &member_node.node {
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
		let (node, _) = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = &node.node {
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
		let (node, _) = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = &node.node {
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
		let (node, _) = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { children, .. } = &node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, statements, .. } = &main_node.node {
				assert_eq!(name, "main");
				assert_eq!(statements.len(), 1);

				let statement = &statements[0];

				if let Nodes::Expression(Expressions::Operator { name, left, right }) = &statement.node {
					assert_eq!(name, "=");

					if let Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) = &left.node {
						assert_eq!(name, "n");
						assert_eq!(r#type, "f32");
					} else { panic!("Not a variable declaration"); }

					if let Nodes::Expression(Expressions::Accessor { left, right }) = &right.node {
						if let Nodes::Expression(Expressions::Call { name, parameters }) = &left.node {
							assert_eq!(name, "intrinsic");
							assert_eq!(parameters.len(), 1);

							if let Nodes::Expression(Expressions::Literal { value }) = &parameters[0].node {
								assert_eq!(value, "0");
							} else { panic!("Not a literal"); }
						} else { panic!("Not a function call"); }

						if let Nodes::Expression(Expressions::Member { name }) = &right.node {
							assert_eq!(name, "y");
						} else { panic!("Not a member"); }
					} else { panic!("Not an accessor"); }
				} else { panic!("Not an operator"); }
			} else { panic!("Not a function"); }
		} else { panic!("Not root node") }
	}
}