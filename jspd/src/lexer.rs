use std::{borrow::BorrowMut, cell::RefCell, collections::HashMap, num::NonZeroUsize, ops::Deref, rc::{Rc, Weak}};
use std::hash::Hash;

use super::parser;

pub type ParentNodeReference = Weak<RefCell<Node>>;

#[derive(Clone, Debug)]
pub struct NodeReference(Rc<RefCell<Node>>);

impl NodeReference {
	pub fn new<F, E>(f: F) -> Result<NodeReference, E> where F: FnOnce(ParentNodeReference) -> Result<Node, E> {
		let mut error = None;

		let node = Rc::new_cyclic(|r| {
			match f(r.clone()) {
				Ok(node) => {
					RefCell::new(node)
				}
				Err(e) => {
					error = Some(e);
					RefCell::new(Node::root())
				}
			}
		});

		if let Some(e) = error {
			Err(e)
		} else {
			Ok(NodeReference(node))
		}
	}
}

impl From<Node> for NodeReference {
	fn from(node: Node) -> Self {
		NodeReference(Rc::new(RefCell::new(node)))
	}
}

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

pub(super) fn lex_with_root(mut root: Node, node: &parser::Node, parser_program: &parser::ProgramState) -> Result<NodeReference, LexError> {
	match &node.node {
		parser::Nodes::Scope { name, children } => {
			assert_eq!(name, "root");

			for child in children {
				lex_parsed_node(&mut root, None, child, parser_program,)?;
			}
		
			return Ok(root.into());
		}
		_ => { return Err(LexError::Undefined{ message: None }); }
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
	pub fn root() -> Node {
		let void: NodeReference = Node::r#struct("void", Vec::new()).into();
		let u8_t: NodeReference = Node::r#struct("u8", Vec::new()).into();
		let u16_t: NodeReference = Node::r#struct("u16", Vec::new()).into();
		let u32_t: NodeReference = Node::r#struct("u32", Vec::new()).into();
		let f32_t: NodeReference = Node::r#struct("f32", Vec::new()).into();

		let vec2u16: NodeReference = Node::r#struct("vec2u16", vec![
			Node::member("x", u16_t.clone()).into(),
			Node::member("y", u16_t.clone()).into(),
		]).into();

		let vec3f32: NodeReference = Node::r#struct("vec3f", vec![
			Node::member("x", f32_t.clone()).into(),
			Node::member("y", f32_t.clone()).into(),
			Node::member("z", f32_t.clone()).into(),
		]).into();
	
		let vec4f32: NodeReference = Node::r#struct("vec4f", vec![
			Node::member("x", f32_t.clone()).into(),
			Node::member("y", f32_t.clone()).into(),
			Node::member("z", f32_t.clone()).into(),
			Node::member("w", f32_t.clone()).into(),
		]).into();

		let mat4f32: NodeReference = Node::r#struct("mat4f32", vec![
			Node::member("x", vec4f32.clone()).into(),
			Node::member("y", vec4f32.clone()).into(),
			Node::member("z", vec4f32.clone()).into(),
			Node::member("w", vec4f32.clone()).into(),
		]).into();
	
		let mut root = Node::scope("root".to_string());
		
		for c in vec![
			void,
			u8_t,
			u16_t,
			u32_t,
			f32_t,
			vec2u16,
			vec3f32,
			vec4f32,
			mat4f32
		] {
			// root.add_child(c.clone(), c.clone());
		}

		root
	}

	/// Creates a scope node which is a logical container for other nodes.
	pub fn scope(name: String,) -> Node {
		let mut node = Node {
			parent: None,
			node: Nodes::Scope{ name, children: Vec::with_capacity(16), program_state: ProgramState { types: HashMap::new(), members: HashMap::new() } },
		};

		node
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
	pub fn r#struct(name: &str, fields: Vec<NodeReference>) -> Node {
		Node {
			parent: None,
			node: Nodes::Struct {
				name: name.to_string(),
				template: None,
				fields,
				types: Vec::new(),
			},
		}
	}

	pub fn member(name: &str, r#type: NodeReference) -> Node {
		Node {
			parent: None,
			node: Nodes::Member {
				name: name.to_string(),
				r#type,
				count: None,
			},
		}
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

	pub fn function(name: &str, params: Vec<NodeReference>, return_type: NodeReference, statements: Vec<NodeReference>,) -> Node {
		Node {
			parent: None,
			node: Nodes::Function {
				name: name.to_string(),
				params,
				return_type,
				statements,
			},
		}
	}

	pub fn expression(expression: Expressions) -> Node {
		Node {
			parent: None,
			node: Nodes::Expression(expression),
		}
	}

	pub fn glsl(code: String, references: Vec<NodeReference>) -> Node {
		Node {
			parent: None,
			node: Nodes::GLSL {
				code,
				references,
			},
		}
	}

	pub fn binding(name: &str, r#type: BindingTypes, set: u32, binding: u32, read: bool, write: bool) -> Node {
		Node {
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
		}
	}

	pub fn binding_array(name: &str, r#type: BindingTypes, set: u32, binding: u32, read: bool, write: bool, count: usize) -> Node {
		Node {
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
		}
	}

	pub fn push_constant(members: Vec<NodeReference>) -> Node {
		Node {
			parent: None,
			node: Nodes::PushConstant {
				members,
			},
		}
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

	pub fn add_child(&mut self, parent: ParentNodeReference, mut child: Node) -> NodeReference {
		child.parent = Some(parent);

		let child: NodeReference = child.into();

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

				c.push(child.clone());
			}
			Nodes::Struct { fields, .. } => {
				fields.push(child.clone());
			}
			Nodes::Function { statements, .. } => {
				statements.push(child.clone());
			}
			_ => {}
		}

		child
	}

	pub fn add_children(&mut self, parent: ParentNodeReference, children: Vec<Node>) -> Vec<NodeReference> {
		let mut ch = Vec::with_capacity(children.len());

		for child in children {
			ch.push(self.add_child(parent.clone(), child));
		}

		ch
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

	// Traverses the tree to find a reference to a node with the given name.
	pub fn get_reference(&self, name: &str) -> Option<NodeReference> {
		self.get_program_state()?.members.get(name).or_else(|| self.get_program_state()?.types.get(name)).map(|r| r.clone()).or(self.parent()?.upgrade()?.borrow().get_reference(name))
	}

	pub fn node_mut(&mut self) -> &mut Nodes {
		&mut self.node
	}

	fn get_program_state(&self) -> Option<&ProgramState> {
		match &self.node {
			Nodes::Scope { program_state, .. } => {
				Some(program_state)
			}
			_ => {
				None
			}
		}
	}

	fn get_program_state_mut(&mut self) -> Option<&mut ProgramState> {
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
	Undefined {
		message: Option<String>,
	},
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

fn lex_parsed_node(parent_node: &mut Node, parent_node_reference: Option<ParentNodeReference>, parser_node: &parser::Node, parser_program: &parser::ProgramState) -> Result<NodeReference, LexError> {
	let node = match &parser_node.node {
		parser::Nodes::Scope{ name, children } => {
			assert_ne!(name, "root"); // The root scope node cannot be an inner part of the program.

			let mut this = Node::scope(name.clone());

			NodeReference::new(move |p| {
				for child in children {
					lex_parsed_node(&mut this, Some(p.clone()), child, parser_program,)?;
				}

				Ok(parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this))
			})?
		}
		parser::Nodes::Struct { name, fields } => {
			if let Some(n) = parent_node.get_reference(name) { // If the type already exists, return it.
				return Ok(n.clone());
			}
			
			let mut this = Node::r#struct(&name, Vec::new());

			for field in fields {
				lex_parsed_node(&mut this, None, &field, parser_program,)?;
			}

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::Member { name, r#type } => {
			let t = if r#type.contains('<') {
				let mut s = r#type.split(|c| c == '<' || c == '>');

				let outer_type_name = s.next().ok_or(LexError::Undefined{ message: Some("No outer name".to_string()) })?;

				let outer_type = lex_parsed_node(parent_node, None, parser_program.types.get(outer_type_name).ok_or(LexError::ReferenceToUndefinedType{ type_name: outer_type_name.to_string() })?, parser_program,)?;

				let inner_type_name = s.next().ok_or(LexError::Undefined{ message: Some("No inner name".to_string()) })?;

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

					x
				} else {					
					lex_parsed_node(parent_node, parent_node_reference.clone(), parser_program.types.get(inner_type_name).ok_or(LexError::ReferenceToUndefinedType{ type_name: inner_type_name.to_string() })?, parser_program,)?
				};

				if let Some(n) = parent_node.get_reference(r#type) { // If the type already exists, return it.
					return Ok(n.clone());
				}

				let children = Vec::new();

				let this = Node {
					parent: None,
					node: Nodes::Struct {
						name: r#type.clone(),
						template: Some(outer_type.clone()),
						fields: children,
						types: vec![inner_type],
					},
				};

				let this: NodeReference = this.into();

				// parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this);

				return Ok(this);
			} else if r#type.contains('[') {
				let mut s = r#type.split(|c| c == '[' || c == ']');

				let type_name = s.next().ok_or(LexError::Undefined{ message: Some("No type name".to_string()) })?;

				let member_type = lex_parsed_node(parent_node, None, parser_program.types.get(type_name).ok_or(LexError::ReferenceToUndefinedType{ type_name: type_name.to_string() })?, parser_program,)?;

				let count = s.next().ok_or(LexError::Undefined{ message: Some("No count".to_string()) })?.parse().map_err(|_| LexError::Undefined{ message: Some("Invalid count".to_string()) })?;

				Node::array(&name, member_type, count)
			} else {
				lex_parsed_node(parent_node, None, parser_program.types.get(r#type.as_str()).ok_or(LexError::ReferenceToUndefinedType{ type_name: r#type.clone() })?, parser_program,)?
			};

			let this = Node::member(&name, t,);

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::Function { name, return_type, statements, .. } => {
			let t = parser_program.types.get(return_type.as_str()).ok_or(LexError::ReferenceToUndefinedType{ type_name: return_type.clone() })?;
			let t = lex_parsed_node(parent_node, None, t, parser_program,)?;

			let mut this = Node::function(name, Vec::new(), t, Vec::new(),);

			for statement in statements {
				lex_parsed_node(&mut this, None, statement, parser_program,)?;
			}

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::PushConstant { members } => {
			let mut this = Node::push_constant(vec![]);

			for member in members {
				if let parser::Nodes::Member { r#type, .. } = &member.node {
					let t = parser_program.types.get(r#type.as_str()).ok_or(LexError::ReferenceToUndefinedType{ type_name: r#type.clone() })?;
					lex_parsed_node(&mut this, None, t, parser_program,)?;
				}
			}

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::Binding { name, r#type, set, descriptor, read, write, count } => {
			let r#type = match &r#type.node {
				parser::Nodes::Type { .. } => {
					BindingTypes::buffer(lex_parsed_node(parent_node, None, &r#type, parser_program,)?)
				}
				parser::Nodes::Image { format } => {
					BindingTypes::Image { format: format.clone() }
				}
				parser::Nodes::CombinedImageSampler { .. } => {
					BindingTypes::CombinedImageSampler
				}
				_ => { return Err(LexError::Undefined{ message: Some("Invalid binding type".to_string()) }); }
			};

			let this = if let Some(count) = count {
				Node::binding_array(&name, r#type, *set, *descriptor, *read, *write, count.get())
			} else {
				Node::binding(&name, r#type, *set, *descriptor, *read, *write)
			};

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::Type { name, members } => {
			let mut this = Node::r#struct(name, Vec::new());

			for member in members {
				lex_parsed_node(&mut this, None, member, parser_program,)?;
			}

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::Image { format } => {
			let this = Node::binding("image", BindingTypes::Image { format: format.clone() }, 0, 0, false, false);

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::CombinedImageSampler { .. } => {
			let this = Node::binding("combined_image_sampler", BindingTypes::CombinedImageSampler, 0, 0, false, false);

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::GLSL { code, input, .. } => {
			let mut references = Vec::new();

			for i in input {
				references.push(parent_node.get_reference(i).ok_or(LexError::AccessingUndeclaredMember { name: i.clone() })?.clone());
			}

			let this = Node::glsl(code.clone(), references);

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
		}
		parser::Nodes::Expression(expression) => {
			let this = match expression {
				parser::Expressions::Return => {
					Node::expression(Expressions::Return)
				}
				parser::Expressions::Accessor{ left, right } => {
					Node::expression(Expressions::Accessor {
						left: lex_parsed_node(parent_node, None, left, parser_program,)?,
						right: lex_parsed_node(parent_node, None, right, parser_program,)?,
					})
				}
				parser::Expressions::Member{ name } => {
					Node::expression(Expressions::Member {
						source: Some(parent_node.get_reference(name).ok_or(LexError::AccessingUndeclaredMember{ name: name.clone() })?.clone()),
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
					let function = lex_parsed_node(parent_node, None, t, parser_program,)?;
					let parameters = parameters.iter().map(|e| lex_parsed_node(parent_node, None, e, parser_program,)).collect::<Result<Vec<NodeReference>, LexError>>()?;

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
						left: lex_parsed_node(parent_node, None, left, parser_program,)?,
						right: lex_parsed_node(parent_node, None, right, parser_program,)?,
					})
				}
				parser::Expressions::VariableDeclaration{ name, r#type } => {
					parent_node.get_reference(r#type).ok_or(LexError::ReferenceToUndefinedType{ type_name: r#type.clone() })?;
					let this = Node::expression(Expressions::VariableDeclaration {
						name: name.clone(),
						r#type: r#type.clone(),
					});

					this
				}
			};

			parent_node.add_child(parent_node_reference.ok_or(LexError::Undefined { message: None })?, this)
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