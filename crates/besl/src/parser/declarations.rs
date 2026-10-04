//! Parses BESL tokens into syntax nodes that preserve the source structure.
//!
//! # Example shader
//!
//! ```glsl
//! Light: struct {
//!     position: vec3,
//!     color: vec3,
//! }
//!
//! main: fn () -> void {
//!     gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
//! }
//! ```
//!
//! Use [`crate::parse`] as the entry point. The parser records cross-references by name.
//! The [`crate::lexer`] module resolves those names later.

use super::expressions::{
	execute_parsers, parse_const, parse_descriptor, parse_function, parse_macro, parse_member, parse_push_constant,
	parse_shader_interface_declaration, parse_struct,
};
use crate::lexer::BufferMemoryClass;

/// The `ElseBranch` enum keeps `else if` chains distinct from plain `else` blocks,
/// so later stages can lower each form by structure. See [`Nodes::Conditional`].
#[derive(Clone, Debug)]
pub enum ElseBranch<'a> {
	/// An `else { ... }` block.
	Block(Vec<Node<'a>>),
	/// An `else if` link. The node is always a [`Nodes::Conditional`].
	If(Box<Node<'a>>),
}

impl<'a> ElseBranch<'a> {
	/// Returns the branch as a mutable statement list. An `else if` link is one conditional statement.
	pub fn statements_mut(&mut self) -> &mut [Node<'a>] {
		match self {
			Self::Block(statements) => statements,
			Self::If(conditional) => std::slice::from_mut(conditional),
		}
	}
}

/// The `MatchArm` struct holds one `pattern => body` arm of a `match` statement, as written in source.
/// The lexer checks and normalizes it into a [`crate::MatchArm`]. See [`Nodes::Match`].
#[derive(Clone, Debug)]
pub struct MatchArm<'a> {
	/// The alternatives of an or-pattern such as `1 | 2`. A single pattern is a one-element list.
	pub patterns: Vec<MatchPattern<'a>>,
	/// The arm body. An expression arm such as `0 => n = 1,` is one statement.
	pub statements: Vec<Node<'a>>,
}

/// The `MatchPattern` enum lists the pattern forms a `match` arm can test a scalar against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchPattern<'a> {
	/// A literal such as `3`, `true`, or `-1`. `negative` records a leading `-`.
	Literal { value: &'a str, negative: bool },
	/// The `_` pattern, which matches every value.
	Wildcard,
}

/// The `TypeName` enum preserves type structure while the parser still borrows source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeName<'a> {
	Named(&'a str),
	Array { element: Box<TypeName<'a>>, count: u32 },
	Record { role: RecordRole, fields: Vec<TypeField<'a>> },
}

/// The `RecordRole` enum preserves how an anonymous record participates in a shader stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordRole {
	Interface,
	Output,
}

impl std::fmt::Display for RecordRole {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Interface => "interface",
			Self::Output => "output",
		})
	}
}

/// The `TypeField` struct identifies one named value in an anonymous record type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeField<'a> {
	pub name: &'a str,
	pub type_name: TypeName<'a>,
}

/// The `RecordField` struct pairs a record-literal field name with its value expression.
#[derive(Clone, Debug)]
pub struct RecordField<'a> {
	pub name: &'a str,
	pub value: Node<'a>,
}

impl<'a> From<&'a str> for TypeName<'a> {
	fn from(name: &'a str) -> Self {
		Self::Named(name)
	}
}

impl std::fmt::Display for TypeName<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Named(name) => f.write_str(name),
			Self::Array { element, count } => write!(f, "{element}[{count}]"),
			Self::Record { role, fields } => {
				write!(f, "{role} {{")?;
				for (index, field) in fields.iter().enumerate() {
					if index == 0 {
						f.write_str(" ")?;
					} else {
						f.write_str(", ")?;
					}
					write!(f, "{}: {}", field.name, field.type_name)?;
				}
				f.write_str(if fields.is_empty() { "}" } else { " }" })
			}
		}
	}
}

/// Parses a token stream into the root scope of its declarations.
pub(crate) fn parse<'i, 'a: 'i>(tokens: &'i [&'a str]) -> Result<Node<'a>, ParsingFailReasons> {
	let mut iterator = tokens.iter();

	let parsers = [
		parse_push_constant,
		parse_struct,
		parse_function,
		parse_macro,
		parse_const,
		parse_descriptor,
		parse_shader_interface_declaration,
		parse_member,
	];

	let mut children: Vec<Node<'a>> = Vec::with_capacity(64);

	loop {
		let (expression, iter) = execute_parsers(parsers.as_slice(), iterator)?;

		children.push(expression);

		iterator = iter;

		if iterator.len() == 0 {
			break;
		}
	}

	Ok(Node::root_with_children(children))
}

use std::borrow::Cow;
use std::num::{NonZeroU32, NonZeroUsize};

#[derive(Clone, Debug)]
pub struct Node<'a> {
	pub(crate) node: Nodes<'a>,
}

impl<'a> Node<'a> {
	pub fn root() -> Node<'a> {
		Self::scope("root", Vec::new())
	}

	pub fn root_with_children(children: Vec<Node<'a>>) -> Node<'a> {
		Self::scope("root", children)
	}

	pub fn scope(name: &'a str, children: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Scope { name, children },
		}
	}

	pub fn r#struct(name: &'a str, fields: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Struct { name, fields },
		}
	}

	pub fn member(name: &'a str, r#type: impl Into<String>) -> Node<'a> {
		Node {
			node: Nodes::Member {
				name,
				r#type: r#type.into(),
			},
		}
	}

	pub fn member_expression(name: impl Into<Cow<'a, str>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Member { name: name.into() }),
		}
	}

	pub fn function(
		name: &'a str,
		params: Vec<Node<'a>>,
		return_type: impl Into<TypeName<'a>>,
		statements: Vec<Node<'a>>,
	) -> Node<'a> {
		Node {
			node: Nodes::Function {
				name,
				params,
				return_type: return_type.into(),
				statements,
			},
		}
	}

	/// Builds a `match` statement. The lexer rejects matches that don't cover every scrutinee value.
	pub fn r#match(scrutinee: Node<'a>, arms: Vec<MatchArm<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Match {
				scrutinee: Box::new(scrutinee),
				arms,
			},
		}
	}

	/// Builds an `if` statement. Pass `None` as `else_branch` for an `if` without an `else` branch.
	pub fn conditional(condition: Node<'a>, statements: Vec<Node<'a>>, else_branch: Option<ElseBranch<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Conditional {
				condition: Box::new(condition),
				statements,
				else_branch,
			},
		}
	}

	pub fn for_loop(initializer: Node<'a>, condition: Node<'a>, update: Node<'a>, statements: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::ForLoop {
				initializer: Box::new(initializer),
				condition: Box::new(condition),
				update: Box::new(update),
				statements,
			},
		}
	}

	pub fn main_function(statements: Vec<Node<'a>>) -> Node<'a> {
		Self::function("main", Vec::new(), "void", statements)
	}

	/// Builds a resource binding. Pass [`Node::buffer`], [`Node::image`], or a `combined_*_image_sampler` builder as
	/// `r#type`.
	pub fn binding(name: &'a str, r#type: BindingResource<'a>, slot: u32, read: bool, write: bool) -> Node<'a> {
		Self::binding_with_count(name, r#type, slot, read, write, None, None)
	}

	/// Builds a buffer binding that stores thread-varying data in device memory.
	pub fn device_buffer_binding(name: &'a str, r#type: BindingResource<'a>, slot: u32, read: bool, write: bool) -> Node<'a> {
		Self::binding_with_count(name, r#type, slot, read, write, Some(BufferMemoryClass::Device), None)
	}

	/// Builds a device-memory storage buffer of `element` values whose length is set when the application creates the
	/// buffer, the same as a `descriptor<{ type: element[] }>` declaration.
	///
	/// Use it for buffers whose capacity is a runtime setting, so the capacity does not have to be compiled into the shader.
	pub fn runtime_array_binding(name: &'a str, element: &'a str, slot: u32, read: bool, write: bool) -> Node<'a> {
		Node {
			node: Nodes::Descriptor {
				name,
				resource_type: element,
				runtime_array: true,
				format: None,
				slot,
				read,
				write,
				memory_class: Some(BufferMemoryClass::Device),
				count: None,
			},
		}
	}

	/// Builds a buffer binding that stores dispatch-shared values in constant memory.
	pub fn constant_buffer_binding(name: &'a str, r#type: BindingResource<'a>, slot: u32, read: bool, write: bool) -> Node<'a> {
		Self::binding_with_count(name, r#type, slot, read, write, Some(BufferMemoryClass::Constant), None)
	}

	fn binding_with_count(
		name: &'a str,
		r#type: BindingResource<'a>,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: Option<BufferMemoryClass>,
		count: Option<NonZeroU32>,
	) -> Node<'a> {
		Node {
			node: Nodes::Binding {
				name,
				r#type,
				slot,
				read,
				write,
				memory_class,
				count,
			},
		}
	}

	pub fn binding_array(
		name: &'a str,
		r#type: BindingResource<'a>,
		slot: u32,
		read: bool,
		write: bool,
		count: u32,
	) -> Node<'a> {
		let count = NonZeroU32::new(count).expect(
			"Invalid binding array count. The most likely cause is that a resource array was declared with zero elements.",
		);
		Self::binding_with_count(name, r#type, slot, read, write, None, Some(count))
	}

	pub fn specialization(name: &'a str, r#type: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Specialization { name, r#type },
		}
	}

	/// Describes a buffer resource for [`Node::binding`] whose contents are `members`.
	pub fn buffer(members: Vec<Node<'a>>) -> BindingResource<'a> {
		BindingResource::Buffer { members }
	}

	/// Describes a storage image resource with texel `format` for [`Node::binding`].
	pub fn image(format: &'a str) -> BindingResource<'a> {
		BindingResource::Image { format }
	}

	pub fn push_constant(members: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::PushConstant { members },
		}
	}

	/// Describes a sampled 2D texture resource for [`Node::binding`].
	pub fn combined_image_sampler() -> BindingResource<'a> {
		BindingResource::CombinedImageSampler { format: "" }
	}

	/// Describes a sampled layered 2D texture resource for [`Node::binding`].
	pub fn combined_array_image_sampler() -> BindingResource<'a> {
		BindingResource::CombinedImageSampler {
			format: "ArrayTexture2D",
		}
	}

	/// Describes a sampled cube texture resource for [`Node::binding`].
	pub fn combined_cube_image_sampler() -> BindingResource<'a> {
		BindingResource::CombinedImageSampler { format: "TextureCube" }
	}

	/// Describes a sampled cube-array texture resource for [`Node::binding`].
	pub fn combined_cube_array_image_sampler() -> BindingResource<'a> {
		BindingResource::CombinedImageSampler {
			format: "TextureCubeArray",
		}
	}

	pub fn r#macro(name: &'a str, body: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Macro {
				name,
				body: Box::new(body),
			}),
		}
	}

	pub fn sentence(expressions: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Expression(expressions)),
		}
	}

	pub fn expression(elements: Vec<Node<'a>>) -> Node<'a> {
		Self::sentence(elements)
	}

	pub fn accessor(left: Node<'a>, right: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Accessor {
				left: Box::new(left),
				right: Box::new(right),
			}),
		}
	}

	pub fn call(name: &'a str, parameters: Vec<Node<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Call {
				name: TypeName::Named(name),
				parameters,
			}),
		}
	}

	/// Builds `left token right`, where `token` is a BESL operator such as `+` or `=`.
	///
	/// # Panics
	///
	/// Panics if `token` is not a BESL operator. The most likely cause is a generator that spelled an operator BESL
	/// does not define, such as `^`.
	pub fn operator(token: &str, left: Node<'a>, right: Node<'a>) -> Node<'a> {
		let operator = crate::Operators::from_token(token).unwrap_or_else(|| {
			panic!(
				"Invalid BESL operator `{token}`. The most likely cause is a generator that spelled an operator BESL does not define."
			)
		});
		Node {
			node: Nodes::Expression(Expressions::Operator {
				operator,
				left: Box::new(left),
				right: Box::new(right),
			}),
		}
	}

	pub fn assignment(left: Node<'a>, right: Node<'a>) -> Node<'a> {
		Self::operator("=", left, right)
	}

	/// Builds a typed local declaration.
	///
	/// Generated programs may own their local names, while parsed programs continue to borrow source text.
	pub fn variable_declaration(name: impl Into<Cow<'a, str>>, r#type: &'a str) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::VariableDeclaration {
				name: name.into(),
				r#type: TypeName::Named(r#type),
			}),
		}
	}

	pub fn literal_expression(value: impl Into<Cow<'a, str>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Literal { value: value.into() }),
		}
	}

	/// Builds an anonymous record value from named field expressions.
	pub fn record_literal(fields: Vec<RecordField<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::RecordLiteral { fields }),
		}
	}

	pub fn return_value(value: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Return {
				value: Some(Box::new(value)),
			}),
		}
	}

	pub fn return_void() -> Node<'a> {
		Node {
			node: Nodes::Expression(Expressions::Return { value: None }),
		}
	}

	pub fn let_assignment(name: impl Into<Cow<'a, str>>, r#type: &'a str, value: Node<'a>) -> Node<'a> {
		Self::assignment(Self::variable_declaration(name, r#type), value)
	}

	pub fn member_assignment(name: &'a str, value: Node<'a>) -> Node<'a> {
		Self::assignment(Self::member_expression(name), value)
	}

	pub fn glsl(code: impl Into<Cow<'a, str>>, input: &'a [&'a str], output: &'a [&'a str]) -> Node<'a> {
		Self::raw_code(Some(code.into()), None, None, input, output)
	}

	pub fn hlsl(code: impl Into<Cow<'a, str>>, input: &'a [&'a str], output: &'a [&'a str]) -> Node<'a> {
		Self::raw_code(None, Some(code.into()), None, input, output)
	}

	pub fn msl(code: impl Into<Cow<'a, str>>, input: &'a [&'a str], output: &'a [&'a str]) -> Node<'a> {
		Self::raw_code(None, None, Some(code.into()), input, output)
	}

	/// Builds parser raw code with explicit backend sources and interface names.
	pub fn raw_code(
		glsl: Option<Cow<'a, str>>,
		hlsl: Option<Cow<'a, str>>,
		msl: Option<Cow<'a, str>>,
		input: &'a [&'a str],
		output: &'a [&'a str],
	) -> Node<'a> {
		Node {
			node: Nodes::RawCode {
				glsl,
				hlsl,
				msl,
				input,
				output,
			},
		}
	}

	pub fn input(name: &'a str, format: &'a str, location: u8) -> Node<'a> {
		Node {
			node: Nodes::Input { name, format, location },
		}
	}

	pub fn output(name: &'a str, format: &'a str, location: u8) -> Node<'a> {
		Self::output_array(name, format, location, None, false)
	}

	/// Declares a mesh output array of `count` elements, with one element per vertex, which rasterization
	/// interpolates, when `per_vertex` is set, or one flat element per primitive otherwise. Without a `count` it
	/// declares a plain stage output, like [`Node::output`].
	pub fn output_array(
		name: &'a str,
		format: &'a str,
		location: u8,
		count: Option<NonZeroUsize>,
		per_vertex: bool,
	) -> Node<'a> {
		Node {
			node: Nodes::Output {
				name,
				format,
				location,
				count,
				per_vertex,
			},
		}
	}

	pub fn task_payload(name: &'a str, format: &'a str, count: NonZeroUsize) -> Node<'a> {
		Node {
			node: Nodes::TaskPayload { name, format, count },
		}
	}

	pub fn workgroup(name: &'a str, format: &'a str, count: Option<NonZeroUsize>) -> Node<'a> {
		Node {
			node: Nodes::Workgroup { name, format, count },
		}
	}

	pub fn intrinsic(name: &'a str, parameters: Node<'a>, body: Node<'a>, r#return: &'a str) -> Node<'a> {
		Self::intrinsic_with_parameters(name, vec![parameters], body, r#return)
	}

	/// Builds an intrinsic whose portable signature has more than one parameter.
	pub fn intrinsic_with_parameters(name: &'a str, parameters: Vec<Node<'a>>, body: Node<'a>, r#return: &'a str) -> Node<'a> {
		let mut elements = parameters;
		elements.push(body);
		Node {
			node: Nodes::Intrinsic {
				name,
				elements,
				r#return,
			},
		}
	}

	pub fn parameter(name: &'a str, r#type: impl Into<TypeName<'a>>) -> Node<'a> {
		Node {
			node: Nodes::Parameter {
				name,
				r#type: r#type.into(),
			},
		}
	}

	pub fn constant(name: &'a str, r#type: impl Into<TypeName<'a>>, value: Node<'a>) -> Node<'a> {
		Node {
			node: Nodes::Const {
				name,
				r#type: r#type.into(),
				value: Box::new(value),
			},
		}
	}

	pub fn name(&self) -> Option<&'a str> {
		match &self.node {
			Nodes::Scope { name, .. } => Some(name),
			Nodes::Struct { name, .. } => Some(name),
			Nodes::Member { name, .. } => Some(name),
			Nodes::Function { name, .. } => Some(name),
			Nodes::Conditional { .. } | Nodes::Match { .. } | Nodes::ForLoop { .. } => None,
			Nodes::Binding { name, .. } => Some(name),
			Nodes::Descriptor { name, .. } => Some(name),
			Nodes::Specialization { name, .. } => Some(name),
			Nodes::Expression(_) => None,
			Nodes::RawCode { .. } => None,
			Nodes::Intrinsic { name, .. } => Some(name),
			Nodes::Parameter { name, .. } => Some(name),
			Nodes::PushConstant { .. } => None,
			Nodes::Input { name, .. }
			| Nodes::Output { name, .. }
			| Nodes::TaskPayload { name, .. }
			| Nodes::Workgroup { name, .. } => Some(name),
			Nodes::Const { name, .. } => Some(name),
		}
	}

	pub fn node_mut(&mut self) -> &mut Nodes<'a> {
		// TODO: maybe do not expose nodes
		&mut self.node
	}

	pub fn node(&self) -> &Nodes<'a> {
		&self.node
	}

	pub fn get_mut(&mut self, name: &str) -> Option<&mut Node<'a>> {
		match &mut self.node {
			Nodes::Scope { children, .. } => children.iter_mut().find(|n| n.name() == Some(name)),
			_ => None,
		}
	}

	pub fn add(&mut self, children: Vec<Node<'a>>) {
		if let Nodes::Scope { children: c, .. } = &mut self.node {
			c.extend(children);
		} else {
			println!("Tried to add children to a non-scope node.");
		}
	}

	/// Places each scope's `main` function last. The stable sort keeps the other declarations in source order.
	pub(crate) fn sort(&mut self) {
		if let Nodes::Scope { children, .. } = &mut self.node {
			children.sort_by_key(|child| child.name() == Some("main"));
			children.iter_mut().for_each(|n| n.sort());
		}
	}
}

/// The `BindingResource` enum describes what a [`Nodes::Binding`] built by engine code gives shaders access to, so
/// the lexer can link every binding kind in one place.
#[derive(Clone, Debug)]
pub enum BindingResource<'a> {
	/// A buffer whose contents are `members`, which shaders read through the binding name.
	Buffer { members: Vec<Node<'a>> },
	/// A storage image with texel `format`.
	Image { format: &'a str },
	/// A sampled texture. `format` names the texture shape, or is empty for a 2D texture.
	CombinedImageSampler { format: &'a str },
}

#[derive(Clone, Debug)]
pub enum Nodes<'a> {
	/// A named group of BESL declarations, similar to a Rust module.
	Scope {
		/// The name used for imports and namespaces.
		name: &'a str,
		children: Vec<Node<'a>>,
	},
	/// A struct declaration and its fields.
	Struct {
		name: &'a str,
		fields: Vec<Node<'a>>,
	},
	/// A field declared in a struct.
	Member {
		name: &'a str,
		r#type: String,
	},
	/// A function declaration and body.
	Function {
		name: &'a str,
		params: Vec<Node<'a>>,
		return_type: TypeName<'a>,
		statements: Vec<Node<'a>>,
	},
	/// An `if` statement, with an optional `else` or `else if` branch.
	Conditional {
		condition: Box<Node<'a>>,
		statements: Vec<Node<'a>>,
		else_branch: Option<ElseBranch<'a>>,
	},
	/// A `match` statement over a scalar value. It runs the first arm whose pattern matches.
	Match {
		scrutinee: Box<Node<'a>>,
		arms: Vec<MatchArm<'a>>,
	},
	ForLoop {
		initializer: Box<Node<'a>>,
		condition: Box<Node<'a>>,
		update: Box<Node<'a>>,
		statements: Vec<Node<'a>>,
	},
	/// A shader resource binding built by engine code. Source declares resources as [`Nodes::Descriptor`].
	Binding {
		name: &'a str,
		r#type: BindingResource<'a>,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: Option<BufferMemoryClass>,
		count: Option<NonZeroU32>,
	},
	/// A named resource descriptor declared directly in BESL source.
	Descriptor {
		name: &'a str,
		resource_type: &'a str,
		runtime_array: bool,
		format: Option<&'a str>,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: Option<BufferMemoryClass>,
		count: Option<NonZeroU32>,
	},
	/// A constant selected when the application creates a pipeline.
	Specialization {
		name: &'a str,
		r#type: &'a str,
	},
	/// A small constant buffer updated during rendering.
	PushConstant {
		members: Vec<Node<'a>>,
	},
	Expression(Expressions<'a>),
	RawCode {
		glsl: Option<Cow<'a, str>>,
		hlsl: Option<Cow<'a, str>>,
		msl: Option<Cow<'a, str>>,
		input: &'a [&'a str],
		output: &'a [&'a str],
	},
	Intrinsic {
		name: &'a str,
		elements: Vec<Node<'a>>,
		r#return: &'a str,
	},
	Input {
		name: &'a str,
		format: &'a str,
		location: u8,
	},
	Output {
		name: &'a str,
		format: &'a str,
		location: u8,
		count: Option<NonZeroUsize>,
		/// Whether a mesh output array holds one element per vertex, interpolated across each triangle, instead of
		/// one flat element per primitive.
		per_vertex: bool,
	},
	/// An array carried from a task shader invocation group to the mesh work it emits.
	TaskPayload {
		name: &'a str,
		format: &'a str,
		count: NonZeroUsize,
	},
	/// Storage shared by all invocations in one task or compute workgroup.
	Workgroup {
		name: &'a str,
		format: &'a str,
		count: Option<NonZeroUsize>,
	},
	Parameter {
		name: &'a str,
		r#type: TypeName<'a>,
	},
	/// A named module-level value known at compile time.
	Const {
		name: &'a str,
		r#type: TypeName<'a>,
		value: Box<Node<'a>>,
	},
}

#[derive(Clone, Debug)]
pub enum Expressions<'a> {
	Expression(Vec<Node<'a>>),
	Accessor {
		left: Box<Node<'a>>,
		right: Box<Node<'a>>,
	},
	Member {
		name: Cow<'a, str>,
	},
	Literal {
		value: Cow<'a, str>,
	},
	RecordLiteral {
		fields: Vec<RecordField<'a>>,
	},
	Call {
		name: TypeName<'a>,
		parameters: Vec<Node<'a>>,
	},
	Operator {
		operator: crate::Operators,
		left: Box<Node<'a>>,
		right: Box<Node<'a>>,
	},
	VariableDeclaration {
		name: Cow<'a, str>,
		r#type: TypeName<'a>,
	},
	RawCode {
		glsl: Option<&'a str>,
		hlsl: Option<&'a str>,
		msl: Option<&'a str>,
		input: &'a [&'a str],
		output: &'a [&'a str],
	},
	Macro {
		name: &'a str,
		body: Box<Node<'a>>,
	},
	Return {
		value: Option<Box<Node<'a>>>,
	},
	Continue,
	Break,
	Discard,
}

#[derive(Clone, Debug)]
pub(super) enum Atoms<'a> {
	Keyword,
	Continue,
	Break,
	Discard,
	Accessor,
	GroupedExpression(Vec<Atoms<'a>>),
	Member {
		name: &'a str,
	},
	Literal {
		value: &'a str,
	},
	RecordLiteral {
		fields: Vec<AtomRecordField<'a>>,
	},
	FunctionCall {
		name: TypeName<'a>,
		parameters: Vec<Vec<Atoms<'a>>>,
	},
	Operator {
		operator: crate::Operators,
	},
	VariableDeclaration {
		name: &'a str,
		r#type: TypeName<'a>,
	},
}

/// The `AtomRecordField` struct preserves a record field until expression lowering builds its syntax node.
#[derive(Clone, Debug)]
pub(super) struct AtomRecordField<'a> {
	pub name: &'a str,
	pub value: Option<Vec<Atoms<'a>>>,
}

#[derive(Debug)]
pub enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax {
		message: String,
	},
	StreamEndedPrematurely,
}

impl std::fmt::Display for ParsingFailReasons {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ParsingFailReasons::NotMine => write!(f, "Parser cannot handle this syntax."),
			ParsingFailReasons::BadSyntax { message } => write!(f, "Bad syntax: {}", message),
			ParsingFailReasons::StreamEndedPrematurely => write!(f, "Token stream ended prematurely."),
		}
	}
}

impl ParsingFailReasons {
	/// Turns a [`ParsingFailReasons::NotMine`] refusal into a syntax error with `message`, for a token the current
	/// parser requires once it has recognized its syntax. Other errors pass through unchanged.
	pub(super) fn claimed(self, message: impl FnOnce() -> String) -> Self {
		match self {
			Self::NotMine => Self::BadSyntax { message: message() },
			error => error,
		}
	}
}

/// The result type returned by a syntax parser.
pub(super) type FeatureParserResult<'i, 'a> = Result<(Node<'a>, std::slice::Iter<'i, &'a str>), ParsingFailReasons>;

/// A function that tries to parse a token sequence.
pub(super) type FeatureParser<'i, 'a> = fn(std::slice::Iter<'i, &'a str>) -> FeatureParserResult<'i, 'a>;

/// The result of an expression parser: where it stopped reading tokens.
pub(super) type ExpressionParserResult<'i, 'a> = Result<std::slice::Iter<'i, &'a str>, ParsingFailReasons>;
/// A function that tries to parse expression tokens, appending the atoms it reads to the accumulator.
pub(super) type ExpressionParser<'i, 'a> =
	fn(std::slice::Iter<'i, &'a str>, &mut Vec<Atoms<'a>>) -> ExpressionParserResult<'i, 'a>;
