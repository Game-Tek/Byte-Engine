use std::{rc::Rc, collections::HashMap};

use super::parser;

pub(super) fn lex(node: &parser::Node, parser_program: &parser::ProgramState) -> Result<Node, LexError> {
	let mut program = ProgramState {
		types: HashMap::new(),
	};

	return lex_parsed_node(node, parser_program, &mut program);
}

#[derive(Clone)]
pub(crate) struct Node {
	pub(crate) node: Nodes,
	pub(crate) children: Vec<Rc<Node>>,
}

#[derive(Clone)]
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

#[derive(Clone)]
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
		return_type: String,
		statements: Vec<Rc<Node>>,
		raw: Option<String>,
	},
}

#[derive(Clone)]
pub(crate) enum Expressions {
	Member,
	Literal,
	FunctionCall,
	VariableDeclaration,
	Assignment,
}

pub(crate) enum LexError {
	Undefined,
	NoSuchType,
}

type LexerReturn<'a> = Result<(Node, std::slice::Iter<'a, String>), LexError>;
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

fn lex_parsed_node(parser_node: &parser::Node, parser_program: &parser::ProgramState, program: &mut ProgramState) -> Result<Node, LexError> {
	match &parser_node.node {
		parser::Nodes::Feature { name, feature } => {
			match feature {
				parser::Features::Root => {
					let mut children = Vec::new();

					for child in &parser_node.children {
						children.push(Rc::new(lex_parsed_node(child, parser_program, program)?));
					}

					return Ok(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Root,
						},
						children,
					});
				}
				parser::Features::Scope => {
					let mut children = Vec::new();

					for child in &parser_node.children {
						children.push(Rc::new(lex_parsed_node(child, parser_program, program)?));
					}

					return Ok(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Scope,
						},
						children,
					});
				}
				parser::Features::Struct { fields } => {
					let mut children = Vec::new();

					for field in fields {
						children.push(Rc::new(lex_parsed_node(&field, parser_program, program)?));
					}

					return Ok(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Struct {
								fields: children,
							},
						},
						children: Vec::new(),
					});
				}
				parser::Features::Member { r#type } => {
					let t = parser_program.types.get(r#type.as_str()).ok_or(LexError::NoSuchType)?;
					let t = lex_parsed_node(t, parser_program, program)?;

					return Ok(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Member {
								r#type: Rc::new(t),
							},
						},
						children: Vec::new(),
					});
				}
				parser::Features::Function { params, return_type, statements, raw } => {
					let mut children = Vec::new();

					for param in params {
						children.push(Rc::new(lex_parsed_node(&param, parser_program, program)?));
					}

					for statement in statements {
						children.push(Rc::new(lex_parsed_node(&statement, parser_program, program)?));
					}

					return Ok(Node {
						node: Nodes::Feature {
							name: name.clone(),
							feature: Features::Function {
								params: children,
								return_type: return_type.clone(),
								statements: Vec::new(),
								raw: raw.clone(),
							},
						},
						children: Vec::new(),
					});
				}
			}
		}
		parser::Nodes::Expression { expression, children: _ } => {
			match expression {
				parser::Expressions::Member => {
					return Ok(Node {
						node: Nodes::Expression {
							expression: Expressions::Member,
							children: Vec::new(),
						},
						children: Vec::new(),
					});
				}
				parser::Expressions::Literal => {
					return Ok(Node {
						node: Nodes::Expression {
							expression: Expressions::Literal,
							children: Vec::new(),
						},
						children: Vec::new(),
					});
				}
				parser::Expressions::FunctionCall => {
					return Ok(Node {
						node: Nodes::Expression {
							expression: Expressions::FunctionCall,
							children: Vec::new(),
						},
						children: Vec::new(),
					});
				}
				parser::Expressions::VariableDeclaration => {
					return Ok(Node {
						node: Nodes::Expression {
							expression: Expressions::VariableDeclaration,
							children: Vec::new(),
						},
						children: Vec::new(),
					});
				}
				parser::Expressions::Assignment => {
					return Ok(Node {
						node: Nodes::Expression {
							expression: Expressions::Assignment,
							children: Vec::new(),
						},
						children: Vec::new(),
					});
				}
			}
		}
	}
}