use std::{borrow::BorrowMut, cell::RefCell, collections::HashMap, mem::MaybeUninit, num::NonZeroUsize, ops::Deref, rc::{Rc, Weak}};
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
	lex_with_root(Node::root(), node, parser_program)
}

pub(super) fn lex_with_root(root: NodeReference, node: &parser::Node, parser_program: &parser::ProgramState) -> Result<NodeReference, LexError> {
	match &node.node {
		parser::Nodes::Scope { name, children } => {
			assert_eq!(name, "root");

			let ch = children.iter().map(|child| {
				lex_parsed_node(Some(root.clone()), Some(Rc::downgrade(&root.0)), child, parser_program,)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;
			
			RefCell::borrow_mut(&root).add_children(ch);
		
			return Ok(root);
		}
		_ => { return Err(LexError::Undefined); }
	}
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

	/// Creates a root node which is the parent of all other nodes in a program.
	/// Only one root node should exist in a program.
	pub fn root() -> NodeReference {
		let void = Node::r#struct("void", Vec::new());
		let u8_t = Node::r#struct("u8", Vec::new());
		let u16_t = Node::r#struct("u16", Vec::new());
		let u32_t = Node::r#struct("u32", Vec::new());
		let f32_t = Node::r#struct("f32", Vec::new());

		let vec2u16 = Node::r#struct("vec2u16", vec![
			Node::member("x", u16_t.clone()),
			Node::member("y", u16_t.clone()),
		]);

		let vec3f32 = Node::r#struct("vec3f", vec![
			Node::member("x", f32_t.clone()),
			Node::member("y", f32_t.clone()),
			Node::member("z", f32_t.clone()),
		]);
	
		let vec4f32 = Node::r#struct("vec4f", vec![
			Node::member("x", f32_t.clone()),
			Node::member("y", f32_t.clone()),
			Node::member("z", f32_t.clone()),
			Node::member("w", f32_t.clone()),
		]);

		let mat4f32 = Node::r#struct("mat4f32", vec![
			Node::member("x", vec4f32.clone()),
			Node::member("y", vec4f32.clone()),
			Node::member("z", vec4f32.clone()),
			Node::member("w", vec4f32.clone()),
		]);
	
		let root = Node::scope("root".to_string(), vec![
			void,
			u8_t,
			u16_t,
			u32_t,
			f32_t,
			vec2u16,
			vec3f32,
			vec4f32,
			mat4f32
		]);

		root
	}

	pub fn root_with_children(children: Vec<NodeReference>) -> NodeReference {
		let root = Node::root();
		RefCell::borrow_mut(&root).add_children(children);
		root
	}

	/// Creates a scope node which is a logical container for other nodes.
	pub fn scope(name: String, children: Vec<NodeReference>) -> NodeReference {
		let mut node = Node {
			parent: None,
			node: Nodes::Scope{ name, children: Vec::with_capacity(children.len()), program_state: ProgramState { types: HashMap::new(), members: HashMap::new() } },
		};

		node.add_children(children);

		Self::internal_new(node)
	}

	/// Creates a struct node which is a type definition.
	///
	/// # Arguments
	///
	/// * `name` - The name of the struct.
	/// * `fields` - The fields of the struct.
	///
	/// # Returns
	///
	/// The struct node.
	pub fn r#struct(name: &str, fields: Vec<NodeReference>) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Struct {
				name: name.to_string(),
				template: None,
				fields,
				types: Vec::new(),
			},
		})
	}

	pub fn member(name: &str, r#type: NodeReference) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Member {
				name: name.to_string(),
				r#type,
				count: None,
			},
		})
	}

	pub fn array(name: &str, r#type: NodeReference, size: usize) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Member {
				name: name.to_string(),
				r#type,
				count: Some(NonZeroUsize::new(size).expect("Invalid size")),
			},
		})
	}

	pub fn function(name: &str, params: Vec<NodeReference>, return_type: NodeReference, statements: Vec<NodeReference>,) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Function {
				name: name.to_string(),
				params,
				return_type,
				statements,
			},
		})
	}

	pub fn expression(expression: Expressions) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Expression(expression),
		})
	}

	pub fn glsl(code: String, references: Vec<NodeReference>) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::GLSL {
				code,
				references,
			},
		})
	}

	pub fn binding(name: &str, r#type: BindingTypes, set: u32, binding: u32, read: bool, write: bool) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Binding {
				name: name.to_string(),
				r#type,
				set,
				binding,
				read,
				write,
				count: None,
			},
		})
	}

	pub fn binding_array(name: &str, r#type: BindingTypes, set: u32, binding: u32, read: bool, write: bool, count: usize) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::Binding {
				name: name.to_string(),
				r#type,
				set,
				binding,
				read,
				write,
				count: Some(NonZeroUsize::new(count).expect("Invalid count")),
			},
		})
	}

	pub fn push_constant(members: Vec<NodeReference>) -> NodeReference {
		Self::internal_new(Node {
			parent: None,
			node: Nodes::PushConstant {
				members,
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
			Nodes::Scope{ children: c, program_state, .. } => {
				match RefCell::borrow(&child).node() {
					Nodes::Struct { name, .. } => {
						program_state.types.insert(name.clone(), child.clone());
					}
					Nodes::Binding { name, .. } | Nodes::Member { name, .. } => {
						program_state.members.insert(name.clone(), child.clone());
					}
					Nodes::PushConstant { .. } => {
						program_state.members.insert("push_constant".to_string(), child.clone());
					}
					_ => {}
				}

				c.push(child);
			}
			Nodes::Struct { fields, .. } => {
				fields.push(child);
			}
			_ => {}
		}
	}

	pub fn add_children(&mut self, children: Vec<NodeReference>) {
		for child in children {
			self.add_child(child);
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
							Nodes::Struct { name, .. } => {
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

	pub fn get_main(&self) -> Option<NodeReference> {
		if let Some(m) = self.get_child("main") {
			return Some(m);
		} else {
			for child in self.get_children()? {
				if let Some(m) = RefCell::borrow(&child).get_main() {
					return Some(m);
				}
			}
		}

		None
	}
	
	pub fn get_children(&self) -> Option<Vec<NodeReference>> {
		match &self.node {
			Nodes::Scope { children, .. } => {
				Some(children.clone())
			}
			_ => {
				None
			}
		}
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

	pub fn node_mut(&mut self) -> &mut Nodes {
		&mut self.node
	}

	pub fn get_program_state(&self) -> Option<&ProgramState> {
		match &self.node {
			Nodes::Scope { program_state, .. } => {
				Some(program_state)
			}
			_ => {
				None
			}
		}
	}

	pub fn get_program_state_mut(&mut self) -> Option<&mut ProgramState> {
		match &mut self.node {
			Nodes::Scope { program_state, .. } => {
				Some(program_state)
			}
			_ => {
				None
			}
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingTypes {
	Buffer {
		r#type: NodeReference,
	},
	CombinedImageSampler,
	Image {
		format: String,
	},
}

impl BindingTypes {
	pub fn buffer(r#type: NodeReference) -> BindingTypes {
		BindingTypes::Buffer{ r#type }
	}
}

#[derive(Clone, Debug,)]
pub enum Nodes {
	Null,
	Scope {
		name: String,
		children: Vec<NodeReference>,
		program_state: ProgramState,
	},
	Struct {
		name: String,
		template: Option<NodeReference>,
		fields: Vec<NodeReference>,
		types: Vec<NodeReference>,
	},
	Member {
		name: String,
		r#type: NodeReference,
		count: Option<NonZeroUsize>,
	},
	Function {
		name: String,
		params: Vec<NodeReference>,
		return_type: NodeReference,
		statements: Vec<NodeReference>,
	},
	Expression(Expressions),
	GLSL {
		code: String,
		references: Vec<NodeReference>,
	},
	Binding {
		name: String,
		set: u32,
		binding: u32,
		read: bool,
		write: bool,
		r#type: BindingTypes,
		count: Option<NonZeroUsize>,
	},
	PushConstant {
		members: Vec<NodeReference>,
	},
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
	Return,
	Member {
		name: String,
		source: Option<NodeReference>,
	},
	Literal { value: String, },
	FunctionCall {
		function: NodeReference,
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

#[derive(Debug, PartialEq, Eq)]
pub enum LexError {
	Undefined,
	FunctionCallParametersDoNotMatchFunctionParameters,
	AccessingUndeclaredMember {
		name: String,
	},
	ReferenceToUndefinedType {
		type_name: String,
	},
}

#[derive(Clone, Debug)]
pub(crate) struct ProgramState {
	pub(crate) types: HashMap<String, NodeReference>,
	pub(crate) members: HashMap<String, NodeReference>,
}

fn lex_parsed_node(scope: Option<NodeReference>, parent_node: Option<ParentNodeReference>, parser_node: &parser::Node, parser_program: &parser::ProgramState) -> Result<NodeReference, LexError> {
	let node = match &parser_node.node {
		parser::Nodes::Scope{ name, children } => {
			assert_ne!(name, "root"); // The root scope node cannot be an inner part of the program.

			let this = Node::scope(name.clone(), Vec::new());

			let ch = children.iter().map(|child| {
				lex_parsed_node(Some(this.clone()), Some(Rc::downgrade(&this.0)), child, parser_program,)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;
			
			RefCell::borrow_mut(&this).add_children(ch);

			this
		}
		parser::Nodes::Struct { name, fields } => {
			if let Some(n) = RefCell::borrow(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state().ok_or(LexError::Undefined)?.types.get(name) { // If the type already exists, return it.
				return Ok(n.clone());
			}
			
			let this = Node::r#struct(&name, Vec::new());

			let ch = fields.iter().map(|field| {
				lex_parsed_node(scope.clone(), Some(Rc::downgrade(&this.0)), &field, parser_program,)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;

			RefCell::borrow_mut(&this).add_children(ch);

			RefCell::borrow_mut(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state_mut().ok_or(LexError::Undefined)?.types.insert(name.clone(), this.clone());
			RefCell::borrow_mut(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state_mut().ok_or(LexError::Undefined)?.types.insert(format!("{}*", name.clone()), this.clone());

			this
		}
		parser::Nodes::Member { name, r#type } => {
			let t = if r#type.contains('<') {
				let mut s = r#type.split(|c| c == '<' || c == '>');

				let outer_type_name = s.next().ok_or(LexError::Undefined)?;

				let outer_type = lex_parsed_node(scope.clone(), None, parser_program.types.get(outer_type_name).ok_or(LexError::ReferenceToUndefinedType{ type_name: outer_type_name.to_string() })?, parser_program,)?;

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

					RefCell::borrow_mut(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state_mut().ok_or(LexError::Undefined)?.types.insert(format!("{}*", stripped), x.clone());

					x
				} else {					
					lex_parsed_node(scope.clone(), parent_node.clone(), parser_program.types.get(inner_type_name).ok_or(LexError::ReferenceToUndefinedType{ type_name: inner_type_name.to_string() })?, parser_program,)?
				};

				if let Some(n) = RefCell::borrow_mut(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state_mut().ok_or(LexError::Undefined)?.types.get(r#type) { // If the type already exists, return it.
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

				RefCell::borrow_mut(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state_mut().ok_or(LexError::Undefined)?.types.insert(r#type.clone(), node.clone());

				node
			} else {
				let t = parser_program.types.get(r#type.as_str()).ok_or(LexError::ReferenceToUndefinedType{ type_name: r#type.clone() })?;
				lex_parsed_node(scope, None, t, parser_program,)?
			};

			Node::member(&name, t,)
		}
		parser::Nodes::Function { name, return_type, statements, .. } => {
			let t = parser_program.types.get(return_type.as_str()).ok_or(LexError::ReferenceToUndefinedType{ type_name: return_type.clone() })?;
			let t = lex_parsed_node(scope.clone(), None, t, parser_program,)?;

			let this = Node::function(name, Vec::new(), t, Vec::new(),);

			let st = statements.iter().map(|statement| {
				lex_parsed_node(scope.clone(), Some(Rc::downgrade(&this.0)), statement, parser_program,)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;

			match RefCell::borrow_mut(&this).node {
				Nodes::Function { ref mut statements, .. } => {
					statements.extend(st);
				}
				_ => {}
			}

			this
		}
		parser::Nodes::PushConstant { members } => {
			let members = members.iter().map(|member| {
				lex_parsed_node(scope.clone(), parent_node.clone(), member, parser_program,)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;

			Node::push_constant(members)
		}
		parser::Nodes::Binding { name, r#type, set, descriptor, read, write, count } => {
			let r#type = match &r#type.node {
				parser::Nodes::Type { .. } => {
					BindingTypes::buffer(lex_parsed_node(scope.clone(), None, &r#type, parser_program,)?)
				}
				parser::Nodes::Image { format } => {
					BindingTypes::Image { format: format.clone() }
				}
				parser::Nodes::CombinedImageSampler { .. } => {
					BindingTypes::CombinedImageSampler
				}
				_ => { return Err(LexError::Undefined); }
			};

			if let Some(count) = count {
				Node::binding_array(&name, r#type, *set, *descriptor, *read, *write, count.get())
			} else {
				Node::binding(&name, r#type, *set, *descriptor, *read, *write)
			}
		}
		parser::Nodes::Type { name, members } => {
			let this = Node::r#struct(name, Vec::new());

			let ch = members.iter().map(|member| {
				lex_parsed_node(scope.clone(), Some(Rc::downgrade(&this.0)), member, parser_program,)
			}).collect::<Result<Vec<NodeReference>, LexError>>()?;

			RefCell::borrow_mut(&this).add_children(ch);

			this
		}
		parser::Nodes::Image { format } => {
			Node::binding("image", BindingTypes::Image { format: format.clone() }, 0, 0, false, false)
		}
		parser::Nodes::CombinedImageSampler { .. } => {
			Node::binding("combined_image_sampler", BindingTypes::CombinedImageSampler, 0, 0, false, false)
		}
		parser::Nodes::GLSL { code, .. } => {
			Node::glsl(code.clone(), Vec::new())
		}
		parser::Nodes::Expression(expression) => {
			match expression {
				parser::Expressions::Return => {
					Node::expression(Expressions::Return)
				}
				parser::Expressions::Accessor{ left, right } => {
					Node::expression(Expressions::Accessor {
						left: lex_parsed_node(scope.clone(), None, left, parser_program,)?,
						right: lex_parsed_node(scope.clone(), None, right, parser_program,)?,
					})
				}
				parser::Expressions::Member{ name } => {
					Node::expression(Expressions::Member {
						source: Some(RefCell::borrow(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state().ok_or(LexError::Undefined)?.members.get(name).ok_or(LexError::AccessingUndeclaredMember{ name: name.clone() })?.clone()),
						name: name.clone(),
					})
				}
				parser::Expressions::Literal{ value } => {
					Node::expression(Expressions::Literal {
						value: value.clone(),
					})
				}
				parser::Expressions::FunctionCall{ name, parameters } => {
					let t = parser_program.types.get(name.as_str()).ok_or(LexError::ReferenceToUndefinedType{ type_name: name.clone() })?;
					let function = lex_parsed_node(scope.clone(), None, t, parser_program,)?;
					let parameters = parameters.iter().map(|e| lex_parsed_node(scope.clone(), None, e, parser_program,)).collect::<Result<Vec<NodeReference>, LexError>>()?;

					{ // Validate function call
						let function = RefCell::borrow(&function.0);
						let function = function.node();

						match function {
							Nodes::Function { params, .. } => {
								if params.len() != parameters.len() { return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters); }
							}
							Nodes::Struct { fields, .. } => {
								if parameters.len() != fields.len() { return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters); }
							}
							_ => { panic!("Expected function"); }
						}
					}

					Node::expression(Expressions::FunctionCall {
						function,
						parameters,
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
						left: lex_parsed_node(scope.clone(), None, left, parser_program,)?,
						right: lex_parsed_node(scope.clone(), None, right, parser_program,)?,
					})
				}
				parser::Expressions::VariableDeclaration{ name, r#type } => {
					RefCell::borrow(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state().ok_or(LexError::Undefined)?.types.get(r#type).ok_or(LexError::ReferenceToUndefinedType{ type_name: r#type.clone() })?;
					let this = Node::expression(Expressions::VariableDeclaration {
						name: name.clone(),
						r#type: r#type.clone(),
					});

					RefCell::borrow_mut(&scope.clone().ok_or(LexError::Undefined)?.0).get_program_state_mut().ok_or(LexError::Undefined)?.members.insert(name.clone(), this.clone());

					this
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
	fn lex_non_existant_function_struct_member_type() {
		let source = "
Foo: struct {
	bar: NonExistantType
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		lex(&node, &program).err().filter(|e| e == &LexError::ReferenceToUndefinedType{ type_name: "NonExistantType".to_string() }).expect("Expected error");
	}

	#[test]
	fn lex_non_existant_function_return_type() {
		let source = "
main: fn () -> NonExistantType {}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		lex(&node, &program).err().filter(|e| e == &LexError::ReferenceToUndefinedType{ type_name: "NonExistantType".to_string() }).expect("Expected error");
	}

	#[test]
	fn lex_wrong_parameter_count() {
		let source = "
function: fn () -> void {}
main: fn () -> void {
	function(vec3f(1.0, 1.0, 1.0), vec3f(0.0, 0.0, 0.0));
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		lex(&node, &program).err().filter(|e| e == &LexError::FunctionCallParametersDoNotMatchFunctionParameters).expect("Expected error");
	}

	#[test]
	fn lex_function() {
		let source = "
main: fn () -> void {
	position: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	position = position;
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		let node = lex(&node, &program).expect("Failed to lex");
		let node = node.borrow();

		assert!(node.parent().is_none());

		match &node.node {
			Nodes::Scope{ .. } => {
				let main = node.get_child("main").expect("Expected main");
				let main = RefCell::borrow(&main.0);

				match main.node() {
					Nodes::Function { name, return_type, statements, .. } => {
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
									Nodes::Expression(Expressions::FunctionCall{ function, parameters, .. }) => {
										let function = RefCell::borrow(&function.0);
										let name = function.get_name().expect("Expected name");

										assert_eq!(name, "vec4f");
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
	#[ignore]
	fn lex_member() {
		let source = "
color: In<vec4f>;
";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		let node = lex(&node, &program).expect("Failed to lex");
		let node = node.borrow();

		match node.node() {
			Nodes::Scope{ name, children, .. } => {
				assert_eq!(name, "root");

				let color = children[0].borrow();

				match color.node() {
					Nodes::Member { name, r#type, .. } => {
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
		lex(&node, &program).expect("Failed to lex");
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

		let node = node.borrow();

		match node.node() {
			Nodes::Scope{ name, .. } => {
				assert_eq!(name, "root");

				let vertex = node.get_child("Vertex").expect("Expected Vertex");
				let vertex = RefCell::borrow(&vertex.0);

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

	// #[test]
	// fn push_constant() {
	// }

	#[test]
	fn fragment_shader() {
		let source = r#"
		main: fn () -> void {
			albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let (node, program) = parser::parse(tokens).expect("Failed to parse");
		let node = lex(&node, &program).expect("Failed to lex");

		let node = node.borrow();

		match node.node() {
			Nodes::Scope{ name, .. } => {
				assert_eq!(name, "root");

				let main = node.get_child("main").expect("Expected main");
				let main = RefCell::borrow(&main.0);

				match main.node() {
					Nodes::Function { name, return_type, statements, .. } => {
						assert_eq!(name, "main");
						assert_type(&return_type.borrow(), "void");

						let albedo = statements[0].borrow();

						match albedo.node() {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								let albedo = left.borrow();

								assert_eq!(operator, &Operators::Assignment);

								match albedo.node() {
									Nodes::Expression(Expressions::VariableDeclaration{ name, r#type }) => {
										assert_eq!(name, "albedo");
										assert_eq!(r#type, "vec3f");
									}
									_ => { panic!("Expected expression"); }
								}

								let constructor = right.borrow();

								match constructor.node() {
									Nodes::Expression(Expressions::FunctionCall{ function, parameters, .. }) => {
										let function = RefCell::borrow(&function.0);
										let name = function.get_name().expect("Expected name");

										assert_eq!(name, "vec3f");
										assert_eq!(parameters.len(), 3);
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
}