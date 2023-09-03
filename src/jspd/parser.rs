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

	let f32 = Rc::new(make_no_member_struct("f32"));
	let vec2f = Rc::new(make_struct("vec2f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32"))]));
	let vec3f = Rc::new(make_struct("vec3f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32")), Rc::new(make_member("z", "f32"))]));
	let vec4f = Rc::new(make_struct("vec4f", vec![Rc::new(make_member("x", "f32")), Rc::new(make_member("y", "f32")), Rc::new(make_member("z", "f32")), Rc::new(make_member("w", "f32"))]));

	program_state.types.insert("f32".to_string(), f32);
	program_state.types.insert("vec2f".to_string(), vec2f);
	program_state.types.insert("vec3f".to_string(), vec3f);
	program_state.types.insert("vec4f".to_string(), vec4f);

	let mut iterator = tokens.iter();

	let parsers = vec![
		parse_struct,
		parse_function,
		parse_macro,
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

use std::{collections::HashMap, rc::Rc};

#[derive(Clone)]
pub(super) struct Node {
	pub(crate) node: Nodes,
	pub(crate) children: Vec<Rc<Node>>,
}

#[derive(Clone)]
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

#[derive(Clone)]
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

#[derive(Clone)]
pub(super) enum Expressions {
	Member,
	Literal,
	FunctionCall,
	VariableDeclaration,
	Assignment,
}

#[derive(Debug)]
pub(super) enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax,
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
			Expressions::Member => 0,
			Expressions::Literal => 0,
			Expressions::FunctionCall => 0,
			Expressions::VariableDeclaration => 0,
			Expressions::Assignment => 255,
		}
	}
}

/// Type of the result of a parser.
type ParserResult<'a> = Result<((Rc<Node>, ProgramState), std::slice::Iter<'a, String>), ParsingFailReasons>;

/// A parser is a function that tries to parse a sequence of tokens.
type Parser<'a> = fn(std::slice::Iter<'a, String>, &ProgramState) -> ParserResult<'a>;

/// Execute a list of parsers on a stream of tokens.
fn execute_parsers<'a>(parsers: &[Parser<'a>], iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone(), program) {
			return Ok(r);
		}
	}

	return Err(ParsingFailReasons::BadSyntax); // No parser could handle this syntax.
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

fn parse_macro<'a>(iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let mut iter = iterator;

	iter.next().and_then(|v| if v == "#" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().and_then(|v| if v == "[" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().ok_or(ParsingFailReasons::BadSyntax)?;
	iter.next().and_then(|v| if v == "]" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax)?;

	return Ok(((Rc::new(make_scope("MACRO", vec![])), program.clone()), iter));
}

fn parse_struct<'a>(mut iterator: std::slice::Iter<'a, String>, program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "struct" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax)?;

	let mut fields = vec![];

	while let Some(v) = iterator.next() {
		if v == "}" {
			break;
		} else if v == "," {
			continue;
		}

		let colon = iterator.next().unwrap();

		if colon != ":" { return Err(ParsingFailReasons::BadSyntax); }

		let type_name = iterator.next().unwrap();

		if !type_name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::BadSyntax); }

		fields.push(Rc::new(make_member(v, type_name)));
	}

	let node = Rc::new(make_struct(name, fields));

	let mut program = program.clone();

	program.types.insert(name.clone(), node.clone());

	return Ok(((node, program.clone()), iterator));
}

fn make_expression_node(following_expression: Option<&Node>, new_expression: Expressions, new_node_children: Option<Vec<Rc<Node>>>) -> Node {
	if let Some(following_expression) = following_expression {
		if let Nodes::Expression { expression, children } = &following_expression.node {
			if expression.precedence() > new_expression.precedence() {
				let mut cont = following_expression.clone();
				if let Nodes::Expression { expression, children } = &mut cont.node {
					children.insert(0, Rc::new(make_expression_node(None, new_expression, new_node_children.clone())));
				}
				cont
			} else {
				let node = Node {
					node: Nodes::Expression {
						expression: new_expression,
						children: new_node_children.unwrap_or(vec![]),
					},
					children: vec![],
				};
		
				node
			}
		} else { panic!("Not an expression"); }
	} else {
		Node {
			node: Nodes::Expression {
				expression: new_expression,
				children: new_node_children.unwrap_or(vec![]),
			},
			children: vec![],
		}
	}
}

fn parse_var_decl<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().ok_or(ParsingFailReasons::BadSyntax)?;

	let possible_following_expressions: Vec<Parser> = vec![
		parse_assignment,
	];

	let ((expression, _), new_iterator) = execute_parsers(&possible_following_expressions, iterator.clone(), program)?;

	let expression = make_expression_node(Some(&expression), Expressions::VariableDeclaration, None);

	return Ok(((Rc::new(expression), program.clone()), new_iterator));
}

fn parse_assignment<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	iterator.next().and_then(|v| if v == "=" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let possible_following_expressions: Vec<Parser> = vec![
		parse_rvalue,
	];

	let ((expression, _), new_iterator) = execute_parsers(&possible_following_expressions, iterator.clone(), program)?;

	let expression = make_expression_node(Some(&expression), Expressions::Assignment, Some(vec![expression.clone()]));

	return Ok(((Rc::new(expression), program.clone()), new_iterator));
}

fn parse_variable<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;

	let lexers: Vec<Parser> = vec![
		parse_assignment,
	];

	if let Some(Ok(((expression, _), new_iterator))) = try_execute_parsers(&lexers, iterator.clone(), program) {
		return Ok(((Rc::new(make_expression_node(Some(&expression), Expressions::Member, None)), program.clone()), new_iterator));
	} else {
		return Ok(((Rc::new(make_expression_node(None, Expressions::Member, None)), program.clone()), iterator));
	}
}

fn parse_accessor<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;

	let lexers: Vec<Parser> = vec![
		parse_variable,
	];

	let ((expression, _), new_iterator) = execute_parsers(&lexers, iterator.clone(), program)?;

	return Ok(((Rc::new(make_expression_node(Some(&expression), Expressions::Member, None)), program.clone()), new_iterator));
}

fn parse_literal<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().and_then(|v| if v == "1.0" || v == "0.0" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	return Ok(((Rc::new(make_expression_node(None, Expressions::Literal, None)), program.clone()), iterator));
}

fn parse_rvalue<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	let parsers = vec![
		parse_function_call,
		parse_literal,
		parse_variable,
	];

	return execute_parsers(&parsers, iterator.clone(), program);
}

fn parse_function_call<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let mut children = vec![];

	loop {
		let ((expression, _), new_iterator) = if let Ok(r) = parse_rvalue(iterator.clone(), program) { r } else { return Err(ParsingFailReasons::BadSyntax); };

		children.push(expression);

		iterator = new_iterator;

		// Check if iter is comma
		if iterator.clone().peekable().peek().unwrap().as_str() == "," {
			iterator.next();
		}

		// check if iter is close brace
		if iterator.clone().peekable().peek().unwrap().as_str() == ")" {
			iterator.next();
			break;
		}
	}

	return Ok(((Rc::new(make_expression_node(None, Expressions::FunctionCall, Some(children))), program.clone()), iterator));
}

fn parse_statement<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	let parsers = vec![
		parse_var_decl,
		parse_variable,
		parse_function_call,
	];

	let (lexeme, mut new_iterator) = if let Ok(r) = execute_parsers(&parsers, iterator.clone(), program) { r } else { return Err(ParsingFailReasons::BadSyntax); };

	new_iterator.next().and_then(|f| if f == ";" { Some(f) } else { None }).ok_or(ParsingFailReasons::BadSyntax)?;

	return Ok((lexeme, new_iterator));
}

fn parse_function<'a>(mut iterator: std::slice::Iter<'a, String>, mut program: &ProgramState) -> ParserResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "fn" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ")" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "->" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let return_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax)?;

	iterator.next().and_then(|v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax)?;

	let mut statements = vec![];

	loop {
		let ((expression, _), new_iterator) = if let Ok(r) = parse_statement(iterator.clone(), program) { r } else { return Err(ParsingFailReasons::BadSyntax); };

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

use std::ops::Index;

impl Index<&str> for Node {
    type Output = Node;

    fn index(&self, index: &str) -> &Self::Output {
		for child in &self.children {
			match &child.node {
				Nodes::Feature { name, feature } => {
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

		if let Nodes::Feature { name, feature } = root_node {
			assert_eq!(name, "root");

			let light_node = &node["Light"].node;

			if let (Nodes::Feature { name, feature }, light) = (light_node, light_node) {
				assert_eq!(name, "Light");

				if let Features::Struct { fields } = feature {
					assert_eq!(fields.len(), 2);

					let position = &fields[0];

					if let Nodes::Feature { name, feature } = &position.node {
						assert_eq!(name, "position");

						if let Features::Member { r#type } = feature {
							assert_eq!(r#type, "vec3f");
						} else {
							panic!("Not a member");
						}
					} else {
						panic!("Not a feature");
					}

					let color = &fields[1];

					if let Nodes::Feature { name, feature } = &color.node {
						assert_eq!(name, "color");

						if let Features::Member { r#type } = feature {
							assert_eq!(r#type, "vec3f");
						} else {
							panic!("Not a member");
						}
					} else {
						panic!("Not a feature");
					}
				} else {
					panic!("Not a struct");
				}
			} else {
				panic!("Not a feature");
			}
		}
	}

	#[test]
	fn test_parse_function() {
		let source = "
main: fn () -> void {
	position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let (node, program) = parse(tokens).expect("Failed to parse");

		let root_node = &node.node;

		if let Nodes::Feature { name, feature } = &root_node {
			assert_eq!(name, "root");

			let main_node = &node["main"].node;

			if let Nodes::Feature { name, feature } = &main_node {
				assert_eq!(name, "main");

				if let Features::Function { params, return_type, statements, raw } = feature {
					assert_eq!(params.len(), 0);
					assert_eq!(return_type, "void");
					assert_eq!(statements.len(), 2);

					let statement = &statements[0];

					if let Nodes::Expression { expression, children } = &statement.node {
						if let Expressions::Assignment = expression {
							let var_decl = &children[0];

							if let Nodes::Expression { expression, children } = &var_decl.node {
								if let Expressions::VariableDeclaration = expression {
								} else { panic!("Not a variable declaration"); }
							} else { panic!("Not an expression"); }

							let function_call = &children[1];

							if let Nodes::Expression { expression, children } = &function_call.node {
								if let Expressions::FunctionCall = expression {
									assert_eq!(children.len(), 4);
								} else { panic!("Not a function call"); }
							} else { panic!("Not an expression"); }
						} else { panic!("Not an assignment");}
					} else { panic!("Not an expression"); }
				} else { panic!("Not a function"); }
			} else { panic!("Not a feature"); }
		} else { panic!("Not root node") }
	}

// 	#[test]
// 	fn test_parse_struct_and_function() {
// 		let source = "
// Light: struct {
// 	position: vec3f,
// 	color: vec3f
// }

// #[vertex]
// main: fn () -> void {
// 	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
// 	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
// }";

// 		let tokens = tokenize(source).unwrap();
// 		let nodes = parse(tokens);

// 		assert_eq!(nodes.is_ok(), true);

// 		let root_node = nodes.unwrap();

// 		let struct_node = &root_node["Light"];

// 		assert_eq!(struct_node["type"], "struct");

// 		let position_node = &struct_node["position"];

// 		assert_eq!(&position_node["type"], "member");
// 		assert_eq!(&position_node["data_type"], "vec3f");

// 		let color_node = &struct_node["color"];

// 		assert_eq!(&color_node["type"], "member");
// 		assert_eq!(&color_node["data_type"], "vec3f");

// 		let function_node = &root_node["main"];

// 		assert_eq!(&function_node["type"], "function");
// 		assert_eq!(function_node["data_type"], "void");
// 	}
}