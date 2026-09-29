//! Resolves parsed BESL syntax into a linked semantic tree for compilation.

use std::collections::HashSet;
use std::hash::Hash;
use std::{
	cell::RefCell,
	num::{NonZeroU32, NonZeroUsize},
	ops::Deref,
	rc::{Rc, Weak},
};

use super::lowering::Lexer;
use super::resolution::{DescendantSearch, find_descendant};
use crate::parser;

#[derive(Clone)]
pub struct NodeReference(pub(super) Rc<RefCell<Node>>);

impl std::fmt::Debug for NodeReference {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.borrow().fmt(f)
	}
}

impl NodeReference {
	/// Recursively searches for a child node with the given name.
	pub fn get_descendant(&self, child_name: &str) -> Option<NodeReference> {
		find_descendant(self, child_name, DescendantSearch::Any)
	}

	/// Returns the stable pointer identity used to deduplicate linked semantic nodes without borrowing their contents.
	pub(crate) fn identity(&self) -> usize {
		Rc::as_ptr(&self.0) as usize
	}

	/// Returns the program's `main` entry-point function.
	///
	/// Only functions count, and the search walks nested scopes, never struct fields or function bodies, so a member
	/// or local named `main` is never returned.
	pub fn get_main(&self) -> Option<NodeReference> {
		let node = self.borrow();
		match node.node() {
			Nodes::Function { name, .. } if name == "main" => Some(self.clone()),
			Nodes::Scope { children, .. } => children.iter().find_map(NodeReference::get_main),
			_ => None,
		}
	}

	/// Returns `self`, a function such as `main`, followed by every function it calls directly or transitively, each
	/// once, in first-call order.
	///
	/// Compilers use it to find the functions they must lower. It follows calls through [`Nodes::children`], but not
	/// through inlined intrinsic bodies or macro bodies, which backends expand on their own.
	pub(crate) fn reachable_functions(&self) -> Vec<NodeReference> {
		/// Adds `function` and walks its body once, so recursion and repeated calls add nothing new.
		fn visit_function(function: &NodeReference, seen: &mut HashSet<usize>, functions: &mut Vec<NodeReference>) {
			if !seen.insert(function.identity()) {
				return;
			}
			functions.push(function.clone());
			visit_calls(function, seen, functions);
		}

		fn visit_calls(node: &NodeReference, seen: &mut HashSet<usize>, functions: &mut Vec<NodeReference>) {
			let node = node.borrow();
			let children = match node.node() {
				Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
					let function = function.get();
					if matches!(function.borrow().node(), Nodes::Function { .. }) {
						visit_function(&function, seen, functions);
					}
					parameters.as_slice()
				}
				// Intrinsic bodies and macro bodies are expanded by each backend, so calls inside them are not followed.
				Nodes::Expression(Expressions::IntrinsicCall { arguments, .. }) => arguments.as_slice(),
				Nodes::Expression(Expressions::Macro { .. }) => &[],
				other => {
					for child in other.children() {
						visit_calls(child, seen, functions);
					}
					&[]
				}
			};
			for child in children {
				visit_calls(child, seen, functions);
			}
		}

		let mut functions = Vec::new();
		visit_function(self, &mut HashSet::new(), &mut functions);
		functions
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

/// The `CallTarget` struct links a function call to the declaration it calls without letting recursion leak the tree.
///
/// A call to a [`Nodes::Function`] holds a weak reference, because the function's own statements may contain the call,
/// and a strong reference would form an `Rc` cycle that is never freed. Calls to anything else, such as a type
/// constructor or an array type created on demand, keep a strong reference, because the call may be that node's only
/// owner. Read the target with [`CallTarget::get`].
#[derive(Clone)]
pub struct CallTarget(CallTargetLink);

#[derive(Clone)]
enum CallTargetLink {
	Function(Weak<RefCell<Node>>),
	Owned(NodeReference),
}

impl CallTarget {
	/// Returns the called declaration.
	///
	/// # Panics
	///
	/// Panics if the called function was dropped before the call. The program tree owns every function, so keep the
	/// tree alive while you use its call nodes.
	pub fn get(&self) -> NodeReference {
		match &self.0 {
			CallTargetLink::Function(function) => NodeReference(function.upgrade().expect(
				"Called function no longer exists. The most likely cause is that the program tree was dropped while one of its call nodes was still in use.",
			)),
			CallTargetLink::Owned(target) => target.clone(),
		}
	}
}

impl From<NodeReference> for CallTarget {
	fn from(target: NodeReference) -> Self {
		let is_function = matches!(target.borrow().node(), Nodes::Function { .. });
		if is_function {
			CallTarget(CallTargetLink::Function(Rc::downgrade(&target.0)))
		} else {
			CallTarget(CallTargetLink::Owned(target))
		}
	}
}

impl std::fmt::Debug for CallTarget {
	// Print only the name: a recursive function's body contains this call, so printing the whole node would not end.
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let target = self.get();
		let target = target.borrow();
		write!(f, "CallTarget({:?})", target.get_name())
	}
}

/// Links a parsed program under `root`, which supplies the built-in registry from [`Node::root`] and any generated
/// declarations the program may reference.
pub(crate) fn lex_with_root(root: Node, mut node: parser::Node) -> Result<NodeReference, LexError> {
	node.sort();

	let root: NodeReference = root.into();

	match &mut node.node {
		parser::Nodes::Scope { name, children } => {
			assert_eq!(*name, "root");

			let mut lexer = Lexer::new(root.clone());
			for child in children {
				for declaration in super::entry::normalize_entry(child, &root)? {
					root.borrow_mut().add_child(declaration.into());
				}
				let child = lexer.lex(child)?;
				root.borrow_mut().add_child(child);
			}

			Ok(root)
		}
		_ => Err(LexError::invalid(
			"Invalid program root: the parsed node is not a scope. The most likely cause is that the node passed to the lexer is not the root that besl::parse returns.",
		)),
	}
}

#[derive(Clone)]
pub struct Node {
	pub(super) node: Nodes,
}

impl Node {
	/// Creates the single root node that owns a program's other nodes.
	// Keep the built-in registry contiguous so overload ordering and shared type handles remain auditable together.
	#[allow(clippy::too_many_lines)]
	pub fn root() -> Node {
		let void = primitive_type("void");
		let bool_t = primitive_type("bool");
		let u8_t = primitive_type("u8");
		let u16_t = primitive_type("u16");
		let u32_t = primitive_type("u32");
		let i32_t = primitive_type("i32");
		let f16_t = primitive_type("f16");
		let f32_t = primitive_type("f32");

		let vec2u16 = record_type("vec2u16", [("x", u16_t.clone()), ("y", u16_t.clone())]);
		let vec4u16 = record_type(
			"vec4u16",
			[
				("x", u16_t.clone()),
				("y", u16_t.clone()),
				("z", u16_t.clone()),
				("w", u16_t.clone()),
			],
		);
		let vec2u32 = record_type("vec2u", [("x", u32_t.clone()), ("y", u32_t.clone())]);
		let vec2i32 = record_type("vec2i", [("x", i32_t.clone()), ("y", i32_t.clone())]);
		let vec2f16 = record_type("vec2f16", [("x", f16_t.clone()), ("y", f16_t.clone())]);
		let vec2f32 = record_type("vec2f", [("x", f32_t.clone()), ("y", f32_t.clone())]);
		let vec3f16 = record_type("vec3f16", [("x", f16_t.clone()), ("y", f16_t.clone()), ("z", f16_t.clone())]);
		let vec3f32 = record_type("vec3f", [("x", f32_t.clone()), ("y", f32_t.clone()), ("z", f32_t.clone())]);
		let vec3u32 = record_type("vec3u", [("x", u32_t.clone()), ("y", u32_t.clone()), ("z", u32_t.clone())]);
		let vec4u32 = record_type(
			"vec4u",
			[
				("x", u32_t.clone()),
				("y", u32_t.clone()),
				("z", u32_t.clone()),
				("w", u32_t.clone()),
			],
		);
		let vec4f16 = record_type(
			"vec4f16",
			[
				("x", f16_t.clone()),
				("y", f16_t.clone()),
				("z", f16_t.clone()),
				("w", f16_t.clone()),
			],
		);
		let vec4f32 = record_type(
			"vec4f",
			[
				("x", f32_t.clone()),
				("y", f32_t.clone()),
				("z", f32_t.clone()),
				("w", f32_t.clone()),
			],
		);
		// Packed vectors keep scalar alignment when they are embedded in storage records.
		let packed_vec4f32 = record_type(
			"packed_vec4f",
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
		let mat4x3f32 = record_type(
			"mat4x3f",
			[
				("x", vec3f32.clone()),
				("y", vec3f32.clone()),
				("z", vec3f32.clone()),
				("w", vec3f32.clone()),
			],
		);

		let texture_2d = primitive_type("Texture2D");
		let texture_3d = primitive_type("Texture3D");
		let texture_cube = primitive_type("TextureCube");
		let texture_cube_array = primitive_type("TextureCubeArray");
		let array_texture_2d = primitive_type("ArrayTexture2D");
		let atomic_u32 = primitive_type("atomicu32");
		let atomic_i32 = primitive_type("atomici32");

		let mut builtins = vec![
			void.clone(),
			bool_t.clone(),
			u8_t.clone(),
			u16_t.clone(),
			u32_t.clone(),
			i32_t.clone(),
			f16_t.clone(),
			f32_t.clone(),
			vec2u16,
			vec4u16,
			vec2u32.clone(),
			vec2i32.clone(),
			vec2f16.clone(),
			vec2f32.clone(),
			vec3u32.clone(),
			vec3f16.clone(),
			vec3f32.clone(),
			vec4u32.clone(),
			vec4f16.clone(),
			vec4f32.clone(),
			packed_vec4f32.clone(),
			mat4f32,
			mat4x3f32,
			texture_2d.clone(),
			texture_3d.clone(),
			texture_cube.clone(),
			texture_cube_array.clone(),
			array_texture_2d.clone(),
			atomic_u32.clone(),
			atomic_i32.clone(),
			// Vertex invocation indices are implicit BESL values. Their placeholder locations are
			// never exposed as vertex attributes; backends and the VM map them to dedicated built-ins.
			Node::input(crate::VERTEX_INDEX_BUILTIN, u32_t.clone(), u8::MAX - 1).into(),
			Node::input(crate::INSTANCE_INDEX_BUILTIN, u32_t.clone(), u8::MAX).into(),
			builtin_intrinsic(
				"sample",
				vec![("texture_sampler", texture_2d.clone()), ("uv", vec2f32.clone())],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"texture_lod",
				vec![("texture", texture_2d.clone()), ("uv", vec2f32.clone())],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"texture_lod",
				vec![
					("texture", texture_2d.clone()),
					("uv", vec2f32.clone()),
					("lod", f32_t.clone()),
				],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"texture_lod",
				vec![("texture", texture_3d), ("uv", vec3f32.clone())],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"texture_lod",
				vec![
					("texture", texture_cube),
					("direction", vec3f32.clone()),
					("lod", f32_t.clone()),
				],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"texture_cube_array_lod",
				vec![
					("texture", texture_cube_array),
					("direction", vec3f32.clone()),
					("cube", u32_t.clone()),
					("lod", f32_t.clone()),
				],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"downsample_min",
				vec![
					("texture", texture_2d.clone()),
					("uv", vec2f32.clone()),
					("lod", f32_t.clone()),
				],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"downsample_max",
				vec![
					("texture", texture_2d.clone()),
					("uv", vec2f32.clone()),
					("lod", f32_t.clone()),
				],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"downsample_max",
				vec![
					("texture", array_texture_2d.clone()),
					("uv", vec2f32.clone()),
					("layer", u32_t.clone()),
					("lod", f32_t.clone()),
				],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"fetch",
				vec![("texture", texture_2d.clone()), ("coord", vec2u32.clone())],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"fetch",
				vec![
					("texture", array_texture_2d.clone()),
					("coord", vec2u32.clone()),
					("layer", u32_t.clone()),
				],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"fetch_u32",
				vec![("texture", texture_2d.clone()), ("coord", vec2u32.clone())],
				u32_t.clone(),
			),
			builtin_intrinsic(
				"cross",
				vec![("left", vec3f32.clone()), ("right", vec3f32.clone())],
				vec3f32.clone(),
			),
			builtin_intrinsic("find_lsb", vec![("value", u32_t.clone())], u32_t.clone()),
			builtin_intrinsic("atan2", vec![("y", f32_t.clone()), ("x", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("sincos", vec![("value", f32_t.clone())], vec2f32.clone()),
			builtin_intrinsic("round_to_i32", vec![("value", vec2f32.clone())], vec2i32),
			builtin_intrinsic(
				"smoothstep",
				vec![("edge0", f32_t.clone()), ("edge1", f32_t.clone()), ("value", f32_t.clone())],
				f32_t.clone(),
			),
			builtin_intrinsic("step", vec![("edge", f32_t.clone()), ("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("subgroup_ballot", vec![("predicate", bool_t.clone())], vec4u32.clone()),
			builtin_intrinsic("subgroup_ballot_any", vec![("mask", vec4u32.clone())], bool_t.clone()),
			builtin_intrinsic("subgroup_ballot_find_lsb", vec![("mask", vec4u32.clone())], u32_t.clone()),
			builtin_intrinsic("subgroup_ballot_count", vec![("mask", vec4u32.clone())], u32_t.clone()),
			builtin_intrinsic(
				"subgroup_ballot_and_not",
				vec![("mask", vec4u32.clone()), ("removed", vec4u32.clone())],
				vec4u32,
			),
			builtin_intrinsic(
				"subgroup_broadcast_u32",
				vec![("value", u32_t.clone()), ("source_lane", u32_t.clone())],
				u32_t.clone(),
			),
			builtin_intrinsic(
				"subgroup_broadcast_f32",
				vec![("value", f32_t.clone()), ("source_lane", u32_t.clone())],
				f32_t.clone(),
			),
			builtin_intrinsic("workgroup_barrier", vec![], void.clone()),
			builtin_intrinsic("set_task_mesh_output_count", vec![("count", u32_t.clone())], void.clone()),
			builtin_intrinsic("thread_id", vec![], vec2u32.clone()),
			builtin_intrinsic(
				"set_mesh_output_counts",
				vec![("vertex_count", u32_t.clone()), ("primitive_count", u32_t.clone())],
				void.clone(),
			),
			builtin_intrinsic(
				"set_mesh_vertex_position",
				vec![("vertex_index", u32_t.clone()), ("position", vec4f32.clone())],
				void.clone(),
			),
			builtin_intrinsic(
				"set_mesh_triangle",
				vec![("primitive_index", u32_t.clone()), ("triangle", vec3u32)],
				void.clone(),
			),
			builtin_intrinsic(
				"set_mesh_primitive_render_target_array_index",
				vec![("primitive_index", u32_t.clone()), ("array_index", u32_t.clone())],
				void.clone(),
			),
			builtin_intrinsic(
				"image_load",
				vec![("image", texture_2d.clone()), ("coord", vec2u32.clone())],
				vec4f32.clone(),
			),
			builtin_intrinsic(
				"image_load_u32",
				vec![("image", texture_2d.clone()), ("coord", vec2u32.clone())],
				u32_t.clone(),
			),
			builtin_intrinsic("texture_size", vec![("texture", texture_2d.clone())], vec2u32.clone()),
			builtin_intrinsic("texture_size", vec![("texture", array_texture_2d)], vec2u32.clone()),
			builtin_intrinsic("image_size", vec![("image", texture_2d.clone())], vec2u32.clone()),
			builtin_intrinsic(
				"guard_image_bounds",
				vec![("image", texture_2d.clone()), ("coord", vec2u32.clone())],
				void.clone(),
			),
			builtin_intrinsic(
				"write",
				vec![
					("image", texture_2d.clone()),
					("coord", vec2u32.clone()),
					("value", vec4f32.clone()),
				],
				void.clone(),
			),
			builtin_intrinsic(
				"image_atomic_or",
				vec![("image", texture_2d), ("coord", vec2u32), ("value", u32_t.clone())],
				u32_t.clone(),
			),
		];

		// Families whose overloads differ only in type. Each list keeps its name's overload order, because call
		// resolution selects the first overload whose parameters match.
		builtins.extend(converted_overloads(
			"dot",
			&["left", "right"],
			[
				(&vec2f32, &f32_t),
				(&vec4f32, &f32_t),
				(&vec3f32, &f32_t),
				(&vec2f16, &f16_t),
				(&vec3f16, &f16_t),
				(&vec4f16, &f16_t),
			],
		));
		builtins.extend(converted_overloads(
			"length",
			&["value"],
			[
				(&vec4f32, &f32_t),
				(&vec3f32, &f32_t),
				(&vec2f32, &f32_t),
				(&vec2f16, &f16_t),
				(&vec3f16, &f16_t),
				(&vec4f16, &f16_t),
			],
		));
		builtins.extend(same_type_overloads(
			"normalize",
			&["value"],
			[&vec4f32, &vec3f32, &vec2f32, &vec2f16, &vec3f16, &vec4f16],
		));
		builtins.extend(same_type_overloads(
			"max",
			&["left", "right"],
			[&f32_t, &f16_t, &i32_t, &u32_t, &vec2f32, &vec3f32],
		));
		builtins.extend(same_type_overloads(
			"min",
			&["left", "right"],
			[&f32_t, &f16_t, &i32_t, &u32_t],
		));
		builtins.extend(same_type_overloads(
			"clamp",
			&["value", "minimum", "maximum"],
			[&f32_t, &f16_t, &i32_t, &u32_t, &vec3f32],
		));
		builtins.extend(same_type_overloads("log2", &["value"], [&vec3f32, &f32_t]));
		builtins.extend(same_type_overloads("pow", &["value", "exponent"], [&vec3f32, &f32_t, &f16_t]));
		builtins.extend(same_type_overloads("reflect", &["incident", "normal"], [&vec4f32]));
		builtins.extend(same_type_overloads("abs", &["value"], [&f32_t, &vec2f32, &f16_t, &vec2f16]));
		builtins.extend(same_type_overloads("sqrt", &["value"], [&f32_t, &f16_t]));
		for predicate in ["is_nan", "is_infinite", "is_finite", "is_normal"] {
			builtins.extend(converted_overloads(
				predicate,
				&["value"],
				[(&f16_t, &bool_t), (&f32_t, &bool_t)],
			));
		}
		builtins.extend(same_type_overloads("exp", &["value"], [&f32_t, &vec3f32]));
		for name in [
			"sin",
			"cos",
			"asin",
			"floor",
			"tan",
			"fract",
			"fwidth",
			"radians",
			"inversesqrt",
		] {
			builtins.extend(same_type_overloads(name, &["value"], [&f32_t]));
		}
		builtins.extend(same_type_overloads("round", &["value"], [&f32_t, &vec2f32, &f16_t, &vec2f16]));
		builtins.extend(same_type_overloads(
			"fma",
			&["multiplicand", "multiplier", "addend"],
			[&f32_t, &vec2f32, &vec3f32, &vec4f32, &f16_t, &vec2f16, &vec3f16, &vec4f16],
		));
		// `mix` blends two same-typed values by one scalar factor.
		for value in [&f32_t, &vec2f32, &vec3f32, &vec4f32] {
			builtins.push(builtin_intrinsic(
				"mix",
				vec![("left", value.clone()), ("right", value.clone()), ("factor", f32_t.clone())],
				value.clone(),
			));
		}
		// Conversions take one `value` of the source type and return the target type.
		let conversions: [(&str, &[&NodeReference], &NodeReference); 10] = [
			("f16", &[&f32_t, &f16_t, &u32_t, &i32_t], &f16_t),
			("u16", &[&u32_t], &u16_t),
			("f32", &[&f16_t, &u32_t, &i32_t], &f32_t),
			("vec2f16", &[&vec2f32, &vec2f16], &vec2f16),
			("vec3f16", &[&vec3f32, &vec3f16], &vec3f16),
			("vec4f16", &[&vec4f32, &vec4f16], &vec4f16),
			("vec2f", &[&vec2f16], &vec2f32),
			("vec3f", &[&vec3f16], &vec3f32),
			("vec4f", &[&vec4f16, &packed_vec4f32], &vec4f32),
			("packed_vec4f", &[&vec4f32], &packed_vec4f32),
		];
		for (name, sources, target) in conversions {
			builtins.extend(converted_overloads(
				name,
				&["value"],
				sources.iter().map(|source| (*source, target)),
			));
		}
		builtins.extend(converted_overloads(
			"u32",
			&["value"],
			[&u32_t, &u8_t, &u16_t, &i32_t, &f16_t, &f32_t].map(|source| (source, &u32_t)),
		));
		// Invocation builtins take no arguments and read the invocation's coordinates.
		for name in ["thread_idx", "subgroup_lane_index", "threadgroup_position", "thread_position"] {
			builtins.push(builtin_intrinsic(name, vec![], u32_t.clone()));
		}
		builtins.extend(atomic_intrinsics(atomic_u32, u32_t, void.clone()));
		builtins.extend(atomic_intrinsics(atomic_i32, i32_t, void));

		let mut root = Node::scope("root".to_string());
		root.add_children(builtins);

		root
	}

	/// Creates a scope that groups child nodes.
	pub fn scope(name: String) -> Node {
		Node {
			node: Nodes::Scope {
				name,
				children: Vec::with_capacity(16),
			},
		}
	}

	/// Creates a named struct definition from its fields.
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
		Node {
			node: Nodes::Member {
				name: name.to_string(),
				r#type,
				count: Some(NonZeroUsize::new(size).expect("Invalid size")),
			},
		}
		.into()
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

	/// Builds an `if` statement. Pass `None` as `else_branch` for an `if` without an `else` branch.
	pub fn conditional(condition: NodeReference, statements: Vec<NodeReference>, else_branch: Option<ElseBranch>) -> Node {
		Node {
			node: Nodes::Conditional {
				condition,
				statements,
				else_branch,
			},
		}
	}

	/// Builds a `match` statement over a scalar `scrutinee` of type `type`.
	/// The labels of `arms` must be distinct, and `default` runs for every value no arm lists.
	/// Use [`crate::compile_to_besl`] to build it from source, which checks and normalizes Rust `match` semantics.
	pub fn r#match(scrutinee: NodeReference, r#type: NodeReference, arms: Vec<MatchArm>, default: Vec<NodeReference>) -> Node {
		Node {
			node: Nodes::Match {
				scrutinee,
				r#type,
				arms,
				default,
			},
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
		Self::raw(Some(code), None, None, inputs, outputs)
	}

	pub fn hlsl(code: String, inputs: Vec<NodeReference>, outputs: Vec<NodeReference>) -> Node {
		Self::raw(None, Some(code), None, inputs, outputs)
	}

	pub fn msl(code: String, inputs: Vec<NodeReference>, outputs: Vec<NodeReference>) -> Node {
		Self::raw(None, None, Some(code), inputs, outputs)
	}

	/// Builds linked raw code with explicit backend sources and interface nodes.
	pub fn raw(
		glsl: Option<String>,
		hlsl: Option<String>,
		msl: Option<String>,
		inputs: Vec<NodeReference>,
		outputs: Vec<NodeReference>,
	) -> Node {
		Node {
			node: Nodes::Raw {
				glsl,
				hlsl,
				msl,
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

	/// Builds a device-backed binding. Use [`Self::binding_in_memory`] for dispatch-shared constant data.
	pub fn binding(name: &str, r#type: BindingTypes, slot: u32, read: bool, write: bool) -> Node {
		Self::binding_in_memory(name, r#type, slot, read, write, BufferMemoryClass::Device)
	}

	/// Builds a binding whose memory class is independent from its read and write access.
	pub fn binding_in_memory(
		name: &str,
		r#type: BindingTypes,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: BufferMemoryClass,
	) -> Node {
		Self::binding_with_count(name, r#type, slot, read, write, memory_class, None)
	}

	pub(super) fn binding_with_count(
		name: &str,
		r#type: BindingTypes,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: BufferMemoryClass,
		count: Option<NonZeroU32>,
	) -> Node {
		// A descriptor array of buffers keeps its wrapper struct per resource; only single buffers are lowered.
		let r#type = if count.is_none() {
			r#type.lowered_single_array()
		} else {
			r#type
		};
		Node {
			node: Nodes::Binding {
				name: name.to_string(),
				r#type,
				slot,
				read,
				write,
				memory_class,
				count,
			},
		}
	}

	pub fn binding_array(name: &str, r#type: BindingTypes, slot: u32, read: bool, write: bool, count: usize) -> Node {
		Self::binding_array_in_memory(name, r#type, slot, read, write, BufferMemoryClass::Device, count)
	}

	/// Builds a resource array whose buffer memory class is independent from its read and write access.
	pub fn binding_array_in_memory(
		name: &str,
		r#type: BindingTypes,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: BufferMemoryClass,
		count: usize,
	) -> Node {
		let count = u32::try_from(count)
			.expect("Invalid binding array count. The most likely cause is that a resource array exceeds u32::MAX elements.");
		let count = NonZeroU32::new(count).expect(
			"Invalid binding array count. The most likely cause is that a resource array was declared with zero elements.",
		);
		Self::binding_with_count(name, r#type, slot, read, write, memory_class, Some(count))
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

	pub fn task_payload(name: &str, format: NodeReference, count: u32) -> Node {
		let count = NonZeroUsize::new(count as usize).expect(
			"Invalid task-payload count. The most likely cause is that a task-payload array was declared with zero elements.",
		);
		Node {
			node: Nodes::TaskPayload {
				name: name.to_string(),
				format,
				count,
			},
		}
	}

	pub fn workgroup(name: &str, format: NodeReference, count: Option<NonZeroUsize>) -> Node {
		Node {
			node: Nodes::Workgroup {
				name: name.to_string(),
				format,
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
			| Nodes::Const { name, .. } => Some(name),
			Nodes::Input { name, .. }
			| Nodes::Output { name, .. }
			| Nodes::TaskPayload { name, .. }
			| Nodes::Workgroup { name, .. } => Some(name),
			Nodes::PushConstant { .. } => Some("push_constant"),
			Nodes::Expression(Expressions::VariableDeclaration { name, .. } | Expressions::Member { name, .. }) => Some(name),
			_ => None,
		}
	}

	/// Returns the direct child named `child_name`, such as a function in a scope or a field in a struct.
	///
	/// Declaration containers (scopes, structs and intrinsics) search their declarations. Every other node searches
	/// the executable parts that [`Nodes::children`] yields.
	pub fn get_child(&self, child_name: &str) -> Option<NodeReference> {
		let declarations: &[NodeReference] = match &self.node {
			Nodes::Scope { children, .. }
			| Nodes::Struct { fields: children, .. }
			| Nodes::Intrinsic { elements: children, .. } => children,
			_ => &[],
		};
		declarations
			.iter()
			.chain(self.node.children())
			.find(|child| child.borrow().get_name() == Some(child_name))
			.cloned()
	}

	pub fn node_mut(&mut self) -> &mut Nodes {
		&mut self.node
	}
}

/// The `FixedArray` struct keeps what a fixed-size array buffer was declared with, so shaders can keep indexing it
/// through its wrapper member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixedArray {
	pub count: NonZeroUsize,
	/// The wrapper member's name. `binding.alias[i]` reads `binding[i]`.
	pub alias: String,
}

impl BindingTypes {
	/// Lowers a buffer whose only member is a fixed array into [`BindingTypes::BufferArray`].
	fn lowered_single_array(self) -> Self {
		let lowered = match &self {
			Self::Buffer { members } => match members.as_slice() {
				[member] => match member.borrow().node() {
					Nodes::Member {
						name,
						r#type,
						count: Some(count),
					} => Some(Self::BufferArray {
						element: r#type.clone(),
						fixed: Some(FixedArray {
							count: *count,
							alias: name.clone(),
						}),
					}),
					_ => None,
				},
				_ => None,
			},
			_ => None,
		};
		lowered.unwrap_or(self)
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingTypes {
	Buffer {
		members: Vec<NodeReference>,
	},
	/// A storage buffer holding a flat array of `element` values.
	///
	/// Runtime arrays (`type: T[]`) have no `fixed` size; the bound resource supplies their length. A buffer declared
	/// as a struct whose only member is a fixed array, such as `Meshes: struct { meshes: Mesh[1024] }`, is lowered to
	/// this form, so every backend handles one array shape.
	BufferArray {
		element: NodeReference,
		fixed: Option<FixedArray>,
	},
	CombinedImageSampler {
		format: String,
	},
	Image {
		format: String,
	},
}

/// The `BufferMemoryClass` enum selects the memory region that best matches a buffer's shader access pattern.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BufferMemoryClass {
	/// Use constant memory for small values shared by the dispatch or draw.
	#[default]
	Constant,
	/// Use device memory for large data that varies between shader threads.
	Device,
}

/// The `ElseBranch` enum keeps `else if` chains distinct from plain `else` blocks,
/// so backends can lower each form by structure. See [`Nodes::Conditional`].
#[derive(Clone, Debug)]
pub enum ElseBranch {
	/// An `else { ... }` block.
	Block(Vec<NodeReference>),
	/// An `else if` link. The node is always a [`Nodes::Conditional`].
	If(NodeReference),
}

impl ElseBranch {
	/// Returns the branch as a statement list. An `else if` link is one conditional statement.
	/// Use it in walkers that treat both forms alike.
	pub fn statements(&self) -> &[NodeReference] {
		match self {
			Self::Block(statements) => statements,
			Self::If(conditional) => std::slice::from_ref(conditional),
		}
	}
}

/// The `MatchArm` struct holds one case of a [`Nodes::Match`], so backends can lower it to a `switch` case.
#[derive(Clone, Debug)]
pub struct MatchArm {
	/// The scalar values that select this arm. `bool` values are `0` and `1`.
	/// Labels are distinct across the arms of one match.
	pub values: Vec<i64>,
	pub statements: Vec<NodeReference>,
}

#[derive(Clone)]
pub enum Nodes {
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
	/// An `if` statement, with an optional `else` or `else if` branch.
	Conditional {
		condition: NodeReference,
		statements: Vec<NodeReference>,
		else_branch: Option<ElseBranch>,
	},
	/// A `match` statement over a `bool` or integer value.
	///
	/// The lexer resolves Rust's first-match-wins rule, so the arms' values are distinct and `default` holds the
	/// statements for every other value. Lower it to a `switch` whose `default` case runs `default`.
	Match {
		scrutinee: NodeReference,
		/// The scrutinee's type. It is `bool`, `u8`, `u16`, `u32`, or `i32`.
		r#type: NodeReference,
		arms: Vec<MatchArm>,
		default: Vec<NodeReference>,
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
		msl: Option<String>,
		input: Vec<NodeReference>,
		output: Vec<NodeReference>,
	},
	Binding {
		name: String,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: BufferMemoryClass,
		r#type: BindingTypes,
		count: Option<NonZeroU32>,
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
	TaskPayload {
		name: String,
		format: NodeReference,
		count: NonZeroUsize,
	},
	Workgroup {
		name: String,
		format: NodeReference,
		count: Option<NonZeroUsize>,
	},
	Parameter {
		name: String,
		r#type: NodeReference,
	},
	/// A named module-level value known at compile time.
	Const {
		name: String,
		r#type: NodeReference,
		value: NodeReference,
	},
}

impl Nodes {
	/// Iterates the executable parts this node owns, in source order: a function's statements, the parts of a branch or
	/// loop, a constant's value, and the operands of an expression.
	///
	/// Use it in AST walkers so they list only the nodes they treat specially and recurse into every other node alike.
	/// It never follows a reference to another declaration, such as the declaration a [`Expressions::Member`] reads, a
	/// call's target, or a declared type. Raw backend code and declaration containers such as scopes and structs
	/// yield nothing.
	pub fn children(&self) -> impl Iterator<Item = &NodeReference> {
		let (singles, first, second): ([Option<&NodeReference>; 3], &[NodeReference], &[NodeReference]) = match self {
			Nodes::Function { statements, .. } => ([None; 3], statements, &[]),
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => ([Some(initializer), Some(condition), Some(update)], statements, &[]),
			Nodes::Const { value, .. } => ([Some(value), None, None], &[], &[]),
			Nodes::Expression(expression) => match expression {
				Expressions::Return { value } => ([value.as_ref(), None, None], &[], &[]),
				Expressions::Expression { elements } => ([None; 3], elements, &[]),
				Expressions::FunctionCall { parameters, .. } => ([None; 3], parameters, &[]),
				Expressions::IntrinsicCall { arguments, elements, .. } => ([None; 3], arguments, elements),
				Expressions::Operator { left, right, .. } | Expressions::Accessor { left, right } => {
					([Some(left), Some(right), None], &[], &[])
				}
				Expressions::Macro { body, .. } => ([Some(body), None, None], &[], &[]),
				Expressions::Continue
				| Expressions::Break
				| Expressions::Discard
				| Expressions::Member { .. }
				| Expressions::Literal { .. }
				| Expressions::VariableDeclaration { .. } => ([None; 3], &[], &[]),
			},
			_ => ([None; 3], &[], &[]),
		};
		singles
			.into_iter()
			.flatten()
			.chain(first)
			.chain(second)
			.chain(self.branch_children())
	}

	/// Iterates every part of a branching statement: the condition, then the `if` and `else` statements of a
	/// [`Nodes::Conditional`], or the scrutinee, then the arm and default statements of a [`Nodes::Match`].
	/// Use it in AST walkers that treat every part of a branch alike, so they don't list its fields by hand.
	/// Returns an empty iterator for other nodes.
	pub fn branch_children(&self) -> impl Iterator<Item = &NodeReference> {
		let (head, statements, arms, tail): (_, &[_], &[MatchArm], &[_]) = match self {
			Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => (
				Some(condition),
				statements,
				&[],
				else_branch.as_ref().map_or(&[], ElseBranch::statements),
			),
			Nodes::Match {
				scrutinee,
				arms,
				default,
				..
			} => (Some(scrutinee), &[], arms, default),
			_ => (None, &[], &[], &[]),
		};
		head.into_iter()
			.chain(statements)
			.chain(arms.iter().flat_map(|arm| &arm.statements))
			.chain(tail)
	}

	pub fn is_leaf(&self) -> bool {
		match self {
			Nodes::Function { .. } => false,
			Nodes::Conditional { .. } | Nodes::Match { .. } | Nodes::ForLoop { .. } => false,
			Nodes::Struct { .. } => false,
			Nodes::Binding { .. } => false,
			Nodes::PushConstant { .. } => false,
			Nodes::Input { .. } | Nodes::Output { .. } | Nodes::TaskPayload { .. } | Nodes::Workgroup { .. } => false,
			Nodes::Specialization { .. } => false,
			Nodes::Const { .. } => false,
			Nodes::Parameter { .. } => true,
			Nodes::Scope { .. } => true,
			Nodes::Intrinsic { .. } => true,
			Nodes::Member { .. } => true,
			Nodes::Expression { .. } => true,
			Nodes::Raw { .. } => true,
		}
	}

	pub fn is_indexable(&self) -> bool {
		fn type_is_indexable(r#type: &NodeReference) -> bool {
			let r#type = r#type.borrow();
			matches!(r#type.node(), Nodes::Struct { template: Some(_), .. })
				|| r#type
					.get_name()
					.is_some_and(|name| name.starts_with("vec") || name.starts_with("mat"))
		}

		match self {
			Nodes::Binding {
				r#type: BindingTypes::BufferArray { .. },
				..
			} => true,
			// A descriptor array holds one resource per element.
			Nodes::Binding { count: Some(_), .. } => true,
			Nodes::Member { r#type, count, .. } => count.is_some() || type_is_indexable(r#type),
			Nodes::Input { format, .. } => type_is_indexable(format),
			Nodes::Output { format, count, .. } => count.is_some() || type_is_indexable(format),
			Nodes::TaskPayload { .. } => true,
			Nodes::Workgroup { count, format, .. } => count.is_some() || type_is_indexable(format),
			Nodes::Parameter { r#type, .. }
			| Nodes::Specialization { r#type, .. }
			| Nodes::Const { r#type, .. }
			| Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => type_is_indexable(r#type),
			Nodes::Expression(Expressions::Member { source, .. }) => match source.borrow().node() {
				Nodes::Binding {
					r#type: BindingTypes::CombinedImageSampler { format },
					count: None,
					..
				} => format == "ArrayTexture2D",
				_ => source.borrow().node().is_indexable(),
			},
			Nodes::Expression(Expressions::Accessor { right, .. }) => right.borrow().node().is_indexable(),
			_ => false,
		}
	}

	pub fn is_buffer_binding(&self) -> bool {
		match self {
			Nodes::Binding {
				r#type: BindingTypes::Buffer { .. } | BindingTypes::BufferArray { .. },
				..
			} => true,
			Nodes::Expression(Expressions::Member { source, .. }) => source.borrow().node().is_buffer_binding(),
			_ => false,
		}
	}
}

/// Collects the names of `nodes` so Debug output lists children by name instead of printing whole subtrees.
fn names(nodes: &[NodeReference]) -> Vec<Option<String>> {
	nodes
		.iter()
		.map(|node| node.borrow().get_name().map(str::to_string))
		.collect()
}

impl std::fmt::Debug for Node {
	// Every node variant is formatted here so Debug output stays exhaustive when the AST grows.
	#[allow(clippy::too_many_lines)]
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self.node {
			Nodes::Scope { name, children } => {
				write!(f, "Scope {{ name: {}, children: {:#?} }}", name, names(children))
			}
			Nodes::Struct { name, fields, .. } => {
				write!(f, "Struct {{ name: {}, fields: {:?} }}", name, names(fields))
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
					names(params),
					names(statements)
				)
			}
			Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => {
				write!(
					f,
					"Conditional {{ condition: {:?}, statements: {:?}, else_branch: {:?} }}",
					condition, statements, else_branch
				)
			}
			Nodes::Match {
				scrutinee,
				r#type,
				arms,
				default,
			} => {
				write!(
					f,
					"Match {{ scrutinee: {:?}, type: {:?}, arms: {:?}, default: {:?} }}",
					scrutinee,
					r#type.borrow().get_name(),
					arms,
					default
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
				msl,
				input,
				output,
			} => {
				write!(
					f,
					"RawCode {{ glsl: {:?}, hlsl: {:?}, msl: {:?}, input: {:?}, output: {:?} }}",
					glsl,
					hlsl,
					msl,
					names(input),
					names(output)
				)
			}
			Nodes::Binding {
				name,
				slot,
				read,
				write,
				memory_class,
				r#type,
				count,
			} => {
				write!(
					f,
					"Binding {{ name: {}, slot: {}, read: {}, write: {}, memory_class: {:?}, type: {:?}, count: {:?} }}",
					name, slot, read, write, memory_class, r#type, count
				)
			}
			Nodes::PushConstant { members } => {
				write!(f, "PushConstant {{ members: {:?} }}", names(members))
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
					names(elements),
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
			Nodes::TaskPayload { name, format, count } => {
				write!(
					f,
					"TaskPayload {{ name: {}, format: {:?}, count: {} }}",
					name,
					format.0.borrow().get_name().map(|e| e.to_string()),
					count
				)
			}
			Nodes::Workgroup { name, format, count } => {
				write!(
					f,
					"Workgroup {{ name: {}, format: {:?}, count: {:?} }}",
					name,
					format.0.borrow().get_name().map(|e| e.to_string()),
					count
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

/// The `Operators` enum names every BESL binary operator. The parser reads it from source tokens with
/// [`Operators::from_token`] and orders expressions by [`Operators::precedence`], and backends match it to emit
/// operator syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
	Inequality,
	GreaterThan,
	LessThanOrEqual,
	GreaterThanOrEqual,
	LogicalAnd,
	LogicalOr,
}

/// Pairs each operator with its source token and binding precedence. A lower precedence binds tighter.
const OPERATOR_TOKENS: [(&str, Operators, u8); 18] = [
	("=", Operators::Assignment, 8),
	("||", Operators::LogicalOr, 7),
	("|", Operators::BitwiseOr, 7),
	("&&", Operators::LogicalAnd, 6),
	("&", Operators::BitwiseAnd, 6),
	("==", Operators::Equality, 5),
	("!=", Operators::Inequality, 5),
	("<", Operators::LessThan, 5),
	(">", Operators::GreaterThan, 5),
	("<=", Operators::LessThanOrEqual, 5),
	(">=", Operators::GreaterThanOrEqual, 5),
	("<<", Operators::ShiftLeft, 4),
	(">>", Operators::ShiftRight, 4),
	("+", Operators::Plus, 3),
	("-", Operators::Minus, 3),
	("*", Operators::Multiply, 2),
	("/", Operators::Divide, 2),
	("%", Operators::Modulo, 2),
];

impl Operators {
	/// Reads the operator a source token spells, or returns `None` for any other token.
	pub fn from_token(token: &str) -> Option<Self> {
		OPERATOR_TOKENS
			.iter()
			.find(|(operator_token, ..)| *operator_token == token)
			.map(|(_, operator, _)| *operator)
	}

	/// Returns how loosely this operator binds. The parser splits an expression at its loosest operator first.
	pub fn precedence(&self) -> u8 {
		OPERATOR_TOKENS
			.iter()
			.find(|(_, operator, _)| operator == self)
			.map_or(0, |(.., precedence)| *precedence)
	}
}

#[derive(Clone, Debug)]
pub enum Expressions {
	Return {
		value: Option<NodeReference>,
	},
	Continue,
	/// Leaves the innermost enclosing loop.
	Break,
	Discard,
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
		function: CallTarget,
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

/// The `LexError` enum reports why a parsed program could not be linked, so callers can show the cause to shader
/// authors.
#[derive(Debug, PartialEq, Eq)]
pub enum LexError {
	/// A program that does not follow a BESL rule. The message names the rule and its most likely cause.
	Invalid {
		message: String,
	},
	FunctionCallParametersDoNotMatchFunctionParameters,
	AccessingUndeclaredMember {
		name: String,
	},
	ReferenceToUndefinedType {
		type_name: String,
	},
}

impl LexError {
	/// Reports a rule violation. Write `message` as a succinct error followed by its most likely cause.
	pub(crate) fn invalid(message: impl Into<String>) -> Self {
		LexError::Invalid { message: message.into() }
	}
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

/// Builds the relaxed scalar atomic surface for one signed or unsigned 32-bit type.
fn atomic_intrinsics(atomic: NodeReference, scalar: NodeReference, void: NodeReference) -> Vec<NodeReference> {
	let binary = |name: &str, operand_name: &str| {
		builtin_intrinsic(
			name,
			vec![("value", atomic.clone()), (operand_name, scalar.clone())],
			scalar.clone(),
		)
	};

	vec![
		builtin_intrinsic("atomic_load", vec![("value", atomic.clone())], scalar.clone()),
		builtin_intrinsic(
			"atomic_store",
			vec![("value", atomic.clone()), ("stored", scalar.clone())],
			void,
		),
		binary("atomic_exchange", "stored"),
		builtin_intrinsic(
			"atomic_compare_exchange",
			vec![
				("value", atomic.clone()),
				("expected", scalar.clone()),
				("desired", scalar.clone()),
			],
			scalar.clone(),
		),
		binary("atomic_add", "increment"),
		binary("atomic_sub", "decrement"),
		binary("atomic_min", "candidate"),
		binary("atomic_max", "candidate"),
		binary("atomic_and", "mask"),
		binary("atomic_or", "mask"),
		binary("atomic_xor", "mask"),
	]
}

/// Declares one `name` overload per `(operand, result)` signature, in order. Every parameter in `parameters` takes the
/// operand type. Keep each name's overloads in their intended order, because call resolution picks the first match.
fn converted_overloads<'a>(
	name: &str,
	parameters: &[&str],
	signatures: impl IntoIterator<Item = (&'a NodeReference, &'a NodeReference)>,
) -> Vec<NodeReference> {
	signatures
		.into_iter()
		.map(|(operand, result)| {
			builtin_intrinsic(
				name,
				parameters.iter().map(|parameter| (*parameter, operand.clone())).collect(),
				result.clone(),
			)
		})
		.collect()
}

/// Declares one `name` overload per type in `types`, where every parameter and the result share that type.
fn same_type_overloads<'a>(
	name: &str,
	parameters: &[&str],
	types: impl IntoIterator<Item = &'a NodeReference>,
) -> Vec<NodeReference> {
	converted_overloads(name, parameters, types.into_iter().map(|r#type| (r#type, r#type)))
}

/// Built-in scalar types with a byte representation in storage buffers. `bool`, `void`, and resource handles have none.
pub(crate) const STORABLE_SCALAR_TYPES: [&str; 6] = ["u8", "u16", "u32", "i32", "f16", "f32"];

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
