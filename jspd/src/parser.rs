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

pub(super) fn declare_intrinsic_types(program: &mut ProgramState) {
	let void = Rc::new(make_no_member_struct("void"));
	let u32 = Rc::new(make_no_member_struct("u32"));
	let f32 = Rc::new(make_no_member_struct("f32"));
	let in_type = Rc::new(make_no_member_struct("In")); // Input type
	let out_type = Rc::new(make_no_member_struct("Out")); // Output type
	let push_constant_type = Rc::new(make_no_member_struct("PushConstant")); // Output type
	let vec2f = Rc::new(make_struct("vec2f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32"))]));
	let vec3f = Rc::new(make_struct("vec3f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32")), Rc::new(make_member("z", "f32"))]));
	let vec4f = Rc::new(make_struct("vec4f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32")), Rc::new(make_member("z", "f32")), Rc::new(make_member("w", "f32"))]));
	let mat4f = Rc::new(make_struct("mat4f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32")), Rc::new(make_member("z", "f32")), Rc::new(make_member("w", "f32"))]));

	program.types.insert("void".to_string(), void);
	program.types.insert("u32".to_string(), u32);
	program.types.insert("f32".to_string(), f32);
	program.types.insert("In".to_string(), in_type);
	program.types.insert("Out".to_string(), out_type);
	program.types.insert("PushConstant".to_string(), push_constant_type);
	program.types.insert("vec2f".to_string(), vec2f);
	program.types.insert("vec3f".to_string(), vec3f);
	program.types.insert("vec4f".to_string(), vec4f);
	program.types.insert("mat4f".to_string(), mat4f);
}

/// Parse consumes an stream of tokens and return a JSPD describing the shader.
pub(super) fn parse(tokens: Vec<String>) -> Result<(Node, ProgramState), ParsingFailReasons> {
	let mut program_state = ProgramState {
		types: HashMap::new(),
	};

	declare_intrinsic_types(&mut program_state);

	let mut iterator = tokens.iter();

	let parsers = vec![
		parse_struct,
		parse_function,
		parse_macro,
		parse_member,
	];

	let mut children = vec![];

	loop {
		let ((expression, program), iter) = execute_parsers(&parsers, iterator, &program_state)?;

		program_state = program; // Update program state

		children.push(expression);

		iterator = iter;

		if iterator.len() == 0 {
			break;
		}
	}

	Ok((make_scope("root", children), program_state))
}

use std::{collections::HashMap, rc::Rc};

#[derive(Clone, Debug)]
pub(super) struct Node {
	pub(crate) node: Nodes,
}

#[derive(Clone, Debug)]
pub(super) enum Nodes {
	Scope {
		name: String,
		children: Vec<Rc<Node>>,
	},
	Struct {
		name: String,
		fields: Vec<Rc<Node>>
	},
	Member {
		name: String,
		r#type: String,
	},
	Function {
		name: String,
		params: Vec<Rc<Node>>,
		return_type: String,
		statements: Vec<Rc<Node>>,
		raw: Option<String>,
	},
	Expression(Expressions),
}

#[derive(Clone, Debug)]
pub(super) enum Atoms {
	Accessor,
	Member{ name: String },
	Literal{ value: String, },
	FunctionCall{ name: String, parameters: Vec<Vec<Atoms>> },
	Operator{ name: String, },
	VariableDeclaration{ name: String, r#type: String, },
}

#[derive(Clone, Debug)]
pub(super) enum Expressions {
	Accessor{ left: Rc<Node>, right: Rc<Node>, },
	Member{ name: String },
	Literal{ value: String, },
	FunctionCall{ name: String, parameters: Vec<Rc<Node>> },
	Operator{ name: String, left: Rc<Node>, right: Rc<Node>, },
	VariableDeclaration{ name: String, r#type: String, },
}

#[derive(Debug)]
pub(super) enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax{ message: String, },
	StreamEndedPrematurely,
}

fn make_scope(name:&str, children: Vec<Rc<Node>>) -> Node {
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

fn make_struct(name: &str, children: Vec<Rc::<Node>>) -> Node {
	Node {
		node: Nodes::Struct {
			name: name.to_string(),
			fields: children.clone(),
		},
	}
}

fn make_function(name: &str, params: Vec<Rc<Node>>, return_type: &str, statements: Vec<Rc<Node>>, raw: Option<String>) -> Node {
	Node {
		node: Nodes::Function {
			name: name.to_string(),
			params,
			return_type: return_type.to_string(),
			statements,
			raw,
		},
	}
}

trait Precedence {
	fn precedence(&self) -> u8;
}

impl Precedence for Atoms {
	fn precedence(&self) -> u8 {
		match self {
			Atoms::Accessor => 4,
			Atoms::Member{ .. } => 0,
			Atoms::Literal{ value } => 0,
			Atoms::FunctionCall{ name, parameters } => 0,
			Atoms::Operator{ name } => {
				match name.as_str() {
					"=" => 4,
					"+" => 3,
					"-" => 3,
					"*" => 2,
					"/" => 2,
					_ => 0,
				}
			},
			Atoms::VariableDeclaration{ name, r#type } => 0,
		}
	}
}

/// Type of the result of a parser.
type FeatureParserResult<'a> = Result<((Rc<Node>, ProgramState), std::slice::Iter<'a, String>), ParsingFailReasons>;

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

	let node = Rc::new(make_member(name, &r#type));

	iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected semicolon") })?; // Skip semicolon

	Ok(((node, program.clone()), iterator))
}

fn parse_macro<'a>(iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> FeatureParserResult<'a> {
	let mut iter = iterator;

	iter.next().and_then(|v| if v == "#" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().and_then(|v| if v == "[" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find macro name.") })?;
	iter.next().and_then(|v| if v == "]" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find ] after macro.") })?;

	Ok(((Rc::new(make_scope("MACRO", vec![])), program.clone()), iter))
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

		fields.push(Rc::new(make_member(v, type_name)));
	}

	let node = Rc::new(make_struct(name, fields));

	let mut program = program.clone();

	program.types.insert(name.clone(), node.clone());

	Ok(((node, program.clone()), iterator))
}

fn parse_var_decl<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let variable_name = iterator.next().ok_or(ParsingFailReasons::NotMine).and_then(|v| if v.chars().all(char::is_alphanumeric) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	let variable_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find a type for variable {}", variable_name) }).and_then(|v| if v.chars().all(char::is_alphanumeric) { Ok(v) } else { Err(ParsingFailReasons::NotMine) })?;

	expressions.push(Atoms::VariableDeclaration{ name: variable_name.clone(), r#type: variable_type.clone() });

	let possible_following_expressions: Vec<ExpressionParser> = vec![
		parse_operator,
	];

	let expressions = execute_expression_parsers(&possible_following_expressions, iterator, program, expressions)?;

	Ok(expressions)
}

fn parse_variable<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	name.chars().all(char::is_alphanumeric).then(|| ()).ok_or(ParsingFailReasons::NotMine)?;

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

fn parse_literal<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState, mut expressions: Vec<Atoms>,) -> ExpressionParserResult<'a> {
	let value = iterator.next().and_then(|v| if v == "2.0" || v == "1.0" || v == "0.0" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

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
		parse_var_decl,
		parse_function_call,
		parse_variable,
	];

	let (expressions, mut iterator) = execute_expression_parsers(&parsers, iterator, program, Vec::new())?;

	iterator.next().and_then(|v| if v == ";" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected semicolon") })?; // Skip semicolon

	fn dandc<'a>(atoms: &[Atoms]) -> Rc<Node> {
		let max_precedence_item = atoms.iter().enumerate().max_by_key(|(_, v)| v.precedence());

		if let Some((i, e)) = max_precedence_item {
			match e {
				Atoms::Operator { name } => {		
					let left = dandc(&atoms[..i]);
					let right = dandc(&atoms[i + 1..]);
		
					Rc::new(Node {
						node: Nodes::Expression(Expressions::Operator{ name: name.clone(), left, right }),
					})
				}
				Atoms::Accessor => {
					let left = dandc(&atoms[..i]);
					let right = dandc(&atoms[i + 1..]);

					Rc::new(Node {
						node: Nodes::Expression(Expressions::Accessor{ left, right },),
					})
				}
				Atoms::FunctionCall { name, parameters } => {
					let parameters = parameters.iter().map(|v| dandc(v)).collect::<Vec<Rc<Node>>>();

					Rc::new(Node { node: Nodes::Expression(Expressions::FunctionCall { name: name.clone(), parameters },), })
				}
				Atoms::Literal { value } => { Rc::new(Node { node: Nodes::Expression(Expressions::Literal { value: value.clone() },) },) }
				Atoms::Member { name } => { Rc::new(Node { node: Nodes::Expression(Expressions::Member { name: name.clone() },) },) }
				Atoms::VariableDeclaration { name, r#type } => { Rc::new(Node { node: Nodes::Expression(Expressions::VariableDeclaration { name: name.clone(), r#type: r#type.clone() },) },) }
			}
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
		}

		// check if iter is close brace
		if iterator.clone().peekable().peek().unwrap().as_str() == "}" {
			iterator.next();
			break;
		}
	}

	let mut program = program.clone();

	let node = Rc::new(make_function(name, vec![], return_type, statements, None));

	program.types.insert(name.clone(), node.clone());

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
						Nodes::Function { name: child_name, params: _, return_type: _, statements: _, raw: _ } => { if child_name == index { return child; } }
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
pub(super) struct ProgramState {
	pub(super) types: HashMap<String, Rc::<Node>>,
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
	position: vec3f,
	color: vec3f
}";

		let tokens = tokenize(source).unwrap();
		let (node, program) = parse(tokens).expect("Failed to parse");

		program.types.get("Light").expect("Failed to get Light type");

		if let Nodes::Struct { name, .. } = &node.node {
			assert_eq!(name, "root");
			assert_struct(&node["Light"]);
		}
	}

	fn assert_function(node: &Node) {
		if let Nodes::Function { name, params, return_type, statements, raw: _ } = &node.node {
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

				if let Nodes::Expression(Expressions::FunctionCall { name, parameters, }) = &function_call.node {
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
	position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
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
	position: vec4f = vec4(0.0, 0.0, 0.0, 1.0) * 2.0;
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

					if let Nodes::Expression(Expressions::FunctionCall { name, .. }) = &vec4.node {
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
	position: vec4f = vec4(0.0, 0.0, 0.0, 1.0) * 2.0;
	position.y = 2.0;
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let (node, _program) = parse(tokens).expect("Failed to parse");

		print_tree(&node);

		if let Nodes::Scope{ name, children } = &node.node {
			assert_eq!(children.len(), 1);

			let main_node = &node["main"];

			if let Nodes::Function { name, params, return_type, statements, raw } = &main_node.node {
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
	position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let (node, program) = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { name, children } = &node.node {
			assert_struct(&node["Light"]);
			assert_function(&node["main"]);			
		} else { panic!("Not root node") }
	}

	#[test]
	fn test_parse_member() {
		let source = "color: In<vec4f>;";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let (node, program) = parse(tokens).expect("Failed to parse");

		if let Nodes::Scope { name, children } = &node.node {
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
}