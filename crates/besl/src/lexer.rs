//! The `lexer` module resolves parsed BESL syntax into a linked semantic tree that later compilation stages can execute.

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
		find_descendant(self, child_name, DescendantSearch::Any)
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
		let void = primitive_type("void");
		let u8_t = primitive_type("u8");
		let u16_t = primitive_type("u16");
		let u32_t = primitive_type("u32");
		let i32_t = primitive_type("i32");
		let f32_t = primitive_type("f32");

		let vec2u16 = record_type("vec2u16", [("x", u16_t.clone()), ("y", u16_t.clone())]);
		let vec2u32 = record_type("vec2u", [("x", u32_t.clone()), ("y", u32_t.clone())]);
		let vec2i32 = record_type("vec2i", [("x", i32_t.clone()), ("y", i32_t.clone())]);
		let vec2f32 = record_type("vec2f", [("x", f32_t.clone()), ("y", f32_t.clone())]);
		let vec3f32 = record_type("vec3f", [("x", f32_t.clone()), ("y", f32_t.clone()), ("z", f32_t.clone())]);
		let vec3u32 = record_type("vec3u", [("x", u32_t.clone()), ("y", u32_t.clone()), ("z", u32_t.clone())]);
		let vec4f32 = record_type(
			"vec4f",
			[
				("x", f32_t.clone()),
				("y", f32_t.clone()),
				("z", f32_t.clone()),
				("w", f32_t.clone()),
			],
		);
		let mat4f32 = record_type(
			"mat4f",
			[
				("x", vec4f32.clone()),
				("y", vec4f32.clone()),
				("z", vec4f32.clone()),
				("w", vec4f32.clone()),
			],
		);

		let texture_2d = primitive_type("Texture2D");
		let array_texture_2d = primitive_type("ArrayTexture2D");
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
		let cross_intrinsic = builtin_intrinsic(
			"cross",
			vec![("left", vec3f32.clone()), ("right", vec3f32.clone())],
			vec3f32.clone(),
		);
		let length_intrinsic = builtin_intrinsic("length", vec![("value", vec4f32.clone())], f32_t.clone());
		let normalize_intrinsic = builtin_intrinsic("normalize", vec![("value", vec4f32.clone())], vec4f32.clone());
		let max_intrinsic = builtin_intrinsic(
			"max",
			vec![("left", vec3f32.clone()), ("right", vec3f32.clone())],
			vec3f32.clone(),
		);
		let clamp_intrinsic = builtin_intrinsic(
			"clamp",
			vec![
				("value", vec3f32.clone()),
				("minimum", vec3f32.clone()),
				("maximum", vec3f32.clone()),
			],
			vec3f32.clone(),
		);
		let log2_intrinsic = builtin_intrinsic("log2", vec![("value", vec3f32.clone())], vec3f32.clone());
		let pow_intrinsic = builtin_intrinsic(
			"pow",
			vec![("value", vec3f32.clone()), ("exponent", vec3f32.clone())],
			vec3f32.clone(),
		);
		let reflect_intrinsic = builtin_intrinsic(
			"reflect",
			vec![("incident", vec4f32.clone()), ("normal", vec4f32.clone())],
			vec4f32.clone(),
		);
		let thread_idx_intrinsic = builtin_intrinsic("thread_idx", vec![], u32_t.clone());
		let threadgroup_position_intrinsic = builtin_intrinsic("threadgroup_position", vec![], u32_t.clone());
		let thread_id_intrinsic = builtin_intrinsic("thread_id", vec![], vec2u32.clone());
		let set_mesh_output_counts_intrinsic = builtin_intrinsic(
			"set_mesh_output_counts",
			vec![("vertex_count", u32_t.clone()), ("primitive_count", u32_t.clone())],
			void.clone(),
		);
		let set_mesh_vertex_position_intrinsic = builtin_intrinsic(
			"set_mesh_vertex_position",
			vec![("vertex_index", u32_t.clone()), ("position", vec4f32.clone())],
			void.clone(),
		);
		let set_mesh_triangle_intrinsic = builtin_intrinsic(
			"set_mesh_triangle",
			vec![("primitive_index", u32_t.clone()), ("triangle", vec3u32.clone())],
			void.clone(),
		);
		let image_load_intrinsic = builtin_intrinsic(
			"image_load",
			vec![("image", texture_2d.clone()), ("coord", vec2u32.clone())],
			vec4f32.clone(),
		);
		let guard_image_bounds_intrinsic = builtin_intrinsic(
			"guard_image_bounds",
			vec![("image", texture_2d.clone()), ("coord", vec2u32.clone())],
			void.clone(),
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

		let builtins = vec![
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
			cross_intrinsic,
			length_intrinsic,
			normalize_intrinsic,
			max_intrinsic,
			clamp_intrinsic,
			log2_intrinsic,
			pow_intrinsic,
			reflect_intrinsic,
			thread_idx_intrinsic,
			threadgroup_position_intrinsic,
			thread_id_intrinsic,
			set_mesh_output_counts_intrinsic,
			set_mesh_vertex_position_intrinsic,
			set_mesh_triangle_intrinsic,
			image_load_intrinsic,
			guard_image_bounds_intrinsic,
			write_intrinsic,
		];

		let mut root = Node::scope("root".to_string());
		root.add_children(builtins);

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

	pub fn conditional(condition: NodeReference, statements: Vec<NodeReference>) -> Node {
		Node {
			node: Nodes::Conditional { condition, statements },
		}
	}

	pub fn for_loop(
		initializer: NodeReference,
		condition: NodeReference,
		update: NodeReference,
		statements: Vec<NodeReference>,
	) -> Node {
		Node {
			node: Nodes::ForLoop {
				initializer,
				condition,
				update,
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
		Self::binding_with_count(name, r#type, set, binding, read, write, None)
	}

	fn binding_with_count(
		name: &str,
		r#type: BindingTypes,
		set: u32,
		binding: u32,
		read: bool,
		write: bool,
		count: Option<NonZeroUsize>,
	) -> Node {
		Node {
			node: Nodes::Binding {
				name: name.to_string(),
				r#type,
				set,
				binding,
				read,
				write,
				count,
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
		Self::binding_with_count(
			name,
			r#type,
			set,
			binding,
			read,
			write,
			Some(NonZeroUsize::new(count).expect("Invalid count")),
		)
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

	pub fn constant(name: &str, r#type: NodeReference, value: NodeReference) -> Node {
		Node {
			node: Nodes::Const {
				name: name.to_string(),
				r#type,
				value,
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
		Self::output_with_count(name, format, location, None)
	}

	pub fn output_array(name: &str, format: NodeReference, location: u8, count: u32) -> Node {
		Self::output_with_count(name, format, location, NonZeroUsize::new(count as usize))
	}

	fn output_with_count(name: &str, format: NodeReference, location: u8, count: Option<NonZeroUsize>) -> Node {
		Node {
			node: Nodes::Output {
				name: name.to_string(),
				format,
				location,
				count,
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
			| Nodes::Literal { name, .. }
			| Nodes::Const { name, .. } => Some(name),
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
			Nodes::Conditional { condition, statements } => {
				let mut children = Vec::with_capacity(statements.len() + 1);
				children.push(condition.clone());
				children.extend(statements.iter().cloned());
				Some(children)
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				let mut children = Vec::with_capacity(statements.len() + 3);
				children.push(initializer.clone());
				children.push(condition.clone());
				children.push(update.clone());
				children.extend(statements.iter().cloned());
				Some(children)
			}
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
	Conditional {
		condition: NodeReference,
		statements: Vec<NodeReference>,
	},
	ForLoop {
		initializer: NodeReference,
		condition: NodeReference,
		update: NodeReference,
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
		count: Option<NonZeroUsize>,
	},
	Parameter {
		name: String,
		r#type: NodeReference,
	},
	Literal {
		name: String,
		value: NodeReference,
	},
	/// A module-level constant variable declaration. Stores a named, typed value that is known at compile time.
	Const {
		name: String,
		r#type: NodeReference,
		value: NodeReference,
	},
}

impl Nodes {
	pub fn is_leaf(&self) -> bool {
		match self {
			Nodes::Function { .. } => false,
			Nodes::Conditional { .. } | Nodes::ForLoop { .. } => false,
			Nodes::Struct { .. } => false,
			Nodes::Binding { .. } => false,
			Nodes::PushConstant { .. } => false,
			Nodes::Input { .. } | Nodes::Output { .. } => false,
			Nodes::Specialization { .. } => false,
			Nodes::Const { .. } => false,
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

	pub fn is_indexable(&self) -> bool {
		match self {
			Nodes::Member { count, .. } => count.is_some(),
			Nodes::Output { count, .. } => count.is_some(),
			Nodes::Expression(Expressions::Member { source, .. }) => source.borrow().node().is_indexable(),
			Nodes::Expression(Expressions::Accessor { right, .. }) => right.borrow().node().is_indexable(),
			_ => false,
		}
	}

	pub fn is_buffer_binding(&self) -> bool {
		match self {
			Nodes::Binding {
				r#type: BindingTypes::Buffer { .. },
				..
			} => true,
			Nodes::Expression(Expressions::Member { source, .. }) => source.borrow().node().is_buffer_binding(),
			_ => false,
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
			Nodes::Conditional { condition, statements } => {
				write!(
					f,
					"Conditional {{ condition: {:?}, statements: {:?} }}",
					condition, statements
				)
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				write!(
					f,
					"ForLoop {{ initializer: {:?}, condition: {:?}, update: {:?}, statements: {:?} }}",
					initializer, condition, update, statements
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
			Nodes::Output {
				name,
				format,
				location,
				count,
			} => {
				write!(
					f,
					"Output {{ name: {}, format: {:?}, location: {}, count: {:?} }}",
					name,
					format.0.borrow().get_name().map(|e| e.to_string()),
					location,
					count
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
			Nodes::Const { name, r#type, value } => {
				write!(
					f,
					"Const {{ name: {}, type: {:?}, value: {:?} }}",
					name,
					r#type.0.borrow().get_name().map(|e| e.to_string()),
					value
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
	ShiftLeft,
	ShiftRight,
	BitwiseAnd,
	BitwiseOr,
	Assignment,
	Equality,
	LessThan,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum DescendantSearch {
	Any,
	NonIntrinsic,
}

/// Tries to resolve a reference to a node by visiting the chain of nodes which are the context of the element of the program being lexed.
fn get_reference(chain: &[NodeReference], name: &str) -> Option<NodeReference> {
	for node in chain.iter().rev() {
		let reference = match node.borrow().node() {
			Nodes::Intrinsic { .. } => find_descendant(node, name, DescendantSearch::Any),
			_ => find_descendant(node, name, DescendantSearch::NonIntrinsic),
		};

		if let Some(c) = reference {
			return Some(c);
		}
	}

	None
}

fn resolve_type(chain: &[NodeReference], type_name: &str) -> Result<NodeReference, LexError> {
	get_reference(chain, type_name).ok_or(LexError::ReferenceToUndefinedType {
		type_name: type_name.to_string(),
	})
}

fn resolve_member(chain: &[NodeReference], name: &str) -> Result<NodeReference, LexError> {
	get_reference(chain, name).ok_or(LexError::AccessingUndeclaredMember { name: name.to_string() })
}

/// Clones the lexical scope chain and appends the current parent node.
fn extend_chain(chain: &[NodeReference], parent: &NodeReference) -> Vec<NodeReference> {
	let mut extended = chain.to_vec();
	extended.push(parent.clone());
	extended
}

/// Lexes one parser child in the scope of its parent node.
fn lex_child_with_parent(
	chain: &[NodeReference],
	parent: &NodeReference,
	parser_node: &parser::Node,
) -> Result<NodeReference, LexError> {
	lex_parsed_node(extend_chain(chain, parent), parser_node)
}

/// Resolves raw-code IO references and lowers them into a lexer node.
fn lex_raw_code(
	chain: &[NodeReference],
	glsl: Option<&str>,
	hlsl: Option<&str>,
	input: &[&str],
	output: &[&str],
) -> Result<Node, LexError> {
	let inputs = input
		.iter()
		.map(|name| resolve_member(chain, name))
		.collect::<Result<Vec<_>, _>>()?;

	let vec3f = resolve_member(chain, "vec3f")?;
	let outputs = output
		.iter()
		.map(|name| {
			Node::expression(Expressions::VariableDeclaration {
				name: (*name).to_string(),
				r#type: vec3f.clone(),
			})
			.into()
		})
		.collect();

	Ok(Node::raw(glsl.map(str::to_string), hlsl.map(str::to_string), inputs, outputs))
}

fn find_descendant(node: &NodeReference, child_name: &str, mode: DescendantSearch) -> Option<NodeReference> {
	let prefer_descendants_before_self = mode == DescendantSearch::NonIntrinsic
		&& matches!(
			node.borrow().node(),
			Nodes::Binding { .. }
				| Nodes::PushConstant { .. }
				| Nodes::Member { .. }
				| Nodes::Parameter { .. }
				| Nodes::Input { .. }
				| Nodes::Output { .. }
				| Nodes::Expression(Expressions::Member { .. } | Expressions::VariableDeclaration { .. })
		);

	if !prefer_descendants_before_self && node.borrow().get_name() == Some(child_name) {
		return Some(node.clone());
	}

	let result = match node.borrow().node() {
		Nodes::Scope { children, .. } | Nodes::Struct { fields: children, .. } | Nodes::PushConstant { members: children } => {
			find_in_children(children, child_name, mode == DescendantSearch::NonIntrinsic, mode)
		}
		Nodes::Intrinsic { elements, .. } => {
			if mode == DescendantSearch::Any {
				find_in_children(elements, child_name, false, mode)
			} else {
				None
			}
		}
		Nodes::Member { r#type, .. } | Nodes::Parameter { r#type, .. } => find_descendant(r#type, child_name, mode),
		Nodes::Function { params, statements, .. } => find_in_function(params, statements, child_name, mode),
		Nodes::Conditional { condition, statements } if mode == DescendantSearch::NonIntrinsic => {
			find_descendant(condition, child_name, mode).or_else(|| find_in_descendants(statements, child_name, mode))
		}
		Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} if mode == DescendantSearch::NonIntrinsic => find_descendant(initializer, child_name, mode)
			.or_else(|| find_descendant(condition, child_name, mode))
			.or_else(|| find_descendant(update, child_name, mode))
			.or_else(|| find_in_descendants(statements, child_name, mode)),
		Nodes::Expression(expression) => find_in_expression(expression, child_name, mode),
		Nodes::Raw { output, .. } => find_in_descendants(output, child_name, mode),
		Nodes::Binding { r#type, .. } => match r#type {
			BindingTypes::Buffer { members } => find_in_descendants(members, child_name, mode),
			_ => None,
		},
		Nodes::Input { format, .. } | Nodes::Output { format, .. } => find_descendant(format, child_name, mode),
		_ => None,
	};

	result.or_else(|| {
		if prefer_descendants_before_self && node.borrow().get_name() == Some(child_name) {
			Some(node.clone())
		} else {
			None
		}
	})
}

fn find_in_children(
	children: &[NodeReference],
	child_name: &str,
	prefer_direct_children: bool,
	mode: DescendantSearch,
) -> Option<NodeReference> {
	if prefer_direct_children {
		find_named_child(children, child_name).or_else(|| find_in_descendants(children, child_name, mode))
	} else {
		find_in_descendants(children, child_name, mode)
	}
}

fn find_named_child(children: &[NodeReference], child_name: &str) -> Option<NodeReference> {
	children
		.iter()
		.find(|child| child.borrow().get_name() == Some(child_name))
		.cloned()
}

fn find_in_descendants(children: &[NodeReference], child_name: &str, mode: DescendantSearch) -> Option<NodeReference> {
	children.iter().find_map(|child| find_descendant(child, child_name, mode))
}

fn find_in_function(
	params: &[NodeReference],
	statements: &[NodeReference],
	child_name: &str,
	mode: DescendantSearch,
) -> Option<NodeReference> {
	find_named_child(params, child_name).or_else(|| {
		statements
			.iter()
			.find_map(|statement| find_in_function_statement(statement, child_name, mode))
	})
}

fn find_in_function_statement(statement: &NodeReference, child_name: &str, mode: DescendantSearch) -> Option<NodeReference> {
	match statement.borrow().node() {
		Nodes::Expression(expression) => find_in_function_expression(statement, expression, child_name, mode),
		Nodes::Raw { output, .. } if mode == DescendantSearch::Any => find_in_descendants(output, child_name, mode),
		_ => None,
	}
}

fn find_in_function_expression(
	statement: &NodeReference,
	expression: &Expressions,
	child_name: &str,
	mode: DescendantSearch,
) -> Option<NodeReference> {
	match mode {
		DescendantSearch::Any => match expression {
			Expressions::Operator { left, right, .. } => {
				find_descendant(left, child_name, mode).or_else(|| find_descendant(right, child_name, mode))
			}
			Expressions::VariableDeclaration { name, .. } if child_name == name => Some(statement.clone()),
			Expressions::Accessor { left, right } => {
				find_descendant(left, child_name, mode).or_else(|| find_descendant(right, child_name, mode))
			}
			Expressions::Return { value } => value.as_ref().and_then(|value| find_descendant(value, child_name, mode)),
			_ => None,
		},
		DescendantSearch::NonIntrinsic => match expression {
			Expressions::VariableDeclaration { name, .. } if child_name == name => Some(statement.clone()),
			Expressions::Operator { left, .. } => find_descendant(left, child_name, mode),
			_ => None,
		},
	}
}

fn find_in_expression(expression: &Expressions, child_name: &str, mode: DescendantSearch) -> Option<NodeReference> {
	match expression {
		Expressions::Operator { left, right, .. } => {
			find_descendant(left, child_name, mode).or_else(|| find_descendant(right, child_name, mode))
		}
		Expressions::Member { source, .. } => find_descendant(source, child_name, mode),
		Expressions::Expression { elements } => find_in_descendants(elements, child_name, mode),
		Expressions::VariableDeclaration { r#type, .. } => find_descendant(r#type, child_name, mode),
		Expressions::Accessor { left, right } => {
			find_descendant(right, child_name, mode).or_else(|| find_descendant(left, child_name, mode))
		}
		Expressions::IntrinsicCall { intrinsic, .. } => {
			let intrinsic = intrinsic.borrow();
			if let Nodes::Intrinsic { r#return, .. } = intrinsic.node() {
				find_descendant(r#return, child_name, mode)
			} else {
				None
			}
		}
		Expressions::Return { value } => value.as_ref().and_then(|value| find_descendant(value, child_name, mode)),
		_ => None,
	}
}

fn lex_parsed_node<'a>(chain: Vec<NodeReference>, parser_node: &parser::Node) -> Result<NodeReference, LexError> {
	let node = match &parser_node.node {
		parser::Nodes::Null => Node::new(Nodes::Null).into(),
		parser::Nodes::Scope { name, children } => {
			assert_ne!(*name, "root"); // The root scope node cannot be an inner part of the program.

			let this: NodeReference = Node::scope(name.to_string()).into();
			for child in children {
				let child = lex_child_with_parent(&chain, &this, child)?;
				this.borrow_mut().add_child(child);
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
				let field = lex_child_with_parent(&chain, &this, field)?;
				this.borrow_mut().add_child(field);
			}

			this
		}
		parser::Nodes::Specialization { name, r#type } => {
			let t = resolve_type(&chain, r#type.as_ref())?;

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

				let outer_type = resolve_type(&chain, outer_type_name)?;

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
					resolve_type(&chain, inner_type_name)?
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

				let member_type = resolve_type(&chain, type_name)?;

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
				resolve_type(&chain, r#type)?
			};

			let this: NodeReference = Node::member(name, t).into();

			this
		}
		parser::Nodes::Parameter { name, r#type } => {
			let t = resolve_type(&chain, r#type)?;

			let this = Node::new(Nodes::Parameter {
				name: name.to_string(),
				r#type: t,
			});

			this.into()
		}
		parser::Nodes::Input { name, format, location } => {
			let t = resolve_type(&chain, format)?;

			let this = Node::new(Nodes::Input {
				name: name.to_string(),
				format: t,
				location: location.clone(),
			});

			this.into()
		}
		parser::Nodes::Output {
			name,
			format,
			location,
			count,
		} => {
			let t = resolve_type(&chain, format)?;

			let this = Node::new(Nodes::Output {
				name: name.to_string(),
				format: t,
				location: location.clone(),
				count: *count,
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
			let t = resolve_type(&chain, return_type)?;

			let this: NodeReference = Node::function(name, Vec::new(), t, Vec::new()).into();

			for param in params {
				let param = lex_child_with_parent(&chain, &this, param)?;
				match this.borrow_mut().node_mut() {
					Nodes::Function { params, .. } => {
						params.push(param);
					}
					_ => {
						panic!("Expected function");
					}
				}
			}

			for statement in statements {
				let statement = lex_child_with_parent(&chain, &this, statement)?;
				this.borrow_mut().add_child(statement);
			}

			this
		}
		parser::Nodes::Conditional { condition, statements } => {
			let condition = lex_parsed_node(chain.clone(), condition)?;
			let mut lexed_statements = Vec::with_capacity(statements.len());

			for statement in statements {
				lexed_statements.push(lex_parsed_node(chain.clone(), statement)?);
			}

			Node::conditional(condition, lexed_statements).into()
		}
		parser::Nodes::ForLoop {
			initializer,
			condition,
			update,
			statements,
		} => {
			let initializer = lex_parsed_node(chain.clone(), initializer)?;
			let mut scoped_chain = chain.clone();
			scoped_chain.push(initializer.clone());
			let condition = lex_parsed_node(scoped_chain.clone(), condition)?;
			let update = lex_parsed_node(scoped_chain.clone(), update)?;
			let mut lexed_statements = Vec::with_capacity(statements.len());

			for statement in statements {
				lexed_statements.push(lex_parsed_node(scoped_chain.clone(), statement)?);
			}

			Node::for_loop(initializer, condition, update, lexed_statements).into()
		}
		parser::Nodes::PushConstant { members } => {
			let this: NodeReference = Node::push_constant(vec![]).into();

			for member in members
				.iter()
				.filter(|member| matches!(member.node, parser::Nodes::Member { .. }))
			{
				let c = lex_child_with_parent(&chain, &this, member)?;
				this.borrow_mut().add_child(c);
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
		} => lex_raw_code(&chain, glsl.as_deref(), hlsl.as_deref(), input, output)?.into(),
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
					source: resolve_member(&chain, name)?,
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
					let function = resolve_type(&chain, name)?;
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
						"<<" => Operators::ShiftLeft,
						">>" => Operators::ShiftRight,
						"&" => Operators::BitwiseAnd,
						"|" => Operators::BitwiseOr,
						"=" => Operators::Assignment,
						"==" => Operators::Equality,
						"<" => Operators::LessThan,
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
						r#type: resolve_type(&chain, r#type)?,
					});

					this
				}
				parser::Expressions::RawCode {
					glsl,
					hlsl,
					input,
					output,
				} => lex_raw_code(&chain, *glsl, *hlsl, input, output)?,
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
			let this: NodeReference = Node::intrinsic(name, Vec::new(), resolve_type(&chain, r#return)?).into();

			for element in elements {
				let element = lex_child_with_parent(&chain, &this, element)?;
				this.borrow_mut().add_child(element);
			}

			this
		}
		parser::Nodes::Const { name, r#type, value } => {
			let t = resolve_type(&chain, r#type)?;

			let v = lex_parsed_node(chain.clone(), value)?;

			Node::constant(name, t, v).into()
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

fn primitive_type(name: &str) -> NodeReference {
	Node::r#struct(name, Vec::new()).into()
}

fn record_type<const N: usize>(name: &str, fields: [(&str, NodeReference); N]) -> NodeReference {
	Node::r#struct(
		name,
		fields
			.into_iter()
			.map(|(field_name, field_type)| Node::member(field_name, field_type).into())
			.collect(),
	)
	.into()
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

	#[test]
	fn lex_same_named_buffer_members_resolve_to_member_declarations() {
		let script = r#"
		main: fn () -> void {
			let material_index: u32 = meshes.meshes[0].material_index;
			let mapped: u32 = pixel_mapping.pixel_mapping[1];
		}
		"#;

		let mut root = Node::root();
		let u32_type = root.get_child("u32").expect("Expected u32");
		let mesh = root.add_child(Node::r#struct("Mesh", vec![Node::member("material_index", u32_type.clone()).into()]).into());

		root.add_children(vec![
			Node::binding(
				"meshes",
				BindingTypes::Buffer {
					members: vec![Node::array("meshes", mesh, 4)],
				},
				0,
				0,
				true,
				false,
			)
			.into(),
			Node::binding(
				"pixel_mapping",
				BindingTypes::Buffer {
					members: vec![Node::array("pixel_mapping", u32_type, 4)],
				},
				0,
				1,
				true,
				true,
			)
			.into(),
		]);

		let node = crate::compile_to_besl(script, Some(root)).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let material_index_access = match statements[0].borrow().node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => right.clone(),
			_ => panic!("Expected assignment"),
		};
		let (indexed_meshes, material_index_member) = match material_index_access.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, right }) => (left.clone(), right.clone()),
			_ => panic!("Expected struct member accessor"),
		};
		match material_index_member.borrow().node() {
			Nodes::Expression(Expressions::Member { name, source }) => {
				assert_eq!(name, "material_index");
				assert!(matches!(
					source.borrow().node(),
					Nodes::Member { name, count, .. } if name == "material_index" && count.is_none()
				));
			}
			_ => panic!("Expected material_index member expression"),
		}

		let meshes_member = match indexed_meshes.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, .. }) => match left.borrow().node() {
				Nodes::Expression(Expressions::Accessor { left, right }) => {
					assert_eq!(left.borrow().get_name(), Some("meshes"));
					assert!(
						right.borrow().node().is_indexable(),
						"Expected meshes.meshes to stay indexable"
					);
					right.clone()
				}
				_ => panic!("Expected meshes accessor"),
			},
			_ => panic!("Expected indexed meshes accessor"),
		};
		match meshes_member.borrow().node() {
			Nodes::Expression(Expressions::Member { name, source }) => {
				assert_eq!(name, "meshes");
				assert!(matches!(
					source.borrow().node(),
					Nodes::Member { name, count, .. } if name == "meshes" && count == &Some(NonZeroUsize::new(4).expect("Expected valid count"))
				));
			}
			_ => panic!("Expected meshes member expression"),
		}

		let pixel_mapping_access = match statements[1].borrow().node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => right.clone(),
			_ => panic!("Expected assignment"),
		};
		let pixel_mapping_member = match pixel_mapping_access.borrow().node() {
			Nodes::Expression(Expressions::Accessor { left, .. }) => {
				assert!(left.borrow().node().is_indexable());
				match left.borrow().node() {
					Nodes::Expression(Expressions::Accessor { right, .. }) => right.clone(),
					_ => panic!("Expected pixel_mapping accessor"),
				}
			}
			_ => panic!("Expected indexed pixel_mapping accessor"),
		};
		match pixel_mapping_member.borrow().node() {
			Nodes::Expression(Expressions::Member { name, source }) => {
				assert_eq!(name, "pixel_mapping");
				assert!(matches!(
					source.borrow().node(),
					Nodes::Member { name, count, .. } if name == "pixel_mapping" && count == &Some(NonZeroUsize::new(4).expect("Expected valid count"))
				));
			}
			_ => panic!("Expected pixel_mapping member expression"),
		};
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

	#[test]
	fn lex_builtin_cross_intrinsic() {
		let script = r#"
		main: fn () -> void {
			let normal: vec3f = cross(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0));
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
							assert_eq!(name, "cross");
							assert_type(&r#return.borrow(), "vec3f");
						}
						_ => panic!("Expected intrinsic"),
					}
				}
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}
	}

	#[test]
	fn lex_builtin_length_and_normalize_intrinsics() {
		let script = r#"
		main: fn () -> void {
			let magnitude: f32 = length(vec3f(3.0, 4.0, 0.0));
			let direction: vec3f = normalize(vec3f(3.0, 4.0, 0.0));
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let magnitude = statements[0].borrow();
		let direction = statements[1].borrow();

		match magnitude.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => match intrinsic.borrow().node() {
					Nodes::Intrinsic { name, r#return, .. } => {
						assert_eq!(name, "length");
						assert_type(&r#return.borrow(), "f32");
					}
					_ => panic!("Expected intrinsic"),
				},
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}

		match direction.node() {
			Nodes::Expression(Expressions::Operator { right, .. }) => match right.borrow().node() {
				Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => match intrinsic.borrow().node() {
					Nodes::Intrinsic { name, r#return, .. } => {
						assert_eq!(name, "normalize");
						assert_type(&r#return.borrow(), "vec4f");
					}
					_ => panic!("Expected intrinsic"),
				},
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}
	}

	#[test]
	fn lex_builtin_reflect_intrinsic() {
		let root = Node::root();
		let reflect = root.get_child("reflect").expect("Expected reflect builtin");
		let result = match reflect.borrow().node() {
			Nodes::Intrinsic {
				name,
				elements,
				r#return,
			} => {
				assert_eq!(name, "reflect");
				assert_eq!(elements.len(), 2);
				assert_type(&r#return.borrow(), "vec4f");
			}
			_ => panic!("Expected intrinsic"),
		};
		result
	}

	#[test]
	fn lex_builtin_thread_idx_intrinsic() {
		let script = r#"
		main: fn () -> void {
			let index: u32 = thread_idx();
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
					assert!(arguments.is_empty());
					match intrinsic.borrow().node() {
						Nodes::Intrinsic { name, r#return, .. } => {
							assert_eq!(name, "thread_idx");
							assert_type(&r#return.borrow(), "u32");
						}
						_ => panic!("Expected intrinsic"),
					}
				}
				_ => panic!("Expected intrinsic call"),
			},
			_ => panic!("Expected assignment"),
		}
	}

	#[test]
	fn lex_const_variable() {
		let script = r#"
		PI: const f32 = 3.14;

		main: fn () -> void {
			PI;
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");

		let pi = node.get_descendant("PI").expect("Expected PI const");
		let pi = pi.borrow();

		match pi.node() {
			Nodes::Const { name, r#type, value } => {
				assert_eq!(name, "PI");
				assert_eq!(r#type.borrow().get_name().unwrap(), "f32");
				match value.borrow().node() {
					Nodes::Expression(Expressions::Literal { value }) => {
						assert_eq!(value, "3.14");
					}
					_ => panic!("Expected a literal expression value"),
				}
			}
			_ => panic!("Expected Const node"),
		}
	}

	#[test]
	fn lex_conditional_block() {
		let script = r#"
		main: fn () -> void {
			let n: u32 = 0;
			if (n < 1) {
				n = 2;
			}
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let conditional = statements[1].borrow();
		match conditional.node() {
			Nodes::Conditional { condition, statements } => {
				assert_eq!(statements.len(), 1);

				match condition.borrow().node() {
					Nodes::Expression(Expressions::Operator { operator, .. }) => {
						assert_eq!(operator, &Operators::LessThan);
					}
					_ => panic!("Expected less-than condition"),
				}
			}
			_ => panic!("Expected conditional node"),
		}
	}

	#[test]
	fn lex_for_loop_block() {
		let script = r#"
		main: fn () -> void {
			let sum: u32 = 0;
			for (let i: u32 = 0; i < 4; i = i + 1) {
				sum = sum + i;
			}
		}
		"#;

		let node = crate::compile_to_besl(script, None).expect("Failed to lex");
		let main = node.get_descendant("main").expect("Expected main");
		let main = main.borrow();

		let Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected function");
		};

		let for_loop = statements[1].borrow();
		match for_loop.node() {
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				assert_eq!(statements.len(), 1);
				assert!(matches!(
					initializer.borrow().node(),
					Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::Assignment
				));
				assert!(matches!(
					condition.borrow().node(),
					Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::LessThan
				));
				assert!(matches!(
					update.borrow().node(),
					Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::Assignment
				));
			}
			_ => panic!("Expected for loop node"),
		}
	}

	#[test]
	fn lex_bitwise_expression() {
		let script = r#"
		main: fn () -> void {
			let packed: u32 = 1 << 8 | 2 & 255;
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
				Nodes::Expression(Expressions::Operator { operator, left, right }) => {
					assert_eq!(operator, &Operators::BitwiseOr);
					assert!(matches!(
						left.borrow().node(),
						Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::ShiftLeft
					));
					assert!(matches!(
						right.borrow().node(),
						Nodes::Expression(Expressions::Operator { operator, .. }) if operator == &Operators::BitwiseAnd
					));
				}
				_ => panic!("Expected bitwise or expression"),
			},
			_ => panic!("Expected assignment"),
		}
	}
}
