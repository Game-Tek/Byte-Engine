use std::{rc::Rc, collections::HashMap};

use super::parser;

pub(super) fn lex(node: &parser::Node, parser_program: &parser::ProgramState) -> Result<Node, LexError> {
	let mut program = ProgramState {
		types: HashMap::new(),
	};

	return lex_parsed_node(node, parser_program, &mut program).map(|e| e.as_ref().clone());
}

#[derive(Clone, Debug)]
pub struct Node {
	pub node: Nodes,
}

#[derive(Clone, Debug)]
pub enum Nodes {
	Scope{ name: String, children: Vec<Rc<Node>> },
	Struct {
		name: String,
		template: Option<Rc<Node>>,
		fields: Vec<Rc<Node>>,
		types: Vec<Rc<Node>>,
	},
	Member {
		name: String,
		r#type: Rc<Node>,
	},
	Function {
		name: String,
		params: Vec<Rc<Node>>,
		return_type: Rc<Node>,
		statements: Vec<Rc<Node>>,
		raw: Option<String>,
	},
	Expression(Expressions),
	GLSL {
		code: String,
	}
}

#[derive(Clone, Debug)]
pub(crate) enum Features {

}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operators {
	Plus,
	Minus,
	Multiply,
	Divide,
	Modulo,
	Assignment,
	Equality,
}

#[derive(Clone, Debug)]
pub enum Expressions {
	Member{ name: String },
	Literal { value: String },
	FunctionCall { name: String, parameters: Vec<Rc<Node>> },
	Operator {
		operator: Operators,
		left: Rc<Node>,
		right: Rc<Node>,
	},
	VariableDeclaration {
		name: String,
		// r#type: Rc<Node>,
		r#type: String,
	},
	Accessor {
		left: Rc<Node>,
		right: Rc<Node>,
	}
}

#[derive(Debug)]
pub(crate) enum LexError {
	Undefined,
	NoSuchType{
		type_name: String,
	},
}

type LexerReturn<'a> = Result<(Rc<Node>, std::slice::Iter<'a, String>), LexError>;
type Lexer<'a> = fn(std::slice::Iter<'a, String>, &'a parser::ProgramState) -> LexerReturn<'a>;

#[derive(Clone)]
pub(crate) struct ProgramState {
	pub(crate) types: HashMap<String, Rc::<Node>>,
}

/// Execute a list of lexers on a stream of tokens.
fn execute_lexers<'a>(lexers: &[Lexer<'a>], iterator: std::slice::Iter<'a, String>, program: &'a parser::ProgramState) -> LexerReturn<'a> {
	for lexer in lexers {
		if let Ok(r) = lexer(iterator.clone(), program) {
			return Ok(r);
		}
	}

	Err(LexError::Undefined) // No lexer could handle this syntax.
}

/// Tries to execute a list of lexers on a stream of tokens. But it's ok if none of them can handle the syntax.
fn try_execute_lexers<'a>(lexers: &[Lexer<'a>], iterator: std::slice::Iter<'a, String>, program: &'a parser::ProgramState) -> Option<LexerReturn<'a>> {
	for lexer in lexers {
		if let Ok(r) = lexer(iterator.clone(), program) {
			return Some(Ok(r));
		}
	}

	None
}

fn lex_parsed_node(parser_node: &parser::Node, parser_program: &parser::ProgramState, program: &mut ProgramState) -> Result<Rc<Node>, LexError> {
	match &parser_node.node {
		parser::Nodes::Scope{ name, children } => {
			let mut ch = Vec::new();

			for child in children {
				ch.push(lex_parsed_node(child, parser_program, program)?);
			}

			Ok(Rc::new(Node {
				node: Nodes::Scope{ name: name.clone(), children: ch, }
			}))
		}
		parser::Nodes::Struct { name, fields } => {
			if let Some(n) = program.types.get(name) { // If the type already exists, return it.
				return Ok(n.clone());
			}

			let mut children = Vec::new();

			for field in fields {
				children.push(lex_parsed_node(&field, parser_program, program)?);
			}

			let struct_node = Node {
				node: Nodes::Struct {
					name: name.clone(),
					template: None,
					fields: children,
					types: Vec::new(),
				},
			};

			let node = Rc::new(struct_node);

			program.types.insert(name.clone(), node.clone());
			program.types.insert(format!("{}*", name.clone()), node.clone());

			Ok(node)
		}
		parser::Nodes::Member { name, r#type } => {
			let t = if r#type.contains('<') {
				let mut s = r#type.split(|c| c == '<' || c == '>');

				let outer_type_name = s.next().ok_or(LexError::Undefined)?;

				let outer_type = lex_parsed_node(parser_program.types.get(outer_type_name).ok_or(LexError::NoSuchType{ type_name: outer_type_name.to_string() })?, parser_program, program)?;

				let inner_type_name = s.next().ok_or(LexError::Undefined)?;

				let inner_type = if let Some(stripped) = inner_type_name.strip_suffix('*') {
					let x = Rc::new(
						Node {
							node: Nodes::Struct {
								name: format!("{}*", stripped),
								template: Some(outer_type.clone()),
								fields: Vec::new(),
								types: Vec::new(),
							},
						}
					);

					program.types.insert(format!("{}*", stripped), x.clone());

					x
				} else {					
					lex_parsed_node(parser_program.types.get(inner_type_name).ok_or(LexError::NoSuchType{ type_name: inner_type_name.to_string() })?, parser_program, program)?
				};

				if let Some(n) = program.types.get(r#type) { // If the type already exists, return it.
					return Ok(n.clone());
				}

				let children = Vec::new();

				// for field in fields {
				// 	children.push(lex_parsed_node(&field, parser_program, program)?);
				// }

				let struct_node = Node {
					node: Nodes::Struct {
						name: r#type.clone(),
						template: Some(outer_type.clone()),
						fields: children,
						types: vec![inner_type],
					},
				};

				let node = Rc::new(struct_node);

				program.types.insert(r#type.clone(), node.clone());

				node
			} else {
				let t = parser_program.types.get(r#type.as_str()).ok_or(LexError::NoSuchType{ type_name: r#type.clone() })?;
				lex_parsed_node(t, parser_program, program)?
			};

			Ok(Rc::new(Node {
				node: Nodes::Member {
					name: name.clone(),
					r#type: t,
				},
			}))
		}
		parser::Nodes::Function { name, return_type, statements, raw, .. } => {
			let t = parser_program.types.get(return_type.as_str()).ok_or(LexError::NoSuchType{ type_name: return_type.clone() })?;
			let t = lex_parsed_node(t, parser_program, program)?;

			return Ok(Rc::new(Node {
				node: Nodes::Function {
					name: name.clone(),
					params: Vec::new(),
					return_type: t,
					statements: statements.iter().map(|e| lex_parsed_node(e, parser_program, program).unwrap()).collect(),
					raw: raw.clone(),
				},
			}));
		}
		parser::Nodes::Expression(expression) => {
			match expression {
				parser::Expressions::Accessor{ left, right } => {
					Ok(Rc::new(Node {
						node: Nodes::Expression(Expressions::Accessor {
							left: lex_parsed_node(left, parser_program, program)?,
							right: lex_parsed_node(right, parser_program, program)?,
						}),
					}))
				}
				parser::Expressions::Member{ name } => {
					Ok(Rc::new(Node {
						node: Nodes::Expression(Expressions::Member {
							name: name.clone(),

						}),
					}))
				}
				parser::Expressions::Literal{ value } => {
					Ok(Rc::new(Node {
						node: Nodes::Expression(Expressions::Literal {
							value: value.clone(),
						}),
					}))
				}
				parser::Expressions::FunctionCall{ name, parameters } => {
					Ok(Rc::new(Node {
						node: Nodes::Expression(Expressions::FunctionCall {
							name: name.clone(),
							parameters: parameters.iter().map(|e| lex_parsed_node(e, parser_program, program).unwrap()).collect(),
						}),
					}))
				}
				parser::Expressions::Operator{ name, left, right } => {
					Ok(Rc::new(Node {
						node: Nodes::Expression(Expressions::Operator {
							operator: match name.as_str() {
								"+" => Operators::Plus,
								"-" => Operators::Minus,
								"*" => Operators::Multiply,
								"/" => Operators::Divide,
								"%" => Operators::Modulo,
								"=" => Operators::Assignment,
								"==" => Operators::Equality,
								_ => { panic!("Invalid operator") }
							},
							left: lex_parsed_node(left, parser_program, program)?,
							right: lex_parsed_node(right, parser_program, program)?,
						}),
					}))
				}
				parser::Expressions::VariableDeclaration{ name, r#type } => {
					Ok(Rc::new(Node {
						node: Nodes::Expression(Expressions::VariableDeclaration {
							name: name.clone(),
							// r#type: lex_parsed_node(&r#type, parser_program, program)?,
							r#type: r#type.clone(),
						}),
					}))
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::tokenizer;

	use super::*;

	fn assert_type(node: &Node, type_name: &str) {
		match &node.node {
			Nodes::Struct { name, fields, template, types } => {
				assert_eq!(name, type_name);
			}
			_ => { panic!("Expected type"); }
		}
	}

	#[test]
	fn lex_function() {
		let source = "
main: fn () -> void {
	position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		let node = &lex(&node, &program).expect("Failed to lex");

		match &node.node {
			Nodes::Scope{ name, children } => {
				let main = &children[0];

				match &main.node {
					Nodes::Function { name, params: _, return_type, statements, raw: _ } => {
						assert_eq!(name, "main");
						assert_type(&return_type, "void");

						let position = &statements[0];

						match &position.node {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								let position = &left;

								assert_eq!(operator, &Operators::Assignment);

								match &position.node {
									Nodes::Expression(Expressions::VariableDeclaration{ name, r#type }) => {
										assert_eq!(name, "position");
										
										// assert_type(&r#type, "vec4f");
										assert_eq!(r#type, "vec4f");
									}
									_ => { panic!("Expected expression"); }
								}

								let constructor = &right;

								match &constructor.node {
									Nodes::Expression(Expressions::FunctionCall{ name, parameters }) => {
										assert_eq!(name, "vec4");
										assert_eq!(parameters.len(), 4);
									}
									_ => { panic!("Expected expression"); }
								}
							}
							_ => { panic!("Expected variable declaration"); }
						}
					}
					_ => { panic!("Expected function."); }
				}
			}
			_ => { panic!("Expected scope"); }
		}
	}

	#[test]
	fn lex_member() {
		let source = "
color: In<vec4f>;
";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		let node = &lex(&node, &program).expect("Failed to lex");

		match &node.node {
			Nodes::Scope{ name, children } => {
				assert_eq!(name, "root");

				let color = &children[0];

				match &color.node {
					Nodes::Member { name, r#type } => {
						assert_eq!(name, "color");						
						assert_type(&r#type, "In<vec4f>");
					}
					_ => { panic!("Expected feature"); }
				}
			}
			_ => { panic!("Expected scope"); }
		}
	}
}