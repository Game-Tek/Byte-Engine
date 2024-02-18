use std::{borrow::BorrowMut, cell::RefCell, collections::HashMap, mem::MaybeUninit, ops::Deref, rc::{Rc, Weak}};
use std::hash::Hash;

use super::parser;

pub type ParentNodeReference = Weak<RefCell<Node>>;

#[derive(Clone, Debug)]
pub struct NodeReference(Rc<RefCell<Node>>);

impl PartialEq for NodeReference {
	fn eq(&self, other: &Self) -> bool {
		Rc::ptr_eq(&self.0, &other.0)
	}
}

impl Eq for NodeReference {}

impl Hash for NodeReference {
	fn hash<H>(&self, state: &mut H) where H: std::hash::Hasher {
		Rc::as_ptr(&self.0).hash(state);
	}
}

impl Deref for NodeReference {
	type Target = RefCell<Node>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

pub(super) fn lex(node: &parser::Node, parser_program: &parser::ProgramState) -> Result<NodeReference, LexError> {
	let mut program = ProgramState {
		types: HashMap::new(),
	};

	return lex_parsed_node(None, node, parser_program, &mut program);
}

#[derive(Clone, Debug)]
pub struct Node {
	parent: Option<ParentNodeReference>,
	node: Nodes,
}

impl Node {
	fn internal_new(node: Node) -> NodeReference {
		NodeReference(Rc::new(RefCell::new(node)))
	}

	pub fn scope(name: String, children: Vec<NodeReference>) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Scope{ name, children },
		})
	}

	pub fn r#struct(name: String, fields: Vec<NodeReference>) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Struct {
				name,
				template: None,
				fields,
				types: Vec::new(),
			},
		})
	}

	pub fn member(name: String, r#type: NodeReference) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Member {
				name,
				r#type,
			},
		})
	}

	pub fn function(parent: Option<ParentNodeReference>, name: String, params: Vec<NodeReference>, return_type: NodeReference, statements: Vec<NodeReference>, raw: Option<String>) -> NodeReference {
		Self::internal_new(Node {
			parent,
			node: Nodes::Function {
				name,
				params,
				return_type,
				statements,
				raw,
			},
		})
	}

	pub fn expression(expression: Expressions) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Expression(expression),
		})
	}

	pub fn glsl(code: String) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::GLSL {
				code,
			},
		})
	}

	pub fn new(node: Nodes) -> Node {
		Node {
			parent: None,
			node,
		}
	}

	pub fn with_parent(self, parent: ParentNodeReference) -> Node {
		Node {
			parent: Some(parent),
			node: self.node,
		}
	}

	pub fn add_child(&mut self, child: NodeReference) {
		match &mut self.node {
			Nodes::Scope{ children, .. } => {
				children.push(child);
			}
			Nodes::Function { statements, .. } => {
				statements.push(child);
			}
			_ => {}
		}
	}

	pub fn add_children(&mut self, children: Vec<NodeReference>) {
		match &mut self.node {
			Nodes::Scope{ children: c, .. } => {
				c.extend(children);
			}
			Nodes::Struct { fields, .. } => {
				fields.extend(children);
			}
			_ => {}
		}
	}

	pub fn parent(&self) -> Option<ParentNodeReference> {
		self.parent.clone()
	}

	pub fn node(&self) -> &Nodes {
		&self.node
	}

	pub fn get_child(&self, child_name: &str) -> Option<NodeReference> {
		match &self.node {
			Nodes::Scope { children, .. } => {
				for child in children {
					if let Ok(borrowed_child) = child.try_borrow() {
						match borrowed_child.node() {
							Nodes::Function { name, .. } => {
								if child_name == name {
									return Some(child.clone());
								}
							}
							_ => {}
						}
					}
				}
			}
			_ => {}
		}

		None
	}
	
	pub fn get_name(&self) -> Option<String> {
		match &self.node {
			Nodes::Scope { name, .. } => {
				Some(name.clone())
			}
			Nodes::Function { name, .. } => {
				Some(name.clone())
			}
			Nodes::Member { name, .. } => {
				Some(name.clone())
			}
			Nodes::Struct { name, .. } => {
				Some(name.clone())
			}
			_ => {
				None
			}
		}
	}
}

#[derive(Clone, Debug,)]
pub enum Nodes {
	Null,
	Scope{ name: String, children: Vec<NodeReference> },
	Struct {
		name: String,
		template: Option<NodeReference>,
		fields: Vec<NodeReference>,
		types: Vec<NodeReference>,
	},
	Member {
		name: String,
		r#type: NodeReference,
	},
	Function {
		name: String,
		params: Vec<NodeReference>,
		return_type: NodeReference,
		statements: Vec<NodeReference>,
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

#[derive(Clone, Debug,)]
pub enum Expressions {
	Member{ name: String },
	Literal { value: String },
	FunctionCall {
		function: NodeReference,
		name: String,
		parameters: Vec<NodeReference>
	},
	Operator {
		operator: Operators,
		left: NodeReference,
		right: NodeReference,
	},
	VariableDeclaration {
		name: String,
		// r#type: NodeReference,
		r#type: String,
	},
	Accessor {
		left: NodeReference,
		right: NodeReference,
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
	pub(crate) types: HashMap<String, NodeReference>,
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

fn lex_parsed_node(parent_node: Option<ParentNodeReference>, parser_node: &parser::Node, parser_program: &parser::ProgramState, program: &mut ProgramState) -> Result<NodeReference, LexError> {
	let node = match &parser_node.node {
		parser::Nodes::Scope{ name, children } => {
			let this = Node::scope(name.clone(), Vec::new());

			let ch = children.iter().map(|child| {
				lex_parsed_node(Some(Rc::downgrade(&this.0)), child, parser_program, program)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;
			
			RefCell::borrow_mut(&this).add_children(ch);

			this
		}
		parser::Nodes::Struct { name, fields } => {
			if let Some(n) = program.types.get(name) { // If the type already exists, return it.
				return Ok(n.clone());
			}

			let this = Node::r#struct(name.clone(), Vec::new());

			let ch = fields.iter().map(|field| {
				lex_parsed_node(Some(Rc::downgrade(&this.0)), &field, parser_program, program)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;

			RefCell::borrow_mut(&this).add_children(ch);

			program.types.insert(name.clone(), this.clone());
			program.types.insert(format!("{}*", name.clone()), this.clone());

			this
		}
		parser::Nodes::Member { name, r#type } => {
			let t = if r#type.contains('<') {
				let mut s = r#type.split(|c| c == '<' || c == '>');

				let outer_type_name = s.next().ok_or(LexError::Undefined)?;

				let outer_type = lex_parsed_node(None, parser_program.types.get(outer_type_name).ok_or(LexError::NoSuchType{ type_name: outer_type_name.to_string() })?, parser_program, program)?;

				let inner_type_name = s.next().ok_or(LexError::Undefined)?;

				let inner_type = if let Some(stripped) = inner_type_name.strip_suffix('*') {
					let x = Node::internal_new(
						Node {
							parent: None,
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
					lex_parsed_node(parent_node.clone(), parser_program.types.get(inner_type_name).ok_or(LexError::NoSuchType{ type_name: inner_type_name.to_string() })?, parser_program, program)?
				};

				if let Some(n) = program.types.get(r#type) { // If the type already exists, return it.
					return Ok(n.clone());
				}

				let children = Vec::new();

				// for field in fields {
				// 	children.push(lex_parsed_node(&field, parser_program, program)?);
				// }

				let struct_node = Node {
					parent: None,
					node: Nodes::Struct {
						name: r#type.clone(),
						template: Some(outer_type.clone()),
						fields: children,
						types: vec![inner_type],
					},
				};

				let node = Node::internal_new(struct_node);

				program.types.insert(r#type.clone(), node.clone());

				node
			} else {
				let t = parser_program.types.get(r#type.as_str()).ok_or(LexError::NoSuchType{ type_name: r#type.clone() })?;
				lex_parsed_node(None, t, parser_program, program)?
			};

			Node::member(name.clone(), t,)
		}
		parser::Nodes::Function { name, return_type, statements, raw, .. } => {
			let t = parser_program.types.get(return_type.as_str()).ok_or(LexError::NoSuchType{ type_name: return_type.clone() })?;
			let t = lex_parsed_node(None, t, parser_program, program)?;

			let this = Node::function(parent_node.clone(), name.clone(), Vec::new(), t, Vec::new(), raw.clone(),);

			let st = statements.iter().map(|statement| {
				lex_parsed_node(Some(Rc::downgrade(&this.0)), statement, parser_program, program)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;

			match RefCell::borrow_mut(&this).node {
				Nodes::Function { ref mut statements, .. } => {
					statements.extend(st);
				}
				_ => {}
			}

			this
		}
		parser::Nodes::Expression(expression) => {
			match expression {
				parser::Expressions::Accessor{ left, right } => {
					Node::expression(Expressions::Accessor {
							left: lex_parsed_node(None, left, parser_program, program)?,
							right: lex_parsed_node(None, right, parser_program, program)?,
						})
				}
				parser::Expressions::Member{ name } => {
					Node::expression(Expressions::Member {
							name: name.clone(),
						})
				}
				parser::Expressions::Literal{ value } => {
					Node::expression(Expressions::Literal {
							value: value.clone(),
						})
				}
				parser::Expressions::FunctionCall{ name, parameters } => {
					let t = parser_program.types.get(name.as_str()).ok_or(LexError::NoSuchType{ type_name: name.clone() })?;
					let function = lex_parsed_node(None, t, parser_program, program)?;
					Node::expression(Expressions::FunctionCall {
						function,
						name: name.clone(),
						parameters: parameters.iter().map(|e| lex_parsed_node(None, e, parser_program, program).unwrap()).collect(),
					})
				}
				parser::Expressions::Operator{ name, left, right } => {
					Node::expression(Expressions::Operator {
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
							left: lex_parsed_node(None, left, parser_program, program)?,
							right: lex_parsed_node(None, right, parser_program, program)?,
						})
				}
				parser::Expressions::VariableDeclaration{ name, r#type } => {
					Node::expression(Expressions::VariableDeclaration {
							name: name.clone(),
							// r#type: lex_parsed_node(&r#type, parser_program, program)?,
							r#type: r#type.clone(),
						})
				}
			}
		}
	};

	Ok(node)
}

#[cfg(test)]
mod tests {
	use crate::tokenizer;

	use super::*;

	fn assert_type(node: &Node, type_name: &str) {
		match &node.node {
			Nodes::Struct { name, .. } => {
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
		let node = lex(&node, &program).expect("Failed to lex");
		let node = node.borrow();

		assert!(node.parent().is_none());

		match &node.node {
			Nodes::Scope{ children, .. } => {
				let main = children[0].borrow();

				// assert_eq!(main.node(), node.node());

				match main.node() {
					Nodes::Function { name, params: _, return_type, statements, raw: _ } => {
						assert_eq!(name, "main");
						assert_type(&return_type.borrow(), "void");

						let position = statements[0].borrow();

						match position.node() {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								let position = left.borrow();

								assert_eq!(operator, &Operators::Assignment);

								match position.node() {
									Nodes::Expression(Expressions::VariableDeclaration{ name, r#type }) => {
										assert_eq!(name, "position");
										
										// assert_type(&r#type, "vec4f");
										assert_eq!(r#type, "vec4f");
									}
									_ => { panic!("Expected expression"); }
								}

								let constructor = right.borrow();

								match constructor.node() {
									Nodes::Expression(Expressions::FunctionCall{ name, parameters, .. }) => {
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
		let node = lex(&node, &program).expect("Failed to lex");
		let node = node.borrow();

		match node.node() {
			Nodes::Scope{ name, children } => {
				assert_eq!(name, "root");

				let color = children[0].borrow();

				match color.node() {
					Nodes::Member { name, r#type } => {
						assert_eq!(name, "color");						
						assert_type(&r#type.borrow(), "In<vec4f>");
					}
					_ => { panic!("Expected feature"); }
				}
			}
			_ => { panic!("Expected scope"); }
		}
	}

	#[test]
	fn parse_script() {
		let script = r#"
		used: fn () -> void {
			return;
		}

		not_used: fn () -> void {
			return;
		}

		main: fn () -> void {
			used();
		}
		"#;

		let tokens = tokenizer::tokenize(script).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		let node = lex(&node, &program).expect("Failed to lex");
	}

	#[test]
	fn lex_struct() {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}
		"#;

		let tokens = tokenizer::tokenize(script).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		let node = lex(&node, &program).expect("Failed to lex");
		dbg!(&node);

		let node = node.borrow();

		match node.node() {
			Nodes::Scope{ name, children } => {
				assert_eq!(name, "root");

				let vertex = children[0].borrow();

				match vertex.node() {
					Nodes::Struct { name, fields, .. } => {
						assert_eq!(name, "Vertex");
						assert_eq!(fields.len(), 2);
					}
					_ => { panic!("Expected struct"); }
				}
			}
			_ => { panic!("Expected scope"); }
		}
	}
}