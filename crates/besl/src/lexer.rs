use std::hash::Hash;
use std::{
	cell::RefCell,
	num::NonZeroUsize,
	ops::Deref,
	rc::{Rc, Weak},
};

use super::parser;

pub type ParentNodeReference = Weak<RefCell<Node>>;

#[derive(Clone)]
pub struct NodeReference(Rc<RefCell<Node>>);

impl std::fmt::Debug for NodeReference {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.borrow().fmt(f)
	}
}

impl NodeReference {
	pub fn new<F, E>(f: F) -> Result<NodeReference, E>
	where
		F: FnOnce(ParentNodeReference) -> Result<Node, E>,
	{
		let mut error = None;

		let node = Rc::new_cyclic(|r| match f(r.clone()) {
			Ok(node) => RefCell::new(node),
			Err(e) => {
				error = Some(e);
				RefCell::new(Node::root())
			}
		});

		if let Some(e) = error {
			Err(e)
		} else {
			Ok(NodeReference(node))
		}
	}

	/// Recursively searches for a child node with the given name.
	pub fn get_descendant(&self, child_name: &str) -> Option<NodeReference> {
		if self.borrow().get_name() == Some(child_name) {
			return Some(self.clone());
		}

		match &self.borrow().node {
			Nodes::Scope { children: members, .. }
			| Nodes::Struct { fields: members, .. }
			| Nodes::PushConstant { members }
			| Nodes::Intrinsic { elements: members, .. } => {
				for member in members {
					if let Some(c) = member.get_descendant(child_name) {
						return Some(c);
					}
				}
			}
			Nodes::Member { r#type, .. } | Nodes::Parameter { r#type, .. } => {
				if let Some(c) = r#type.get_descendant(child_name) {
					return Some(c);
				}
			}
			Nodes::Function { params, statements, .. } => {
				for param in params {
					if param.borrow().get_name() == Some(child_name) {
						return Some(param.clone());
					}
				}

				for statement in statements {
					match RefCell::borrow(&statement).node() {
						Nodes::Expression(expression) => match expression {
							Expressions::Operator { left, right, .. } => {
								if let Some(c) = left.get_descendant(child_name) {
									return Some(c);
								}
								if let Some(c) = right.get_descendant(child_name) {
									return Some(c);
								}
							}
							Expressions::VariableDeclaration { name, .. } => {
								if child_name == name {
									return Some(statement.clone());
								}
							}
							Expressions::Accessor { left, right } => {
								if let Some(c) = left.get_descendant(child_name) {
									return Some(c);
								}
								if let Some(c) = right.get_descendant(child_name) {
									return Some(c);
								}
							}
							Expressions::Return { value } => {
								if let Some(value) = value {
									if let Some(c) = value.get_descendant(child_name) {
										return Some(c);
									}
								}
							}
							_ => {}
						},
						Nodes::Raw { output, .. } => {
							for o in output {
								if let Some(c) = o.get_descendant(child_name) {
									return Some(c);
								}
							}
						}
						_ => {}
					}
				}
			}
			Nodes::Expression(expression) => match expression {
				Expressions::Operator { left, right, .. } => {
					if let Some(c) = left.get_descendant(child_name) {
						return Some(c);
					}
					if let Some(c) = right.get_descendant(child_name) {
						return Some(c);
					}
				}
				Expressions::Member { source, .. } => {
					if let Some(c) = source.get_descendant(child_name) {
						return Some(c);
					}
				}
				Expressions::Expression { elements } => {
					for e in elements {
						if let Some(c) = e.get_descendant(child_name) {
							return Some(c);
						}
					}
				}
				Expressions::VariableDeclaration { r#type, .. } => {
					if let Some(c) = r#type.get_descendant(child_name) {
						return Some(c);
					}
				}
				Expressions::Accessor { left, right } => {
					if let Some(c) = right.get_descendant(child_name) {
						return Some(c);
					}
					if let Some(c) = left.get_descendant(child_name) {
						return Some(c);
					}
				}
				Expressions::IntrinsicCall { intrinsic, .. } => {
					let intrinsic = intrinsic.borrow();
					match intrinsic.node() {
						Nodes::Intrinsic { r#return, .. } => {
							if let Some(c) = r#return.get_descendant(child_name) {
								return Some(c);
							}
						}
						_ => {}
					}
				}
				Expressions::Return { value } => {
					if let Some(value) = value {
						if let Some(c) = value.get_descendant(child_name) {
							return Some(c);
						}
					}
				}
				_ => {}
			},
			Nodes::Raw { output, .. } => {
				for o in output {
					if let Some(c) = o.get_descendant(child_name) {
						return Some(c);
					}
				}
			}
			Nodes::Binding { r#type, .. } => {
				if let BindingTypes::Buffer { members } = r#type {
					for member in members {
						if let Some(c) = member.get_descendant(child_name) {
							return Some(c);
						}
					}
				}
			}
			Nodes::Input { format, .. } | Nodes::Output { format, .. } => {
				if let Some(c) = format.get_descendant(child_name) {
					return Some(c);
				}
			}
			_ => {}
		}

		None
	}

	pub fn get_children(&self) -> Option<Vec<NodeReference>> {
		self.borrow().get_children()
	}

	/// Returns the main function of the program.
	pub fn get_main(&self) -> Option<NodeReference> {
		if let Some(m) = self.get_descendant("main") {
			return Some(m);
		} else {
			for child in self.get_children()? {
				if let Some(m) = child.get_main() {
					return Some(m);
				}
			}
		}

		None
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
	fn hash<H>(&self, state: &mut H)
	where
		H: std::hash::Hasher,
	{
		Rc::as_ptr(&self.0).hash(state);
	}
}

impl Deref for NodeReference {
	type Target = RefCell<Node>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

pub(super) fn lex(mut node: parser::Node) -> Result<NodeReference, LexError> {
	node.sort();
	lex_with_root(Node::root(), node)
}

pub(super) fn lex_with_root(root: Node, mut node: parser::Node) -> Result<NodeReference, LexError> {
	node.sort();

	let root: NodeReference = root.into();

	match &node.node {
		parser::Nodes::Scope { name, children } => {
			assert_eq!(*name, "root");

			for child in children {
				let c = lex_parsed_node(vec![root.clone()], child)?;
				root.borrow_mut().add_child(c);
			}

			return Ok(root);
		}
		_ => {
			return Err(LexError::Undefined { message: None });
		}
	}
}

#[derive(Clone)]
pub struct Node {
	// parent: Option<ParentNodeReference>,
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
		let i32_t: NodeReference = Node::r#struct("i32", Vec::new()).into();
		let f32_t: NodeReference = Node::r#struct("f32", Vec::new()).into();

		let vec2u16: NodeReference = Node::r#struct(
			"vec2u16",
			vec![
				Node::member("x", u16_t.clone()).into(),
				Node::member("y", u16_t.clone()).into(),
			],
		)
		.into();

		let vec2u32: NodeReference = Node::r#struct(
			"vec2u",
			vec![
				Node::member("x", u32_t.clone()).into(),
				Node::member("y", u32_t.clone()).into(),
			],
		)
		.into();

		let vec2i32: NodeReference = Node::r#struct(
			"vec2i",
			vec![
				Node::member("x", i32_t.clone()).into(),
				Node::member("y", i32_t.clone()).into(),
			],
		)
		.into();

		let vec2f32: NodeReference = Node::r#struct(
			"vec2f",
			vec![
				Node::member("x", f32_t.clone()).into(),
				Node::member("y", f32_t.clone()).into(),
			],
		)
		.into();

		let vec3f32: NodeReference = Node::r#struct(
			"vec3f",
			vec![
				Node::member("x", f32_t.clone()).into(),
				Node::member("y", f32_t.clone()).into(),
				Node::member("z", f32_t.clone()).into(),
			],
		)
		.into();

		let vec3u32: NodeReference = Node::r#struct(
			"vec3u",
			vec![
				Node::member("x", u32_t.clone()).into(),
				Node::member("y", u32_t.clone()).into(),
				Node::member("z", u32_t.clone()).into(),
			],
		)
		.into();

		let vec4f32: NodeReference = Node::r#struct(
			"vec4f",
			vec![
				Node::member("x", f32_t.clone()).into(),
				Node::member("y", f32_t.clone()).into(),
				Node::member("z", f32_t.clone()).into(),
				Node::member("w", f32_t.clone()).into(),
			],
		)
		.into();

		let mat4f32: NodeReference = Node::r#struct(
			"mat4f",
			vec![
				Node::member("x", vec4f32.clone()).into(),
				Node::member("y", vec4f32.clone()).into(),
				Node::member("z", vec4f32.clone()).into(),
				Node::member("w", vec4f32.clone()).into(),
			],
		)
		.into();

		let texture_2d: NodeReference = Node::r#struct("Texture2D", vec![]).into();
		let array_texture_2d: NodeReference = Node::r#struct("ArrayTexture2D", vec![]).into();
		let sample_intrinsic = builtin_intrinsic(
			"sample",
			vec![("texture_sampler", texture_2d.clone()), ("uv", vec2f32.clone())],
			vec4f32.clone(),
		);
		let fetch_intrinsic = builtin_intrinsic(
			"fetch",
			vec![("texture", texture_2d.clone()), ("coord", vec2u32.clone())],
			vec4f32.clone(),
		);
		let dot_intrinsic = builtin_intrinsic(
			"dot",
			vec![("left", vec4f32.clone()), ("right", vec4f32.clone())],
			f32_t.clone(),
		);
		let write_intrinsic = builtin_intrinsic(
			"write",
			vec![
				("image", texture_2d.clone()),
				("coord", vec2u32.clone()),
				("value", vec4f32.clone()),
			],
			void.clone(),
		);

		let mut root = Node::scope("root".to_string());

		root.add_children(vec![
			void,
			u8_t,
			u16_t,
			u32_t,
			i32_t,
			f32_t,
			vec2u16,
			vec2u32,
			vec2i32,
			vec2f32,
			vec3u32,
			vec3f32,
			vec4f32,
			mat4f32,
			texture_2d,
			array_texture_2d,
			sample_intrinsic,
			fetch_intrinsic,
			dot_intrinsic,
			write_intrinsic,
		]);

		root
	}

	/// Creates a scope node which is a logical container for other nodes.
	pub fn scope(name: String) -> Node {
		let node = Node {
			// parent: None,
			node: Nodes::Scope {
				name,
				children: Vec::with_capacity(16),
			},
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
			node: Nodes::Member {
				name: name.to_string(),
				r#type,
				count: None,
			},
		}
	}

	pub fn array(name: &str, r#type: NodeReference, size: usize) -> NodeReference {
		Self::internal_new(Node {
			node: Nodes::Member {
				name: name.to_string(),
				r#type,
				count: Some(NonZeroUsize::new(size).expect("Invalid size")),
			},
		})
	}

	pub fn function(
		name: &str,
		params: Vec<NodeReference>,
		return_type: NodeReference,
		statements: Vec<NodeReference>,
	) -> Node {
		Node {
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
			node: Nodes::Expression(expression),
		}
	}

	pub fn glsl(code: String, inputs: Vec<NodeReference>, outputs: Vec<NodeReference>) -> Node {
		Node {
			node: Nodes::Raw {
				glsl: Some(code),
				hlsl: None,
				input: inputs,
				output: outputs,
			},
		}
	}

	pub fn hlsl(code: String, inputs: Vec<NodeReference>, outputs: Vec<NodeReference>) -> Node {
		Node {
			node: Nodes::Raw {
				glsl: None,
				hlsl: Some(code),
				input: inputs,
				output: outputs,
			},
		}
	}

	pub fn raw(glsl: Option<String>, hlsl: Option<String>, inputs: Vec<NodeReference>, outputs: Vec<NodeReference>) -> Node {
		Node {
			node: Nodes::Raw {
				glsl,
				hlsl,
				input: inputs,
				output: outputs,
			},
		}
	}

	pub fn r#macro(name: &str, body: NodeReference) -> Node {
		Node {
			node: Nodes::Expression(Expressions::Macro {
				name: name.to_string(),
				body,
			}),
		}
	}

	pub fn binding(name: &str, r#type: BindingTypes, set: u32, binding: u32, read: bool, write: bool) -> Node {
		Node {
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

	pub fn binding_array(
		name: &str,
		r#type: BindingTypes,
		set: u32,
		binding: u32,
		read: bool,
		write: bool,
		count: usize,
	) -> Node {
		Node {
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
			node: Nodes::PushConstant { members },
		}
	}

	pub fn intrinsic(name: &str, elements: Vec<NodeReference>, r#return: NodeReference) -> Node {
		Node {
			node: Nodes::Intrinsic {
				name: name.to_string(),
				elements,
				r#return,
			},
		}
	}

	pub fn specialization(name: &str, r#type: NodeReference) -> Node {
		Node {
			node: Nodes::Specialization {
				name: name.to_string(),
				r#type,
			},
		}
	}

	pub fn input(name: &str, format: NodeReference, location: u8) -> Node {
		Node {
			node: Nodes::Input {
				name: name.to_string(),
				format,
				location,
			},
		}
	}

	pub fn output(name: &str, format: NodeReference, location: u8) -> Node {
		Node {
			node: Nodes::Output {
				name: name.to_string(),
				format,
				location,
			},
		}
	}

	pub fn new(node: Nodes) -> Node {
		Node { node }
	}

	pub fn add_child(&mut self, child: NodeReference) -> NodeReference {
		match &mut self.node {
			Nodes::Scope { children, .. } => {
				children.push(child.clone());
			}
			Nodes::Struct { fields, .. } => {
				fields.push(child.clone());
			}
			Nodes::Function { statements, .. } => {
				statements.push(child.clone());
			}
			Nodes::PushConstant { members } => {
				members.push(child.clone());
			}
			Nodes::Intrinsic { elements, .. } => {
				elements.push(child.clone());
			}
			_ => {}
		}

		child
	}

	pub fn add_children(&mut self, children: Vec<NodeReference>) -> Vec<NodeReference> {
		let mut ch = Vec::with_capacity(children.len());

		for child in children {
			ch.push(self.add_child(child));
		}

		ch
	}

	pub fn node(&self) -> &Nodes {
		&self.node
	}

	pub fn get_name(&self) -> Option<&str> {
		match &self.node {
			Nodes::Scope { name, .. }
			| Nodes::Function { name, .. }
			| Nodes::Member { name, .. }
			| Nodes::Struct { name, .. }
			| Nodes::Intrinsic { name, .. }
			| Nodes::Binding { name, .. }
			| Nodes::Parameter { name, .. }
			| Nodes::Specialization { name, .. }
			| Nodes::Literal { name, .. } => Some(name),
			Nodes::Input { name, .. } | Nodes::Output { name, .. } => Some(name),
			Nodes::PushConstant { .. } => Some("push_constant"),
			Nodes::Expression(expression) => match expression {
				Expressions::VariableDeclaration { name, .. } | Expressions::Member { name, .. } => Some(name),
				_ => None,
			},
			_ => None,
		}
	}

	pub fn get_children(&self) -> Option<Vec<NodeReference>> {
		match &self.node {
			Nodes::Scope { children, .. }
			| Nodes::Struct { fields: children, .. }
			| Nodes::Intrinsic { elements: children, .. } => Some(children.clone()),
			Nodes::Function { statements, .. } => Some(statements.clone()),
			Nodes::Expression(expression) => match expression {
				Expressions::IntrinsicCall { elements: children, .. } => Some(children.clone()),
				_ => None,
			},
			_ => None,
		}
	}

	pub fn get_child(&self, child_name: &str) -> Option<NodeReference> {
		self.get_children()?
			.iter()
			.find(|child| child.borrow().get_name() == Some(child_name))
			.cloned()
	}

	pub fn node_mut(&mut self) -> &mut Nodes {
		&mut self.node
	}

	pub fn null() -> Node {
		Self { node: Nodes::Null }
	}

	fn sentence(elements: Vec<NodeReference>) -> Node {
		Self {
			node: Nodes::Expression(Expressions::Expression { elements }),
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingTypes {
	Buffer { members: Vec<NodeReference> },
	CombinedImageSampler { format: String },
	Image { format: String },
}

#[derive(Clone)]
pub enum Nodes {
	Null,
	Scope {
		name: String,
		children: Vec<NodeReference>,
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
	Specialization {
		name: String,
		r#type: NodeReference,
	},
	Expression(Expressions),
	Raw {
		glsl: Option<String>,
		hlsl: Option<String>,
		input: Vec<NodeReference>,
		output: Vec<NodeReference>,
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
	Intrinsic {
		name: String,
		elements: Vec<NodeReference>,
		r#return: NodeReference,
	},
	Input {
		name: String,
		format: NodeReference,
		location: u8,
	},
	Output {
		name: String,
		format: NodeReference,
		location: u8,
	},
	Parameter {
		name: String,
		r#type: NodeReference,
	},
	Literal {
		name: String,
		value: NodeReference,
	},
}

impl Nodes {
	pub fn is_leaf(&self) -> bool {
		match self {
			Nodes::Function { .. } => false,
			Nodes::Struct { .. } => false,
			Nodes::Binding { .. } => false,
			Nodes::PushConstant { .. } => false,
			Nodes::Input { .. } | Nodes::Output { .. } => false,
			Nodes::Specialization { .. } => false,
			Nodes::Literal { .. } => true,
			Nodes::Parameter { .. } => true,
			Nodes::Null => true,
			Nodes::Scope { .. } => true,
			Nodes::Intrinsic { .. } => true,
			Nodes::Member { .. } => true,
			Nodes::Expression { .. } => true,
			Nodes::Raw { .. } => true,
		}
	}
}

impl std::fmt::Debug for Node {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self.node {
			Nodes::Null => {
				write!(f, "Null")
			}
			Nodes::Scope { name, children } => {
				write!(
					f,
					"Scope {{ name: {}, children: {:#?} }}",
					name,
					children.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string()))
				)
			}
			Nodes::Struct { name, fields, .. } => {
				write!(
					f,
					"Struct {{ name: {}, fields: {:?} }}",
					name,
					fields.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string()))
				)
			}
			Nodes::Member { name, r#type, .. } => {
				write!(
					f,
					"Member {{ name: {}, type: {:?} }}",
					name,
					r#type.0.borrow().get_name().map(|e| e.to_string())
				)
			}
			Nodes::Function {
				name,
				params,
				statements,
				..
			} => {
				write!(
					f,
					"Function {{ name: {}, parameters: {:?}, statements: {:?} }}",
					name,
					params.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string())),
					statements.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string()))
				)
			}
			Nodes::Specialization { name, r#type } => {
				write!(
					f,
					"Specialization {{ name: {}, type: {:?} }}",
					name,
					r#type.0.borrow().get_name().map(|e| e.to_string())
				)
			}
			Nodes::Expression(expression) => {
				write!(f, "Expression {{ {:?} }}", expression)
			}
			Nodes::Raw {
				glsl,
				hlsl,
				input,
				output,
			} => {
				write!(
					f,
					"RawCode {{ glsl: {:?}, hlsl: {:?}, input: {:?}, output: {:?} }}",
					glsl,
					hlsl,
					input.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string())),
					output.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string()))
				)
			}
			Nodes::Binding {
				name,
				set,
				binding,
				read,
				write,
				r#type,
				count,
			} => {
				write!(
					f,
					"Binding {{ name: {}, set: {}, binding: {}, read: {}, write: {}, type: {:?}, count: {:?} }}",
					name, set, binding, read, write, r#type, count
				)
			}
			Nodes::PushConstant { members } => {
				write!(
					f,
					"PushConstant {{ members: {:?} }}",
					members.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string()))
				)
			}
			Nodes::Intrinsic {
				name,
				elements,
				r#return,
			} => {
				write!(
					f,
					"Intrinsic {{ name: {}, elements: {:?}, return: {:?} }}",
					name,
					elements.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string())),
					r#return.0.borrow().get_name().map(|e| e.to_string())
				)
			}
			Nodes::Parameter { name, r#type } => {
				write!(
					f,
					"Parameter {{ name: {}, type: {:?} }}",
					name,
					r#type.0.borrow().get_name().map(|e| e.to_string())
				)
			}
			Nodes::Input { name, format, location } => {
				write!(
					f,
					"Input {{ name: {}, format: {:?}, location: {} }}",
					name,
					format.0.borrow().get_name().map(|e| e.to_string()),
					location
				)
			}
			Nodes::Output { name, format, location } => {
				write!(
					f,
					"Output {{ name: {}, format: {:?}, location: {} }}",
					name,
					format.0.borrow().get_name().map(|e| e.to_string()),
					location
				)
			}
			Nodes::Literal { name, value } => {
				write!(
					f,
					"Literal {{ name: {}, value: {:?} }}",
					name,
					value.0.borrow().get_name().map(|e| e.to_string())
				)
			}
		}
	}
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
	Return {
		value: Option<NodeReference>,
	},
	Member {
		name: String,
		source: NodeReference,
	},
	Expression {
		elements: Vec<NodeReference>,
	},
	Literal {
		value: String,
	},
	FunctionCall {
		function: NodeReference,
		parameters: Vec<NodeReference>,
	},
	IntrinsicCall {
		intrinsic: NodeReference,
		arguments: Vec<NodeReference>,
		elements: Vec<NodeReference>,
	},
	Operator {
		operator: Operators,
		left: NodeReference,
		right: NodeReference,
	},
	VariableDeclaration {
		name: String,
		r#type: NodeReference,
	},
	Accessor {
		left: NodeReference,
		right: NodeReference,
	},
	Macro {
		name: String,
		body: NodeReference,
	},
}

#[derive(Debug, PartialEq, Eq)]
pub enum LexError {
	Undefined { message: Option<String> },
	FunctionCallParametersDoNotMatchFunctionParameters,
	AccessingUndeclaredMember { name: String },
	ReferenceToUndefinedType { type_name: String },
}

/// Tries to resolve a reference to a node by visiting the chain of nodes which are the context of the element of the program being lexed.
fn get_reference(chain: &[NodeReference], name: &str) -> Option<NodeReference> {
	for node in chain.iter().rev() {
		let reference = match node.borrow().node() {
			Nodes::Intrinsic { .. } => node.get_descendant(name),
			_ => get_non_intrinsic_descendant(node, name),
		};

		if let Some(c) = reference {
			return Some(c);
		}
	}

	None
}

fn get_non_intrinsic_descendant(node: &NodeReference, child_name: &str) -> Option<NodeReference> {
	if node.borrow().get_name() == Some(child_name) {
		return Some(node.clone());
	}

	match &node.borrow().node {
		Nodes::Scope { children: members, .. } | Nodes::Struct { fields: members, .. } | Nodes::PushConstant { members } => {
			for member in members {
				if let Some(c) = get_non_intrinsic_descendant(member, child_name) {
					return Some(c);
				}
			}
		}
		Nodes::Intrinsic { .. } => {}
		Nodes::Member { r#type, .. } | Nodes::Parameter { r#type, .. } => {
			if let Some(c) = get_non_intrinsic_descendant(r#type, child_name) {
				return Some(c);
			}
		}
		Nodes::Function { params, statements, .. } => {
			for param in params {
				if param.borrow().get_name() == Some(child_name) {
					return Some(param.clone());
				}
			}

			for statement in statements {
				match RefCell::borrow(statement).node() {
					Nodes::Expression(expression) => match expression {
						Expressions::Operator { left, right, .. } => {
							if let Some(c) = get_non_intrinsic_descendant(left, child_name) {
								return Some(c);
							}
							if let Some(c) = get_non_intrinsic_descendant(right, child_name) {
								return Some(c);
							}
						}
						Expressions::VariableDeclaration { name, .. } => {
							if child_name == name {
								return Some(statement.clone());
							}
						}
						Expressions::Accessor { left, right } => {
							if let Some(c) = get_non_intrinsic_descendant(left, child_name) {
								return Some(c);
							}
							if let Some(c) = get_non_intrinsic_descendant(right, child_name) {
								return Some(c);
							}
						}
						Expressions::Return { value } => {
							if let Some(value) = value {
								if let Some(c) = get_non_intrinsic_descendant(value, child_name) {
									return Some(c);
								}
							}
						}
						_ => {}
					},
					Nodes::Raw { output, .. } => {
						for output in output {
							if let Some(c) = get_non_intrinsic_descendant(output, child_name) {
								return Some(c);
							}
						}
					}
					_ => {}
				}
			}
		}
		Nodes::Expression(expression) => match expression {
			Expressions::Operator { left, right, .. } => {
				if let Some(c) = get_non_intrinsic_descendant(left, child_name) {
					return Some(c);
				}
				if let Some(c) = get_non_intrinsic_descendant(right, child_name) {
					return Some(c);
				}
			}
			Expressions::Member { source, .. } => {
				if let Some(c) = get_non_intrinsic_descendant(source, child_name) {
					return Some(c);
				}
			}
			Expressions::Expression { elements } => {
				for element in elements {
					if let Some(c) = get_non_intrinsic_descendant(element, child_name) {
						return Some(c);
					}
				}
			}
			Expressions::VariableDeclaration { r#type, .. } => {
				if let Some(c) = get_non_intrinsic_descendant(r#type, child_name) {
					return Some(c);
				}
			}
			Expressions::Accessor { left, right } => {
				if let Some(c) = get_non_intrinsic_descendant(right, child_name) {
					return Some(c);
				}
				if let Some(c) = get_non_intrinsic_descendant(left, child_name) {
					return Some(c);
				}
			}
			Expressions::IntrinsicCall { intrinsic, .. } => {
				let intrinsic = intrinsic.borrow();
				if let Nodes::Intrinsic { r#return, .. } = intrinsic.node() {
					if let Some(c) = get_non_intrinsic_descendant(r#return, child_name) {
						return Some(c);
					}
				}
			}
			Expressions::Return { value } => {
				if let Some(value) = value {
					if let Some(c) = get_non_intrinsic_descendant(value, child_name) {
						return Some(c);
					}
				}
			}
			_ => {}
		},
		Nodes::Raw { output, .. } => {
			for output in output {
				if let Some(c) = get_non_intrinsic_descendant(output, child_name) {
					return Some(c);
				}
			}
		}
		Nodes::Binding { r#type, .. } => {
			if let BindingTypes::Buffer { members } = r#type {
				for member in members {
					if let Some(c) = get_non_intrinsic_descendant(member, child_name) {
						return Some(c);
					}
				}
			}
		}
		Nodes::Input { format, .. } | Nodes::Output { format, .. } => {
			if let Some(c) = get_non_intrinsic_descendant(format, child_name) {
				return Some(c);
			}
		}
		_ => {}
	}

	None
}

fn lex_parsed_node<'a>(chain: Vec<NodeReference>, parser_node: &parser::Node) -> Result<NodeReference, LexError> {
	let node = match &parser_node.node {
		parser::Nodes::Null => Node::new(Nodes::Null).into(),
		parser::Nodes::Scope { name, children } => {
			assert_ne!(*name, "root"); // The root scope node cannot be an inner part of the program.

			let this: NodeReference = Node::scope(name.to_string()).into();

			for child in children {
				let c = lex_parsed_node(
					{
						let mut chain = chain.clone();
						chain.push(this.clone());
						chain
					},
					child,
				)?;
				this.borrow_mut().add_child(c);
			}

			this
		}
		parser::Nodes::Struct { name, fields } => {
			if let Some(n) = get_reference(&chain, name.as_ref()) {
				// If the type already exists, return it.
				return Ok(n.clone());
			}

			let this: NodeReference = Node::r#struct(name.as_ref(), Vec::new()).into();

			for field in fields {
				let mut chain = chain.clone();
				chain.push(this.clone());
				let c = lex_parsed_node(chain, &field)?;
				this.borrow_mut().add_child(c);
			}

			this
		}
		parser::Nodes::Specialization { name, r#type } => {
			let t = get_reference(&chain, r#type.as_ref()).ok_or(LexError::ReferenceToUndefinedType {
				type_name: r#type.to_string(),
			})?;

			let this = Node::new(Nodes::Specialization {
				name: name.to_string(),
				r#type: t,
			});

			this.into()
		}
		parser::Nodes::Member { name, r#type } => {
			let t = if r#type.contains('<') {
				let mut s = r#type.split(|c| c == '<' || c == '>');

				let outer_type_name = s.next().ok_or(LexError::Undefined {
					message: Some("No outer name".to_string()),
				})?;

				let outer_type = get_reference(&chain, outer_type_name).ok_or(LexError::ReferenceToUndefinedType {
					type_name: outer_type_name.to_string(),
				})?;

				let inner_type_name = s.next().ok_or(LexError::Undefined {
					message: Some("No inner name".to_string()),
				})?;

				let inner_type = if let Some(stripped) = inner_type_name.strip_suffix('*') {
					let x = Node::internal_new(Node {
						node: Nodes::Struct {
							name: format!("{}*", stripped),
							template: Some(outer_type.clone()),
							fields: Vec::new(),
							types: Vec::new(),
						},
					});

					x
				} else {
					get_reference(&chain, inner_type_name).ok_or(LexError::ReferenceToUndefinedType {
						type_name: inner_type_name.to_string(),
					})?
				};

				if let Some(n) = get_reference(&chain, r#type) {
					// If the specialized generic type already exists, return it.
					return Ok(n.clone());
				}

				let children = Vec::new();

				let this = Node {
					node: Nodes::Struct {
						name: r#type.to_string(),
						template: Some(outer_type.clone()),
						fields: children,
						types: vec![inner_type],
					},
				};

				let this: NodeReference = this.into();

				return Ok(this);
			} else if r#type.contains('[') {
				let mut s = r#type.split(|c| c == '[' || c == ']');

				let type_name = s.next().ok_or(LexError::Undefined {
					message: Some("No type name".to_string()),
				})?;

				let member_type = get_reference(&chain, type_name).ok_or(LexError::ReferenceToUndefinedType {
					type_name: type_name.to_string(),
				})?;

				let count = s
					.next()
					.ok_or(LexError::Undefined {
						message: Some("No count".to_string()),
					})?
					.parse()
					.map_err(|_| LexError::Undefined {
						message: Some("Invalid count".to_string()),
					})?;

				return Ok(Node::array(name, member_type, count));
			} else {
				get_reference(&chain, r#type).ok_or(LexError::ReferenceToUndefinedType {
					type_name: r#type.to_string(),
				})?
			};

			let this: NodeReference = Node::member(name, t).into();

			this
		}
		parser::Nodes::Parameter { name, r#type } => {
			let t = get_reference(&chain, r#type).ok_or(LexError::ReferenceToUndefinedType {
				type_name: r#type.to_string(),
			})?;

			let this = Node::new(Nodes::Parameter {
				name: name.to_string(),
				r#type: t,
			});

			this.into()
		}
		parser::Nodes::Input { name, format, location } => {
			let t = get_reference(&chain, format).ok_or(LexError::ReferenceToUndefinedType {
				type_name: format.to_string(),
			})?;

			let this = Node::new(Nodes::Input {
				name: name.to_string(),
				format: t,
				location: location.clone(),
			});

			this.into()
		}
		parser::Nodes::Output { name, format, location } => {
			let t = get_reference(&chain, format).ok_or(LexError::ReferenceToUndefinedType {
				type_name: format.to_string(),
			})?;

			let this = Node::new(Nodes::Output {
				name: name.to_string(),
				format: t,
				location: location.clone(),
			});

			this.into()
		}
		parser::Nodes::Function {
			name,
			return_type,
			statements,
			params,
			..
		} => {
			let t = get_reference(&chain, return_type).ok_or(LexError::ReferenceToUndefinedType {
				type_name: return_type.to_string(),
			})?;

			let this: NodeReference = Node::function(name, Vec::new(), t, Vec::new()).into();

			for param in params {
				let mut chain = chain.clone();
				chain.push(this.clone());
				let c = lex_parsed_node(chain, param)?;
				match this.borrow_mut().node_mut() {
					Nodes::Function { params, .. } => {
						params.push(c);
					}
					_ => {
						panic!("Expected function");
					}
				}
			}

			for statement in statements {
				let mut chain = chain.clone();
				chain.push(this.clone());
				let c = lex_parsed_node(chain, statement)?;
				this.borrow_mut().add_child(c);
			}

			this
		}
		parser::Nodes::PushConstant { members } => {
			let this: NodeReference = Node::push_constant(vec![]).into();

			for member in members {
				let mut chain = chain.clone();
				chain.push(this.clone());
				if let parser::Nodes::Member { .. } = &member.node {
					let c = lex_parsed_node(chain, &member)?;
					this.borrow_mut().add_child(c);
				}
			}

			this
		}
		parser::Nodes::Binding {
			name,
			r#type,
			set,
			descriptor,
			read,
			write,
			count,
		} => {
			let r#type = match &r#type.node {
				parser::Nodes::Type { members, .. } => BindingTypes::Buffer {
					members: members
						.iter()
						.map(|m| lex_parsed_node(chain.clone(), m))
						.collect::<Result<Vec<NodeReference>, LexError>>()?,
				},
				parser::Nodes::Image { format } => BindingTypes::Image {
					format: format.to_string(),
				},
				parser::Nodes::CombinedImageSampler { format } => BindingTypes::CombinedImageSampler {
					format: format.to_string(),
				},
				_ => {
					return Err(LexError::Undefined {
						message: Some("Invalid binding type".to_string()),
					});
				}
			};

			let this = if let Some(count) = count {
				Node::binding_array(name, r#type, *set, *descriptor, *read, *write, count.get())
			} else {
				Node::binding(name, r#type, *set, *descriptor, *read, *write)
			};

			this.into()
		}
		parser::Nodes::Type { name, members } => {
			let mut this = Node::r#struct(name, Vec::new());

			for member in members {
				let c = lex_parsed_node(chain.clone(), member)?;
				this.add_child(c);
			}

			this.into()
		}
		parser::Nodes::Image { format } => {
			let this = Node::binding(
				"image",
				BindingTypes::Image {
					format: format.to_string(),
				},
				0,
				0,
				false,
				false,
			);

			this.into()
		}
		parser::Nodes::CombinedImageSampler { format } => {
			let this = Node::binding(
				"combined_image_sampler",
				BindingTypes::CombinedImageSampler {
					format: format.to_string(),
				},
				0,
				0,
				false,
				false,
			);

			this.into()
		}
		parser::Nodes::RawCode {
			glsl,
			hlsl,
			input,
			output,
			..
		} => {
			let mut inputs = Vec::new();

			for i in *input {
				let name = i;
				inputs.push(
					get_reference(&chain, name)
						.ok_or(LexError::AccessingUndeclaredMember { name: name.to_string() })?
						.clone(),
				);
			}

			let mut outputs = Vec::new();

			for o in *output {
				let name = o.to_string();
				outputs.push(
					Node::expression(Expressions::VariableDeclaration {
						name: name.clone(),
						r#type: get_reference(&chain, "vec3f").ok_or(LexError::AccessingUndeclaredMember { name })?,
					})
					.into(),
				);
			}

			let this = Node::raw(
				glsl.as_ref().map(|v| v.to_string()),
				hlsl.as_ref().map(|v| v.to_string()),
				inputs,
				outputs,
			);

			this.into()
		}
		parser::Nodes::Literal { name, body } => Node::new(Nodes::Literal {
			name: name.to_string(),
			value: lex_parsed_node(chain, body)?,
		})
		.into(),
		parser::Nodes::Expression(expression) => {
			let this = match expression {
				parser::Expressions::Return { value } => Node::expression(Expressions::Return {
					value: match value {
						Some(value) => Some(lex_parsed_node(chain.clone(), value)?),
						None => None,
					},
				}),
				parser::Expressions::Accessor { left, right } => {
					let left = lex_parsed_node(chain.clone(), left)?;

					let right = {
						let left = left.clone();

						let mut chain = chain.clone();
						chain.push(left); // Add left to chain to be able to access its members

						lex_parsed_node(chain.clone(), right)?
					};

					Node::expression(Expressions::Accessor { left, right })
				}
				parser::Expressions::Member { name } => Node::expression(Expressions::Member {
					source: get_reference(&chain, name)
						.ok_or(LexError::AccessingUndeclaredMember { name: name.to_string() })?
						.clone(),
					name: name.to_string(),
				}),
				parser::Expressions::Literal { value } => Node::expression(Expressions::Literal {
					value: value.to_string(),
				}),
				parser::Expressions::Expression(elements) => Node::sentence(
					elements
						.iter()
						.map(|e| lex_parsed_node(chain.clone(), e))
						.collect::<Result<Vec<NodeReference>, LexError>>()?,
				),
				parser::Expressions::Call { name, parameters } => {
					// let r = function.clone(); // Clone to be able to borrow it in and return it
					let function = get_reference(&chain, name).ok_or(LexError::ReferenceToUndefinedType {
						type_name: name.to_string(),
					})?;
					let r = function.clone(); // Clone to be able to borrow it in and return it
					let parameters = parameters
						.iter()
						.map(|e| lex_parsed_node(chain.clone(), e))
						.collect::<Result<Vec<NodeReference>, LexError>>()?;

					{
						// Validate function call
						let b = RefCell::borrow(&function.0);
						match b.node() {
							Nodes::Function { params, .. } | Nodes::Struct { fields: params, .. } => {
								if params.len() != parameters.len() {
									return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters);
								}
								Node::expression(Expressions::FunctionCall { function: r, parameters })
							}
							Nodes::Intrinsic { elements, .. } => Node::expression(Expressions::IntrinsicCall {
								intrinsic: r,
								arguments: parameters.clone(),
								elements: build_intrinsic(elements, &parameters)?,
							}),
							_ => {
								return Err(LexError::Undefined {
									message: Some("Encountered parsing error while evaluating function call. Expected Function | Struct | Intrinsic, but found other.".to_string()),
								});
							}
						}
					}
				}
				parser::Expressions::Operator { name, left, right } => Node::expression(Expressions::Operator {
					operator: match *name {
						"+" => Operators::Plus,
						"-" => Operators::Minus,
						"*" => Operators::Multiply,
						"/" => Operators::Divide,
						"%" => Operators::Modulo,
						"=" => Operators::Assignment,
						"==" => Operators::Equality,
						_ => {
							panic!("Invalid operator")
						}
					},
					left: lex_parsed_node(chain.clone(), left)?,
					right: lex_parsed_node(chain.clone(), right)?,
				}),
				parser::Expressions::VariableDeclaration { name, r#type } => {
					let this = Node::expression(Expressions::VariableDeclaration {
						name: name.to_string(),
						r#type: get_reference(&chain, r#type).ok_or(LexError::ReferenceToUndefinedType {
							type_name: r#type.to_string(),
						})?,
					});

					this
				}
				parser::Expressions::RawCode {
					glsl,
					hlsl,
					input,
					output,
				} => {
					let mut inputs = Vec::new();

					for i in *input {
						let name = i;
						inputs.push(
							get_reference(&chain, name)
								.ok_or(LexError::AccessingUndeclaredMember { name: name.to_string() })?
								.clone(),
						);
					}

					let mut outputs = Vec::new();

					for o in *output {
						let name = o.to_string();
						outputs.push(
							Node::expression(Expressions::VariableDeclaration {
								name: name.clone(),
								r#type: get_reference(&chain, "vec3f").ok_or(LexError::AccessingUndeclaredMember { name })?,
							})
							.into(),
						);
					}

					Node::raw(glsl.map(|v| v.to_string()), hlsl.map(|v| v.to_string()), inputs, outputs)
				}
				parser::Expressions::Macro { name, body } => Node::r#macro(name, lex_parsed_node(chain, body)?),
			};

			this.into()
		}
		parser::Nodes::Intrinsic {
			name,
			elements,
			r#return,
			..
		} => {
			let this: NodeReference = Node::intrinsic(
				name,
				Vec::new(),
				get_reference(&chain, r#return).ok_or(LexError::ReferenceToUndefinedType {
					type_name: r#return.to_string(),
				})?,
			)
			.into();

			for e in elements {
				let mut chain = chain.clone();
				chain.push(this.clone());
				let c = lex_parsed_node(chain, e)?;
				this.borrow_mut().add_child(c);
			}

			this
		}
	};

	Ok(node)
}

fn build_intrinsic(elements: &[NodeReference], parameters: &[NodeReference]) -> Result<Vec<NodeReference>, LexError> {
	let expected_parameter_count = elements
		.iter()
		.filter(|element| matches!(element.borrow().node(), Nodes::Parameter { .. }))
		.count();

	if expected_parameter_count != parameters.len() {
		return Err(LexError::FunctionCallParametersDoNotMatchFunctionParameters);
	}

	let has_body = elements
		.iter()
		.any(|element| !matches!(element.borrow().node(), Nodes::Parameter { .. }));

	if !has_body {
		return Ok(parameters.to_vec());
	}

	build_intrinsic_elements(elements, &mut parameters.iter())
}

fn build_intrinsic_elements<'a>(
	elements: &[NodeReference],
	parameters: &mut impl Iterator<Item = &'a NodeReference>,
) -> Result<Vec<NodeReference>, LexError> {
	let mut ret = Vec::new();

	for e in elements
		.iter()
		.filter(|e| !matches!(e.borrow().node(), Nodes::Parameter { .. }))
	{
		let f = e.borrow();
		let e = match f.node() {
			Nodes::Expression(expression) => match expression {
				Expressions::Member { source, .. } => match source.deref().borrow().node() {
					Nodes::Parameter { .. } => parameters
						.next()
						.ok_or(LexError::Undefined {
							message: Some("Expected parameter".to_string()),
						})?
						.clone(),
					_ => e.clone(),
				},
				Expressions::Expression { elements } => NodeReference::from(Node::expression(Expressions::Expression {
					elements: build_intrinsic_elements(elements, parameters)?,
				})),
				_ => e.clone(),
			},
			_ => e.clone(),
		};

		ret.push(e);
	}

	Ok(ret)
}

fn builtin_intrinsic(name: &str, parameters: Vec<(&str, NodeReference)>, r#return: NodeReference) -> NodeReference {
	let intrinsic: NodeReference = Node::intrinsic(name, Vec::new(), r#return).into();

	for (parameter_name, parameter_type) in parameters {
		intrinsic.borrow_mut().add_child(
			Node::new(Nodes::Parameter {
				name: parameter_name.to_string(),
				r#type: parameter_type,
			})
			.into(),
		);
	}

	intrinsic
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
			_ => {
				panic!("Expected type");
			}
		}
	}

	#[test]
	fn lex_non_existant_function_struct_member_type() {
		let source = "
Foo: struct {
	bar: NonExistantType
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		lex(node)
			.err()
			.filter(|e| {
				e == &LexError::ReferenceToUndefinedType {
					type_name: "NonExistantType".to_string(),
				}
			})
			.expect("Expected error");
	}

	#[test]
	fn lex_non_existant_function_return_type() {
		let source = "
main: fn () -> NonExistantType {}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		lex(node)
			.err()
			.filter(|e| {
				e == &LexError::ReferenceToUndefinedType {
					type_name: "NonExistantType".to_string(),
				}
			})
			.expect("Expected error");
	}

	#[test]
	fn lex_wrong_parameter_count() {
		let source = "
function: fn () -> void {}
main: fn () -> void {
	function(vec3f(1.0, 1.0, 1.0), vec3f(0.0, 0.0, 0.0));
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		lex(node)
			.err()
			.filter(|e| e == &LexError::FunctionCallParametersDoNotMatchFunctionParameters)
			.expect("Expected error");
	}

	#[test]
	fn lex_function() {
		let source = "
main: fn () -> void {
	let position: vec4f = vec4f(0.0, 0.0, 0.0, 1.0);
	position = position;
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		let node = lex(node).expect("Failed to lex");

		let vec4f = node.get_descendant("vec4f").expect("Expected vec4f");

		let nb = node.borrow();

		match &nb.node {
			Nodes::Scope { .. } => {
				let main = node.get_descendant("main").expect("Expected main");
				let main = RefCell::borrow(&main.0);

				match main.node() {
					Nodes::Function {
						name,
						return_type,
						statements,
						..
					} => {
						assert_eq!(name, "main");
						assert_type(&return_type.borrow(), "void");

						let position = statements[0].borrow();

						match position.node() {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								let position = left.borrow();

								assert_eq!(operator, &Operators::Assignment);

								match position.node() {
									Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
										assert_eq!(name, "position");

										assert_eq!(r#type, &vec4f);
									}
									_ => {
										panic!("Expected expression");
									}
								}

								let constructor = right.borrow();

								match constructor.node() {
									Nodes::Expression(Expressions::FunctionCall {
										function, parameters, ..
									}) => {
										let function = RefCell::borrow(&function.0);
										let name = function.get_name().expect("Expected name");

										assert_eq!(name, "vec4f");
										assert_eq!(parameters.len(), 4);
									}
									_ => {
										panic!("Expected expression");
									}
								}
							}
							_ => {
								panic!("Expected variable declaration");
							}
						}
					}
					_ => {
						panic!("Expected function.");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
		}
	}

	#[test]
	#[ignore]
	fn lex_member() {
		let source = "
color: In<vec4f>;
";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		let node = lex(node).expect("Failed to lex");
		let node = node.borrow();

		match node.node() {
			Nodes::Scope { name, children, .. } => {
				assert_eq!(name, "root");

				let color = children[0].borrow();

				match color.node() {
					Nodes::Member { name, r#type, .. } => {
						assert_eq!(name, "color");
						assert_type(&r#type.borrow(), "In<vec4f>");
					}
					_ => {
						panic!("Expected feature");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
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
		let node = parser::parse(&tokens).expect("Failed to parse");
		lex(node).expect("Failed to lex");
	}

	#[test]
	fn lex_struct() {
		let script = r#"
		Vertex: struct {
			array: u32[3],
			position: vec3f,
			normal: vec3f,
		}
		"#;

		let tokens = tokenizer::tokenize(script).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		let node = lex(node).expect("Failed to lex");

		let nb = node.borrow();

		match nb.node() {
			Nodes::Scope { name, .. } => {
				assert_eq!(name, "root");

				let vertex = node.get_descendant("Vertex").expect("Expected Vertex");
				let vertex = RefCell::borrow(&vertex.0);

				match vertex.node() {
					Nodes::Struct { name, fields, .. } => {
						assert_eq!(name, "Vertex");
						assert_eq!(fields.len(), 3);

						let array = fields[0].borrow();

						match array.node() {
							Nodes::Member { name, r#type, count } => {
								assert_eq!(name, "array");
								assert_type(&r#type.borrow(), "u32");
								assert_eq!(count, &Some(NonZeroUsize::new(3).expect("Invalid count")));
							}
							_ => {
								panic!("Expected member");
							}
						}
					}
					_ => {
						panic!("Expected struct");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
		}
	}

	#[test]
	fn lex_array_index_accessor() {
		let script = r#"
		main: fn () -> void {
			let value: f32 = buff.values[1];
		}
		"#;

		let mut root = Node::root();
		let float_type = root.get_child("f32").expect("Expected f32");
		root.add_child(
			Node::binding(
				"buff",
				BindingTypes::Buffer {
					members: vec![Node::array("values", float_type, 3)],
				},
				0,
				0,
				true,
				false,
			)
			.into(),
		);

		let node = crate::compile_to_besl(script, Some(root)).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let statement = statements[0].borrow();
		let Nodes::Expression(Expressions::Operator { right, .. }) = statement.node() else {
			panic!("Expected assignment");
		};
		let right = right.borrow();
		let Nodes::Expression(Expressions::Accessor { left, right }) = right.node() else {
			panic!("Expected outer accessor");
		};
		assert!(matches!(right.borrow().node(), Nodes::Expression(Expressions::Literal { value }) if value == "1"));
		assert!(matches!(
			left.borrow().node(),
			Nodes::Expression(Expressions::Accessor { .. })
		));
	}

	// #[test]
	// fn push_constant() {
	// }

	#[test]
	fn fragment_shader() {
		let source = r#"
		main: fn () -> void {
			let albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let node = parser::parse(&tokens).expect("Failed to parse");
		let node = lex(node).expect("Failed to lex");

		let nb = node.borrow();

		let vec3f = node.get_descendant("vec3f").expect("Expected vec3f");

		match nb.node() {
			Nodes::Scope { name, .. } => {
				assert_eq!(name, "root");

				let main = node.get_descendant("main").expect("Expected main");
				let main = RefCell::borrow(&main.0);

				match main.node() {
					Nodes::Function {
						name,
						return_type,
						statements,
						..
					} => {
						assert_eq!(name, "main");
						assert_type(&return_type.borrow(), "void");

						let albedo = statements[0].borrow();

						match albedo.node() {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								let albedo = left.borrow();

								assert_eq!(operator, &Operators::Assignment);

								match albedo.node() {
									Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
										assert_eq!(name, "albedo");
										assert_eq!(r#type, &vec3f);
									}
									_ => {
										panic!("Expected expression");
									}
								}

								let constructor = right.borrow();

								match constructor.node() {
									Nodes::Expression(Expressions::FunctionCall {
										function, parameters, ..
									}) => {
										let function = RefCell::borrow(&function.0);
										let name = function.get_name().expect("Expected name");

										assert_eq!(name, "vec3f");
										assert_eq!(parameters.len(), 3);
									}
									_ => {
										panic!("Expected expression");
									}
								}
							}
							_ => {
								panic!("Expected variable declaration");
							}
						}
					}
					_ => {
						panic!("Expected function.");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
		}
	}

	// TODO: test function with body with missing close brace

	#[test]
	fn lex_intrinsic() {
		let source = "
main: fn () -> void {
	let n: f32 = intrinsic(0).y;
}";

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let mut node = parser::parse(&tokens).expect("Failed to parse");

		let intrinsic = parser::Node::intrinsic(
			"intrinsic",
			parser::Node::parameter("num", "u32"),
			parser::Node::sentence(vec![
				parser::Node::glsl("vec3(", &[], &[]),
				parser::Node::member_expression("num"),
				parser::Node::glsl(")", &[], &[]),
			]),
			"vec3f",
		);

		node.add(vec![intrinsic]);

		let node = lex(node).expect("Failed to lex");

		let nb = node.borrow();

		match nb.node() {
			Nodes::Scope { name, .. } => {
				assert_eq!(name, "root");

				let main = node.get_descendant("main").unwrap();
				let main = main.borrow();

				match main.node() {
					Nodes::Function { name, statements, .. } => {
						assert_eq!(name, "main");

						let n = statements[0].borrow();

						match n.node() {
							Nodes::Expression(Expressions::Operator { operator, left, right }) => {
								assert_eq!(operator, &Operators::Assignment);

								let n = left.borrow();

								match n.node() {
									Nodes::Expression(Expressions::VariableDeclaration { name, r#type }) => {
										assert_eq!(name, "n");
										assert_type(&r#type.borrow(), "f32");
									}
									_ => {
										panic!("Expected variable declaration");
									}
								}

								let intrinsic = right.borrow();

								match intrinsic.node() {
									Nodes::Expression(Expressions::Accessor { left, right }) => {
										let left = left.borrow();

										match left.node() {
											Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
												let intrinsic = intrinsic.borrow();

												match intrinsic.node() {
													Nodes::Intrinsic { name, elements, .. } => {
														assert_eq!(name, "intrinsic");
														assert_eq!(elements.len(), 2);
													}
													_ => {
														panic!("Expected intrinsic");
													}
												}
											}
											_ => {
												panic!("Expected intrinsic call");
											}
										}

										let right = right.borrow();

										match right.node() {
											Nodes::Expression(Expressions::Member { name, .. }) => {
												assert_eq!(name, "y");
											}
											_ => {
												panic!("Expected member");
											}
										}
									}
									_ => {
										panic!("Expected accessor");
									}
								}
							}
							_ => {
								panic!("Expected assignment");
							}
						}
					}
					_ => {
						panic!("Expected feature");
					}
				}
			}
			_ => {
				panic!("Expected scope");
			}
		}
	}

	#[test]
	fn lex_builtin_texture_intrinsics() {
		let script = r#"
		main: fn () -> void {
			let uv: vec2f = vec2f(0.5, 0.5);
			let coord: vec2u = vec2u(1, 2);
			let color: vec4f = sample(texture_sampler, uv);
			let texel: vec4f = fetch(texture, coord);
		}
		"#;

		let mut root = Node::root();
		root.add_child(
			Node::binding(
				"texture_sampler",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				0,
				true,
				false,
			)
			.into(),
		);
		root.add_child(
			Node::binding(
				"texture",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				1,
				true,
				false,
			)
			.into(),
		);

		let node = crate::compile_to_besl(script, Some(root)).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let sample_statement = statements[2].borrow();
		let fetch_statement = statements[3].borrow();

		let assert_intrinsic_call = |statement: &Node, expected_name: &str| match statement.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => {
				let right = right.borrow();
				match right.node() {
					Nodes::Expression(Expressions::IntrinsicCall {
						intrinsic,
						arguments,
						elements,
					}) => {
						assert_eq!(arguments.len(), 2);
						assert_eq!(elements.len(), 2);

						let intrinsic = intrinsic.borrow();
						match intrinsic.node() {
							Nodes::Intrinsic {
								name,
								r#return,
								elements,
							} => {
								assert_eq!(name, expected_name);
								assert_type(&r#return.borrow(), "vec4f");
								assert_eq!(elements.len(), 2);
							}
							_ => panic!("Expected intrinsic"),
						}
					}
					_ => panic!("Expected intrinsic call"),
				}
			}
			_ => panic!("Expected assignment"),
		};

		assert_intrinsic_call(&sample_statement, "sample");
		assert_intrinsic_call(&fetch_statement, "fetch");
	}

	#[test]
	fn lex_builtin_texture_intrinsics_validate_parameter_count() {
		let source = r#"
		main: fn () -> void {
			let color: vec4f = sample(texture_sampler);
		}
		"#;

		let tokens = tokenizer::tokenize(source).expect("Failed to tokenize");
		let parsed = parser::parse(&tokens).expect("Failed to parse");

		let mut root = Node::root();
		root.add_child(
			Node::binding(
				"texture_sampler",
				BindingTypes::CombinedImageSampler { format: String::new() },
				0,
				0,
				true,
				false,
			)
			.into(),
		);

		lex_with_root(root, parsed)
			.err()
			.filter(|error| error == &LexError::FunctionCallParametersDoNotMatchFunctionParameters)
			.expect("Expected parameter count validation error");
	}

	#[test]
	fn lex_builtin_image_write_intrinsic() {
		let script = r#"
		main: fn () -> void {
			write(image, vec2u(1, 2), vec4f(1.0, 0.0, 0.0, 1.0));
		}
		"#;

		let mut root = Node::root();
		root.add_child(
			Node::binding(
				"image",
				BindingTypes::Image {
					format: "rgba8".to_string(),
				},
				0,
				0,
				false,
				true,
			)
			.into(),
		);

		let node = crate::compile_to_besl(script, Some(root)).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let write_statement = statements[0].borrow();
		match write_statement.node() {
			Nodes::Expression(Expressions::IntrinsicCall {
				intrinsic,
				arguments,
				elements,
			}) => {
				assert_eq!(arguments.len(), 3);
				assert_eq!(elements.len(), 3);

				let intrinsic = intrinsic.borrow();
				match intrinsic.node() {
					Nodes::Intrinsic { name, r#return, .. } => {
						assert_eq!(name, "write");
						assert_type(&r#return.borrow(), "void");
					}
					_ => panic!("Expected intrinsic"),
				}
			}
			_ => panic!("Expected intrinsic call"),
		}
	}

	#[test]
	fn lex_builtin_dot_intrinsic() {
		let script = r#"
		main: fn () -> void {
			let strength: f32 = dot(vec3f(1.0, 0.0, 0.0), vec3f(0.5, 0.5, 0.0));
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let statement = statements[0].borrow();
		match statement.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::IntrinsicCall {
					intrinsic, arguments, ..
				}) => {
					assert_eq!(arguments.len(), 2);
					match intrinsic.borrow().node() {
						Nodes::Intrinsic { name, r#return, .. } => {
							assert_eq!(name, "dot");
							assert_type(&r#return.borrow(), "f32");
						}
						_ => panic!("Expected intrinsic"),
					}
				}
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}
	}
}
