use std::{rc::Rc, collections::HashMap};

use super::parser;

pub(super) fn lex(node: &parser::Node, parser_program: &parser::ProgramState) -> Result<Node, LexError> {
	let mut program = ProgramState {
		types: HashMap::new(),
	};

	return lex_parsed_node(node, parser_program, &mut program).map(|e| e.as_ref().clone());
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
	pub(crate) node: Nodes,
	pub(crate) children: Vec<Rc<Node>>,
}

#[derive(Clone, Debug)]
pub(crate) enum Nodes {
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
pub(crate) enum Features {
	Root,
	Scope,
	Struct {
		fields: Vec<Rc<Node>>
	},
	Member {
		r#type: Rc<Node>,
	},
	Function {
		params: Vec<Rc<Node>>,
		return_type: Rc<Node>,
		statements: Vec<Rc<Node>>,
		raw: Option<String>,
	},
}

#[derive(Clone, Debug)]
pub(crate) enum Expressions {
	Member,
	Literal{ value: String },
	FunctionCall{ name: String },
	VariableDeclaration{
		name: String,
		r#type: Rc<Node>,
	},
	Assignment,
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

	return Err(LexError::Undefined); // No lexer could handle this syntax.
}

/// Tries to execute a list of lexers on a stream of tokens. But it's ok if none of them can handle the syntax.
fn try_execute_lexers<'a>(lexers: &[Lexer<'a>], iterator: std::slice::Iter<'a, String>, program: &'a parser::ProgramState) -> Option<LexerReturn<'a>> {
	for lexer in lexers {
		if let Ok(r) = lexer(iterator.clone(), program) {
			return Some(Ok(r));
		}
	}

	return None;
}

fn lex_parsed_node(parser_node: &parser::Node, parser_program: &parser::ProgramState, program: &mut ProgramState) -> Result<Rc<Node>, LexError> {
	match &parser_node.node {
		parser::Nodes::Feature { name, feature } => {
			match feature {
				parser::Features::Root => {
					let mut children = Vec::new();

					for child in &parser_node.children {
						children.push(lex_parsed_node(child, parser_program, program)?);
					}

					return Ok(Rc::new(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Root,
						},
						children,
					}));
				}
				parser::Features::Scope => {
					let mut children = Vec::new();

					for child in &parser_node.children {
						children.push(lex_parsed_node(child, parser_program, program)?);
					}

					return Ok(Rc::new(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Scope,
						},
						children,
					}));
				}
				parser::Features::Struct { fields } => {
					let mut children = Vec::new();

					for field in fields {
						children.push(lex_parsed_node(&field, parser_program, program)?);
					}

					let struct_node = Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Struct {
								fields: children,
							},
						},
						children: Vec::new(),
					};

					let node = Rc::new(struct_node);

					program.types.insert(name.clone(), node.clone());

					return Ok(node);
				}
				parser::Features::Member { r#type } => {
					let t = parser_program.types.get(r#type.as_str()).ok_or(LexError::NoSuchType{ type_name: r#type.clone() })?;
					let t = lex_parsed_node(t, parser_program, program)?;

					return Ok(Rc::new(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Member {
								r#type: t,
							},
						},
						children: Vec::new(),
					}));
				}
				parser::Features::Function { params, return_type, statements, raw } => {
					let t = parser_program.types.get(return_type.as_str()).ok_or(LexError::NoSuchType{ type_name: return_type.clone() })?;
					let t = lex_parsed_node(t, parser_program, program)?;

					return Ok(Rc::new(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Function {
								params: Vec::new(),
								return_type: t,
								statements: statements.iter().map(|e| lex_parsed_node(e, parser_program, program).unwrap()).collect(),
								raw: raw.clone(),
							},
						},
						children: Vec::new(),
					}));
				}
			}
		}
		parser::Nodes::Expression { expression, children } => {
			match expression {
				parser::Expressions::Member => {
					return Ok(Rc::new(Node {
						node: Nodes::Expression {
							expression: Expressions::Member,
							children: children.iter().map(|e| lex_parsed_node(e, parser_program, program).unwrap()).collect(),
						},
						children: Vec::new(),
					}));
				}
				parser::Expressions::Literal{ value } => {
					return Ok(Rc::new(Node {
						node: Nodes::Expression {
							expression: Expressions::Literal{ value: value.clone() },
							children: children.iter().map(|e| lex_parsed_node(e, parser_program, program).unwrap()).collect(),
						},
						children: Vec::new(),
					}));
				}
				parser::Expressions::FunctionCall{ name } => {
					return Ok(Rc::new(Node {
						node: Nodes::Expression {
							expression: Expressions::FunctionCall{ name: name.clone() },
							children: children.iter().map(|e| lex_parsed_node(e, parser_program, program).unwrap()).collect(),
						},
						children: Vec::new(),
					}));
				}
				parser::Expressions::VariableDeclaration{ name, r#type } => {
					return Ok(Rc::new(Node {
						node: Nodes::Expression {
							expression: Expressions::VariableDeclaration{ name: name.clone(), r#type: lex_parsed_node(parser_program.types.get(r#type).unwrap(), parser_program, program).unwrap() },
							children: children.iter().map(|e| lex_parsed_node(e, parser_program, program).unwrap()).collect(),
						},
						children: Vec::new(),
					}));
				}
				parser::Expressions::Assignment => {
					return Ok(Rc::new(Node {
						node: Nodes::Expression {
							expression: Expressions::Assignment,
							children: children.iter().map(|e| lex_parsed_node(e, parser_program, program).unwrap()).collect(),
						},
						children: Vec::new(),
					}));
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::jspd::tokenizer;

	use super::*;

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
			Nodes::Feature { name, feature } => {
				match feature {
					Features::Scope => {
						assert_eq!(name, "root");

						let main = &node.children[0];

						match &main.node {
							Nodes::Feature { name, feature } => {
								match feature {
									Features::Function { params: _, return_type: _, statements, raw: _ } => {
										assert_eq!(name, "main");
										// TODO: assert return type

										let position = &statements[0];

										match &position.node {
											Nodes::Expression { expression, children } => {
												match expression {
													Expressions::Assignment => {
														assert_eq!(children.len(), 2);

														let position = &children[0];

														match &position.node {
															Nodes::Expression { expression, children: _ } => {
																match expression {
																	Expressions::VariableDeclaration{ name, r#type } => {
																		assert_eq!(name, "position");
																		// TODO: assert type
																	}
																	_ => { panic!("Expected variable declaration"); }
																}
															}
															_ => { panic!("Expected expression"); }
														}

														let constructor = &children[1];

														match &constructor.node {
															Nodes::Expression { expression, children } => {
																match expression {
																	Expressions::FunctionCall{ name } => {
																		assert_eq!(name, "vec4");
																		assert_eq!(children.len(), 4);
																	}
																	_ => { panic!("Expected function call"); }
																}
															}
															_ => { panic!("Expected expression"); }
														}
													}
													_ => { panic!("Expected variable declaration"); }
												}
											}
											_ => { panic!("Expected expression"); }
										}
									}
									_ => { panic!("Expected function got: {:#?}", feature); }
								}
							}
							_ => { panic!("Expected feature"); }
						}
					}
					_ => { panic!("Expected scope"); }
				}
			}
			_ => { panic!("Expected scope"); }
		}
	}
}