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
//! The parser consumes an stream of tokens and creates nodes with features.
//! All nodes which have cross references only do so by name.
//! Those relations are resolved later by the lexer which performs a grammar analysis.

/// Parse consumes an stream of tokens and return a JSPD describing the shader.
pub(super) fn parse(tokens: Vec<String>) -> Result<(Node, ProgramState), ParsingFailReasons> {
	let mut program_state = ProgramState {
		types: HashMap::new(),
	};

	let void = Rc::new(make_no_member_struct("void"));
	let f32 = Rc::new(make_no_member_struct("f32"));
	let in_type = Rc::new(make_no_member_struct("In")); // Input type
	let vec2f = Rc::new(make_struct("vec2f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32"))]));
	let vec3f = Rc::new(make_struct("vec3f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32")), Rc::new(make_member("z", "f32"))]));
	let vec4f = Rc::new(make_struct("vec4f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32")), Rc::new(make_member("z", "f32")), Rc::new(make_member("w", "f32"))]));

	program_state.types.insert("void".to_string(), void);
	program_state.types.insert("f32".to_string(), f32);
	program_state.types.insert("In".to_string(), in_type);
	program_state.types.insert("vec2f".to_string(), vec2f);
	program_state.types.insert("vec3f".to_string(), vec3f);
	program_state.types.insert("vec4f".to_string(), vec4f);

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

	return Ok((make_scope("root", children), program_state));
}

use std::borrow::BorrowMut;
use std::{collections::HashMap, rc::Rc};

#[derive(Clone, Debug)]
pub(super) struct Node {
	pub(crate) node: Nodes,
	pub(crate) children: Vec<Rc<Node>>,
}

#[derive(Clone, Debug)]
pub(super) enum Nodes {
	Feature {
		name: String,
		feature: Features,
	},
	Expression {
		expression: Expressions,
		children: Vec<Rc<Node>>,
	},
}

#[derive(Clone, Debug)]
pub(super) enum Features {
	Root,
	Scope,
	Struct {
		fields: Vec<Rc<Node>>
	},
	Member {
		r#type: String,
	},
	Function {
		params: Vec<Rc<Node>>,
		return_type: String,
		statements: Vec<Rc<Node>>,
		raw: Option<String>,
	},
}

#[derive(Clone, Debug)]
pub(super) enum Expressions {
	Accessor,
	Member{ name: String },
	Literal{ value: String, },
	FunctionCall{ name: String, },
	Operator{ name: String, },
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
		node: Nodes::Feature {
			name: name.to_string(),
			feature: Features::Scope,
		},
		children,
	}
}

fn make_member(name: &str, r#type: &str) -> Node {
	Node {
		node: Nodes::Feature {
			name: name.to_string(),
			feature: Features::Member {
				r#type: r#type.to_string(),
			}
		},
		children: vec![]
	}
}

fn make_no_member_struct(name: &str) -> Node {
	make_struct(name, vec![])
}

fn make_struct(name: &str, children: Vec<Rc::<Node>>) -> Node {
	Node {
		node: Nodes::Feature {
			name: name.to_string(),
			feature: Features::Struct {
				fields: children.clone(),
			}
		},
		children: children.clone(),
	}
}

fn make_function(name: &str, params: Vec<Rc<Node>>, return_type: &str, statements: Vec<Rc<Node>>, raw: Option<String>) -> Node {
	Node {
		node: Nodes::Feature {
			name: name.to_string(),
			feature: Features::Function {
				params,
				return_type: return_type.to_string(),
				statements,
				raw,
			}
		},
		children: vec![]
	}
}

trait Precedence {
	fn precedence(&self) -> u8;
}

impl Precedence for Expressions {
	fn precedence(&self) -> u8 {
		match self {
			Expressions::Accessor => 254,
			Expressions::Member{ name } => 253,
			Expressions::Literal{ value } => 253,
			Expressions::FunctionCall{ name } => 253,
			Expressions::Operator{ name } => 255,
			Expressions::VariableDeclaration{ name, r#type } => 0,
		}
	}
}

/// Type of the result of a parser.
type ParserResult<'a> = Result<((Rc<Node>, ProgramState), std::slice::Iter<'a, String>), ParsingFailReasons>;

/// A parser is a function that tries to parse a sequence of tokens.
type Parser<'a> = fn(std::slice::Iter<'a, String>, &ProgramState) -> ParserResult<'a>;

/// Execute a list of parsers on a stream of tokens.
fn execute_parsers<'a>(parsers: &[Parser<'a>], mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), program) {
			return Ok(r);
		}
	}

	return Err(ParsingFailReasons::BadSyntax{ message: format!("Tried several parsers none could handle the syntax for statement: {}", iterator.next().unwrap()) }); // No parser could handle this syntax.
}

/// Tries to execute a list of parsers on a stream of tokens. But it's ok if none of them can handle the syntax.
fn try_execute_parsers<'a>(parsers: &[Parser<'a>], iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> Option<ParserResult<'a>> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), program) {
			return Some(Ok(r));
		}
	}

	return None;
}

fn parse_member<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;
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

	return Ok(((node, program.clone()), iterator));
}

fn parse_macro<'a>(iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let mut iter = iterator;

	iter.next().and_then(|v| if v == "#" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().and_then(|v| if v == "[" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find macro name.") })?;
	iter.next().and_then(|v| if v == "]" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find ] after macro.") })?;

	return Ok(((Rc::new(make_scope("MACRO", vec![])), program.clone()), iter));
}

fn parse_struct<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;
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

	return Ok(((node, program.clone()), iterator));
}

/// Creates a new node from an expression and a following expression.
/// If the expression has a higher precedence than the following expression, the following expression is inserted as a child of the new node.
/// If the expression has a lower precedence than the following expression, the new node is inserted as a child of the following expression.
///
/// # Panics
///
/// Panics if .
fn make_expression_node(following_expression: Option<&Node>, new_expression: Expressions, new_node_children: Option<Vec<Rc<Node>>>) -> Rc::<Node> {
	if let Some(following_expression) = following_expression {
		if let Nodes::Expression { expression, children: _ } = &following_expression.node {
			if expression.precedence() > new_expression.precedence() {
				let mut cont = following_expression.clone();
				if let Nodes::Expression { expression: _, children } = &mut cont.node {
					children.insert(0, make_expression_node(None, new_expression, new_node_children.clone()));
					Rc::new(cont)
				} else {
					Rc::new(cont)
				}
			} else {
				let node = Node {
					node: Nodes::Expression {
						expression: new_expression,
						children: new_node_children.unwrap_or(vec![]),
					},
					children: vec![],
				};
		
				Rc::new(node)
			}
		} else { panic!("Not an expression"); }
	} else {
		Rc::new(Node {
			node: Nodes::Expression {
				expression: new_expression,
				children: new_node_children.unwrap_or(vec![]),
			},
			children: vec![],
		})
	}
}

fn parse_var_decl<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let variable_name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	let variable_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected to find a type for variable {}", variable_name) })?;

	let possible_following_expressions: Vec<Parser> = vec![
		parse_operator,
	];

	let ((expression, _), new_iterator) = execute_parsers(&possible_following_expressions, iterator.clone(), program)?;

	let expression = make_expression_node(Some(&expression), Expressions::VariableDeclaration{ name: variable_name.clone(), r#type: variable_type.clone()  }, None);

	return Ok(((expression, program.clone()), new_iterator));
}

fn parse_variable<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;

	let lexers = vec![
		parse_operator,
		parse_accessor,
	];

	if let Some(Ok(((expression, _), new_iterator))) = try_execute_parsers(&lexers, iterator.clone(), program) {
		return Ok(((make_expression_node(Some(&expression), Expressions::Member{ name: name.clone() }, None), program.clone()), new_iterator));
	} else {
		return Ok(((make_expression_node(None, Expressions::Member{ name: name.clone() }, None), program.clone()), iterator));
	}
}

fn parse_accessor<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let _ = iterator.next().and_then(|v| if v == "." { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let lexers: Vec<Parser> = vec![
		parse_variable,
	];

	let ((expression, _), new_iterator) = execute_parsers(&lexers, iterator.clone(), program)?;

	return Ok(((make_expression_node(Some(&expression), Expressions::Accessor, None), program.clone()), new_iterator));
}

fn parse_literal<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let value = iterator.next().and_then(|v| if v == "2.0" || v == "1.0" || v == "0.0" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	return Ok(((make_expression_node(None, Expressions::Literal{ value: value.clone() }, None), program.clone()), iterator));
}

fn parse_rvalue<'a>(iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let parsers = vec![
		parse_function_call,
		parse_literal,
		parse_variable,
	];

	return execute_parsers(&parsers, iterator.clone(), program);
}

fn parse_operator<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let operator = iterator.next().and_then(|v| if v == "*" || v == "+" || v == "-" || v == "/" || v == "=" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let possible_following_expressions: Vec<Parser> = vec![
		parse_rvalue,
	];

	let ((expression, _), new_iterator) = execute_parsers(&possible_following_expressions, iterator.clone(), program)?;

	let expression = make_expression_node(Some(&expression), Expressions::Operator { name: operator.clone() }, Some(vec![expression.clone()]));

	return Ok(((expression, program.clone()), new_iterator));
}

fn parse_function_call<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let function_name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let mut children = vec![];

	loop {
		let ((expression, _), new_iterator) = if let Ok(r) = parse_rvalue(iterator.clone(), program) { r } else { return Err(ParsingFailReasons::BadSyntax{ message: format!("Expected an rvalue in call to {}()", function_name) }); };

		children.push(expression);

		iterator = new_iterator;

		// Check if iter is comma
		if iterator.clone().peekable().peek().ok_or(ParsingFailReasons::StreamEndedPrematurely)?.as_str() == "," { iterator.next(); }

		// check if iter is close brace
		if iterator.clone().peekable().peek().ok_or(ParsingFailReasons::StreamEndedPrematurely)?.as_str() == ")" { iterator.next(); break; }
	}

	let valid_expressions = vec![
		parse_operator,
		parse_accessor,
	];

	let (expression, new_iterator) = if let Some(Ok(((expression, _), new_iterator))) = try_execute_parsers(&valid_expressions, iterator.clone(), program) {
		(make_expression_node(Some(&expression), Expressions::FunctionCall{ name: function_name.clone() }, Some(children)), new_iterator)
	} else {
		(make_expression_node(None, Expressions::FunctionCall{ name: function_name.clone() }, Some(children)), iterator)
	};

	return Ok(((expression, program.clone()), new_iterator));
}

fn parse_statement<'a>(iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let parsers = vec![
		parse_var_decl,
		parse_variable,
		parse_function_call,
	];

	let (lexeme, mut new_iterator) = if let Ok(r) = execute_parsers(&parsers, iterator.clone(), program) { r } else { return Err(ParsingFailReasons::BadSyntax{ message: format!("Found an unexpected feature in function statement") }); };

	new_iterator.next().and_then(|f| if f == ";" { Some(f) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected ; to end statement.") })?;

	return Ok((lexeme, new_iterator));
}

fn parse_function<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "fn" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ")" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "->" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let return_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected a return type for function {} declaration.", name) })?;

	iterator.next().and_then(|v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax{ message: format!("Expected a {{ after function {} declaration.", name) })?;

	let mut statements = vec![];

	loop {
		let ((expression, _), new_iterator) = if let Ok(r) = parse_statement(iterator.clone(), program) { r } else { return Err(ParsingFailReasons::BadSyntax{ message: format!("An statement for function {} has unexpected syntax.", name) }); };

		iterator = new_iterator;

		statements.push(expression);

		// check if iter is close brace
		if iterator.clone().peekable().peek().unwrap().as_str() == "}" {
			iterator.next();
			break;
		}
	}

	return Ok(((Rc::new(make_function(name, vec![], return_type, statements, None)), program.clone()), iterator));
}

use std::ops::{Index, DerefMut};

impl Index<&str> for Node {
    type Output = Node;

    fn index(&self, index: &str) -> &Self::Output {
		for child in &self.children {
			match &child.node {
				Nodes::Feature { name, feature: _ } => {
					if name == index {
						return child.as_ref();
					}
				}
				_ => {}
			}
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

	use crate::jspd::tokenizer::tokenize;

	fn print_tree(node: &Node) {
		match &node.node {
			Nodes::Feature { name, feature } => {
				println!("{}: {:#?}", name, feature);
				for child in &node.children {
					print_tree(child);
				}
			}
			Nodes::Expression { expression, children } => {
				println!("{:#?}", expression);
				for child in children {
					print_tree(child);
				}
			}
		}
	}

	fn assert_struct(node: &Node) {
		if let (Nodes::Feature { name, feature }, _light) = (&node.node, node) {
			assert_eq!(name, "Light");

			if let Features::Struct { fields } = feature {
				assert_eq!(fields.len(), 2);

				let position = &fields[0];

				if let Nodes::Feature { name, feature } = &position.node {
					assert_eq!(name, "position");

					if let Features::Member { r#type } = feature {
						assert_eq!(r#type, "vec3f");
					} else { panic!("Not a member"); }
				} else { panic!("Not a feature"); }

				let color = &fields[1];

				if let Nodes::Feature { name, feature } = &color.node {
					assert_eq!(name, "color");

					if let Features::Member { r#type } = feature {
						assert_eq!(r#type, "vec3f");
					} else { panic!("Not a member"); }
				} else { panic!("Not a feature"); }
			} else { panic!("Not a struct"); }
		} else { panic!("Not a feature"); }
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

		let root_node = &node.node;

		if let Nodes::Feature { name, feature: _ } = root_node {
			assert_eq!(name, "root");

			assert_struct(&node["Light"]);
		}
	}

	fn assert_function(node: &Node) {
		let main_node = &node.node;

		if let Nodes::Feature { name, feature } = &main_node {
			assert_eq!(name, "main");

			if let Features::Function { params, return_type, statements, raw: _ } = feature {
				assert_eq!(params.len(), 0);
				assert_eq!(return_type, "void");
				assert_eq!(statements.len(), 2);

				let statement = &statements[0];

				if let Nodes::Expression { expression, children } = &statement.node {
					if let Expressions::Operator { name } = expression {
						assert_eq!(name, "=");

						let var_decl = &children[0];

						if let Nodes::Expression { expression, children: _ } = &var_decl.node {
							if let Expressions::VariableDeclaration{ name, r#type } = expression {
								assert_eq!(name, "position");
								assert_eq!(r#type, "vec4f");
							} else { panic!("Not a variable declaration"); }
						} else { panic!("Not an expression"); }

						let function_call = &children[1];

						if let Nodes::Expression { expression, children } = &function_call.node {
							if let Expressions::FunctionCall{ name } = expression {
								assert_eq!(name, "vec4");
								assert_eq!(children.len(), 4);

								// TODO: assert values
							} else { panic!("Not a function call"); }
						} else { panic!("Not an expression"); }
					} else { panic!("Not an assignment");}
				} else { panic!("Not an expression"); }
			} else { panic!("Not a function"); }
		} else { panic!("Not a feature"); }
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

		if let Nodes::Feature { name, feature: _ } = &node.node {
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

		if let Nodes::Feature { name, feature: _ } = &node.node {
			assert_eq!(name, "root");

			let main_node = &node["main"];

			if let Nodes::Feature { name, feature } = &main_node.node {
				assert_eq!(name, "main");

				if let Features::Function { params, return_type, statements, raw } = feature {
					assert_eq!(statements.len(), 2);

					let statement0 = &statements[0];

					if let Nodes::Expression { expression, children } = &statement0.node {
						if let Expressions::Operator { name } = expression {
							assert_eq!(name, "=");

							let var_decl = &children[0];

							if let Nodes::Expression { expression, children } = &var_decl.node {
								if let Expressions::VariableDeclaration { name, r#type } = expression {
								} else { panic!("Not a variable declaration"); }
							} else { panic!("Not an expression"); }

							let multiply = &children[1];

							if let Nodes::Expression { expression, children } = &multiply.node {
								if let Expressions::Operator { name, } = expression {
									assert_eq!(name, "*");
								} else { panic!("Not a variable declaration"); }

								let vec4 = &children[0];

								if let Nodes::Expression { expression, children } = &vec4.node {
									if let Expressions::FunctionCall { name, } = expression {
										assert_eq!(name, "vec4");
									} else { panic!("Not a variable declaration"); }
								} else { panic!("Not an expression"); }

								let literal = &children[1];

								if let Nodes::Expression { expression, children } = &literal.node {
									if let Expressions::Literal { value, } = expression {
										assert_eq!(value, "2.0");
									} else { panic!("Not a variable declaration"); }
								} else { panic!("Not an expression"); }
							} else { panic!("Not an expression"); }
						} else { panic!("Not an assignment"); }
					} else { panic!("Not an expression"); }
				} else { panic!("Not a function"); }
			} else { panic!("Not a feature"); }
		} else { panic!("Not root node") }
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

		if let Nodes::Feature { name, feature: _ } = &node.node {
			assert_eq!(name, "root");

			let main_node = &node["main"];

			if let Nodes::Feature { name, feature } = &main_node.node {
				assert_eq!(name, "main");

				if let Features::Function { params, return_type, statements, raw } = feature {
					assert_eq!(statements.len(), 3);

					let statement1 = &statements[1];

					if let Nodes::Expression { expression, children } = &statement1.node {
						if let Expressions::Operator { name } = expression {
							assert_eq!(children.len(), 2);
							assert_eq!(name, "=");

							let accessor = &children[0];

							if let Nodes::Expression { expression, children } = &accessor.node {
								if let Expressions::Accessor = expression {
									assert_eq!(children.len(), 2); // position && y

									let position = &children[0];

									if let Nodes::Expression { expression, children } = &position.node {
										if let Expressions::Member { name } = expression {
											assert_eq!(name, "position");
										} else { panic!("Not a variable declaration"); }
									} else { panic!("Not an expression"); }

									let y = &children[1];

									if let Nodes::Expression { expression, children } = &y.node {
										if let Expressions::Member { name } = expression {
											assert_eq!(name, "y");
										} else { panic!("Not a variable declaration"); }
									} else { panic!("Not an expression"); }
								} else { panic!("Not an accessor"); }
							} else { panic!("Not an expression"); }

							let literal = &children[1];

							if let Nodes::Expression { expression, children } = &literal.node {
								if let Expressions::Literal { value, } = expression {
									assert_eq!(value, "2.0");
								} else { panic!("Not a variable declaration"); }
							} else { panic!("Not an expression"); }
						} else { panic!("Not an operator"); }
					} else { panic!("Not an expression"); }
				} else { panic!("Not a function"); }
			} else { panic!("Not a feature"); }
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

		if let Nodes::Feature { name, feature: _ } = &node.node {
			assert_eq!(name, "root");

			assert_struct(&node["Light"]);
			assert_function(&node["main"]);			
		} else { panic!("Not root node") }
	}

	#[test]
	fn test_parse_member() {
		let source = "color: In<vec4f>;";

		let tokens = tokenize(source).expect("Failed to tokenize");
		let (node, program) = parse(tokens).expect("Failed to parse");

		if let Nodes::Feature { name, feature } = &node.node {
			assert_eq!(name, "root");

			let member_node = &node["color"];

			if let Nodes::Feature { name, feature } = &member_node.node {
				assert_eq!(name, "color");

				if let Features::Member { r#type } = &feature {
					assert_eq!(r#type, "In<vec4f>");
				} else { panic!("Not a member"); }
			} else { panic!("Not a feature"); }
		}
	}
}