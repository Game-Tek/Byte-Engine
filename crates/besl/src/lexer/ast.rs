//! Resolves parsed BESL syntax into a linked semantic tree for compilation.

use std::hash::Hash;
use std::{
	cell::RefCell,
	num::{NonZeroU32, NonZeroUsize},
	ops::Deref,
	rc::{Rc, Weak},
};

use super::lowering::lex_parsed_node;
use super::resolution::{find_descendant, DescendantSearch};
use crate::parser;

pub type ParentNodeReference = Weak<RefCell<Node>>;

#[derive(Clone)]
pub struct NodeReference(pub(super) Rc<RefCell<Node>>);

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

	/// Returns the stable pointer identity used to deduplicate linked semantic nodes without borrowing their contents.
	pub(crate) fn identity(&self) -> usize {
		Rc::as_ptr(&self.0) as usize
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

pub(crate) fn lex(mut node: parser::Node) -> Result<NodeReference, LexError> {
	node.sort();
	lex_with_root(Node::root(), node)
}

pub(crate) fn lex_with_root(root: Node, mut node: parser::Node) -> Result<NodeReference, LexError> {
	node.sort();

	let root: NodeReference = root.into();

	match &node.node {
		parser::Nodes::Scope { name, children } => {
			assert_eq!(*name, "root");

			for child in children {
				let c = lex_parsed_node(vec![root.clone()], child)?;
				root.borrow_mut().add_child(c);
			}

			Ok(root)
		}
		_ => Err(LexError::Undefined { message: None }),
	}
}

#[derive(Clone)]
pub struct Node {
	// parent: Option<ParentNodeReference>,
	pub(super) node: Nodes,
}

impl Node {
	pub(super) fn internal_new(node: Node) -> NodeReference {
		NodeReference(Rc::new(RefCell::new(node)))
	}

	/// Creates the single root node that owns a program's other nodes.
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

		let builtins = vec![
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
				vec![("texture", texture_3d.clone()), ("uv", vec3f32.clone())],
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
				"dot",
				vec![("left", vec2f32.clone()), ("right", vec2f32.clone())],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"dot",
				vec![("left", vec4f32.clone()), ("right", vec4f32.clone())],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"dot",
				vec![("left", vec3f32.clone()), ("right", vec3f32.clone())],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"dot",
				vec![("left", vec2f16.clone()), ("right", vec2f16.clone())],
				f16_t.clone(),
			),
			builtin_intrinsic(
				"dot",
				vec![("left", vec3f16.clone()), ("right", vec3f16.clone())],
				f16_t.clone(),
			),
			builtin_intrinsic(
				"dot",
				vec![("left", vec4f16.clone()), ("right", vec4f16.clone())],
				f16_t.clone(),
			),
			builtin_intrinsic(
				"cross",
				vec![("left", vec3f32.clone()), ("right", vec3f32.clone())],
				vec3f32.clone(),
			),
			builtin_intrinsic("length", vec![("value", vec4f32.clone())], f32_t.clone()),
			builtin_intrinsic("length", vec![("value", vec3f32.clone())], f32_t.clone()),
			builtin_intrinsic("length", vec![("value", vec2f16.clone())], f16_t.clone()),
			builtin_intrinsic("length", vec![("value", vec3f16.clone())], f16_t.clone()),
			builtin_intrinsic("length", vec![("value", vec4f16.clone())], f16_t.clone()),
			builtin_intrinsic("normalize", vec![("value", vec4f32.clone())], vec4f32.clone()),
			builtin_intrinsic("normalize", vec![("value", vec3f32.clone())], vec3f32.clone()),
			builtin_intrinsic("normalize", vec![("value", vec2f16.clone())], vec2f16.clone()),
			builtin_intrinsic("normalize", vec![("value", vec3f16.clone())], vec3f16.clone()),
			builtin_intrinsic("normalize", vec![("value", vec4f16.clone())], vec4f16.clone()),
			builtin_intrinsic("max", vec![("left", f32_t.clone()), ("right", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("min", vec![("left", f32_t.clone()), ("right", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("max", vec![("left", f16_t.clone()), ("right", f16_t.clone())], f16_t.clone()),
			builtin_intrinsic("min", vec![("left", f16_t.clone()), ("right", f16_t.clone())], f16_t.clone()),
			builtin_intrinsic(
				"max",
				vec![("left", vec2f32.clone()), ("right", vec2f32.clone())],
				vec2f32.clone(),
			),
			builtin_intrinsic(
				"max",
				vec![("left", vec3f32.clone()), ("right", vec3f32.clone())],
				vec3f32.clone(),
			),
			builtin_intrinsic(
				"clamp",
				vec![
					("value", f32_t.clone()),
					("minimum", f32_t.clone()),
					("maximum", f32_t.clone()),
				],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"clamp",
				vec![
					("value", f16_t.clone()),
					("minimum", f16_t.clone()),
					("maximum", f16_t.clone()),
				],
				f16_t.clone(),
			),
			builtin_intrinsic(
				"clamp",
				vec![
					("value", vec3f32.clone()),
					("minimum", vec3f32.clone()),
					("maximum", vec3f32.clone()),
				],
				vec3f32.clone(),
			),
			builtin_intrinsic("log2", vec![("value", vec3f32.clone())], vec3f32.clone()),
			builtin_intrinsic(
				"pow",
				vec![("value", vec3f32.clone()), ("exponent", vec3f32.clone())],
				vec3f32.clone(),
			),
			builtin_intrinsic(
				"pow",
				vec![("value", f32_t.clone()), ("exponent", f32_t.clone())],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"pow",
				vec![("value", f16_t.clone()), ("exponent", f16_t.clone())],
				f16_t.clone(),
			),
			builtin_intrinsic(
				"reflect",
				vec![("incident", vec4f32.clone()), ("normal", vec4f32.clone())],
				vec4f32.clone(),
			),
			builtin_intrinsic("abs", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("abs", vec![("value", vec2f32.clone())], vec2f32.clone()),
			builtin_intrinsic("sqrt", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("abs", vec![("value", f16_t.clone())], f16_t.clone()),
			builtin_intrinsic("abs", vec![("value", vec2f16.clone())], vec2f16.clone()),
			builtin_intrinsic("sqrt", vec![("value", f16_t.clone())], f16_t.clone()),
			builtin_intrinsic("exp", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("exp", vec![("value", vec3f32.clone())], vec3f32.clone()),
			builtin_intrinsic("sin", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("cos", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("asin", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("atan2", vec![("y", f32_t.clone()), ("x", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("floor", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("sincos", vec![("value", f32_t.clone())], vec2f32.clone()),
			builtin_intrinsic("tan", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("round", vec![("value", vec2f32.clone())], vec2f32.clone()),
			builtin_intrinsic("round", vec![("value", f16_t.clone())], f16_t.clone()),
			builtin_intrinsic("round", vec![("value", vec2f16.clone())], vec2f16.clone()),
			builtin_intrinsic("round_to_i32", vec![("value", vec2f32.clone())], vec2i32.clone()),
			builtin_intrinsic(
				"fma",
				vec![
					("multiplicand", f32_t.clone()),
					("multiplier", f32_t.clone()),
					("addend", f32_t.clone()),
				],
				f32_t.clone(),
			),
			builtin_intrinsic(
				"fma",
				vec![
					("multiplicand", vec2f32.clone()),
					("multiplier", vec2f32.clone()),
					("addend", vec2f32.clone()),
				],
				vec2f32.clone(),
			),
			builtin_intrinsic(
				"fma",
				vec![
					("multiplicand", vec3f32.clone()),
					("multiplier", vec3f32.clone()),
					("addend", vec3f32.clone()),
				],
				vec3f32.clone(),
			),
			builtin_intrinsic(
				"fma",
				vec![
					("multiplicand", vec4f32.clone()),
					("multiplier", vec4f32.clone()),
					("addend", vec4f32.clone()),
				],
				vec4f32.clone(),
			),
			builtin_intrinsic("fract", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("fwidth", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("radians", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("inversesqrt", vec![("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic("f16", vec![("value", f32_t.clone())], f16_t.clone()),
			builtin_intrinsic("f16", vec![("value", f16_t.clone())], f16_t.clone()),
			builtin_intrinsic("f16", vec![("value", u32_t.clone())], f16_t.clone()),
			builtin_intrinsic("f16", vec![("value", i32_t.clone())], f16_t.clone()),
			builtin_intrinsic("u16", vec![("value", u32_t.clone())], u16_t.clone()),
			builtin_intrinsic("f32", vec![("value", f16_t.clone())], f32_t.clone()),
			builtin_intrinsic("f32", vec![("value", u32_t.clone())], f32_t.clone()),
			builtin_intrinsic("f32", vec![("value", i32_t.clone())], f32_t.clone()),
			builtin_intrinsic("vec2f16", vec![("value", vec2f32.clone())], vec2f16.clone()),
			builtin_intrinsic("vec2f16", vec![("value", vec2f16.clone())], vec2f16.clone()),
			builtin_intrinsic("vec3f16", vec![("value", vec3f32.clone())], vec3f16.clone()),
			builtin_intrinsic("vec3f16", vec![("value", vec3f16.clone())], vec3f16.clone()),
			builtin_intrinsic("vec4f16", vec![("value", vec4f32.clone())], vec4f16.clone()),
			builtin_intrinsic("vec4f16", vec![("value", vec4f16.clone())], vec4f16.clone()),
			builtin_intrinsic("vec2f", vec![("value", vec2f16.clone())], vec2f32.clone()),
			builtin_intrinsic("vec3f", vec![("value", vec3f16.clone())], vec3f32.clone()),
			builtin_intrinsic("vec4f", vec![("value", vec4f16.clone())], vec4f32.clone()),
			builtin_intrinsic("packed_vec4f", vec![("value", vec4f32.clone())], packed_vec4f32.clone()),
			builtin_intrinsic("vec4f", vec![("value", packed_vec4f32)], vec4f32.clone()),
			builtin_intrinsic("u32", vec![("value", u32_t.clone())], u32_t.clone()),
			builtin_intrinsic("u32", vec![("value", u8_t.clone())], u32_t.clone()),
			builtin_intrinsic("u32", vec![("value", u16_t.clone())], u32_t.clone()),
			builtin_intrinsic("u32", vec![("value", i32_t)], u32_t.clone()),
			builtin_intrinsic("u32", vec![("value", f16_t)], u32_t.clone()),
			builtin_intrinsic("u32", vec![("value", f32_t.clone())], u32_t.clone()),
			builtin_intrinsic(
				"smoothstep",
				vec![("edge0", f32_t.clone()), ("edge1", f32_t.clone()), ("value", f32_t.clone())],
				f32_t.clone(),
			),
			builtin_intrinsic("step", vec![("edge", f32_t.clone()), ("value", f32_t.clone())], f32_t.clone()),
			builtin_intrinsic(
				"mix",
				vec![("left", f32_t.clone()), ("right", f32_t.clone()), ("factor", f32_t.clone())],
				f32_t.clone(),
			),
			builtin_intrinsic("thread_idx", vec![], u32_t.clone()),
			builtin_intrinsic("subgroup_lane_index", vec![], u32_t.clone()),
			builtin_intrinsic("threadgroup_position", vec![], u32_t.clone()),
			builtin_intrinsic("thread_position", vec![], u32_t.clone()),
			builtin_intrinsic("subgroup_ballot", vec![("predicate", bool_t.clone())], vec4u32.clone()),
			builtin_intrinsic("subgroup_ballot_any", vec![("mask", vec4u32.clone())], bool_t.clone()),
			builtin_intrinsic("subgroup_ballot_find_lsb", vec![("mask", vec4u32.clone())], u32_t.clone()),
			builtin_intrinsic("subgroup_ballot_count", vec![("mask", vec4u32.clone())], u32_t.clone()),
			builtin_intrinsic(
				"subgroup_ballot_and_not",
				vec![("mask", vec4u32.clone()), ("removed", vec4u32.clone())],
				vec4u32.clone(),
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
				vec![("primitive_index", u32_t.clone()), ("triangle", vec3u32.clone())],
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
			builtin_intrinsic(
				"atomic_add",
				vec![("value", atomic_u32.clone()), ("increment", u32_t.clone())],
				u32_t.clone(),
			),
			builtin_intrinsic(
				"atomic_compare_exchange",
				vec![
					("value", atomic_u32.clone()),
					("expected", u32_t.clone()),
					("desired", u32_t.clone()),
				],
				u32_t.clone(),
			),
			builtin_intrinsic("atomic_load", vec![("value", atomic_u32.clone())], u32_t.clone()),
			builtin_intrinsic(
				"atomic_store",
				vec![("value", atomic_u32), ("stored", u32_t.clone())],
				void.clone(),
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
				vec![
					("image", texture_2d.clone()),
					("coord", vec2u32.clone()),
					("value", u32_t.clone()),
				],
				u32_t.clone(),
			),
		];

		let mut root = Node::scope("root".to_string());
		root.add_children(builtins);

		root
	}

	/// Creates a scope that groups child nodes.
	pub fn scope(name: String) -> Node {
		Node {
			// parent: None,
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
			| Nodes::Literal { name, .. }
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
			Nodes::Expression(Expressions::IntrinsicCall { arguments, elements, .. }) => {
				let mut children = Vec::with_capacity(arguments.len() + elements.len());
				children.extend(arguments.iter().cloned());
				children.extend(elements.iter().cloned());
				Some(children)
			}
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

	pub(super) fn sentence(elements: Vec<NodeReference>) -> Node {
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

/// The `BufferMemoryClass` enum selects the memory region that best matches a buffer's shader access pattern.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BufferMemoryClass {
	/// Use constant memory for small values shared by the dispatch or draw.
	#[default]
	Constant,
	/// Use device memory for large data that varies between shader threads.
	Device,
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
	Literal {
		name: String,
		value: NodeReference,
	},
	/// A named module-level value known at compile time.
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
			Nodes::Input { .. } | Nodes::Output { .. } | Nodes::TaskPayload { .. } | Nodes::Workgroup { .. } => false,
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
		fn type_is_indexable(r#type: &NodeReference) -> bool {
			let r#type = r#type.borrow();
			matches!(r#type.node(), Nodes::Struct { template: Some(_), .. })
				|| r#type
					.get_name()
					.is_some_and(|name| name.starts_with("vec") || name.starts_with("mat"))
		}

		match self {
			Nodes::Member { r#type, count, .. } => count.is_some() || type_is_indexable(r#type),
			Nodes::Input { format, .. } => type_is_indexable(format),
			Nodes::Output { format, count, .. } => count.is_some() || type_is_indexable(format),
			Nodes::TaskPayload { .. } => true,
			Nodes::Workgroup { count, format, .. } => count.is_some() || type_is_indexable(format),
			Nodes::Parameter { r#type, .. }
			| Nodes::Specialization { r#type, .. }
			| Nodes::Const { r#type, .. }
			| Nodes::Expression(Expressions::VariableDeclaration { r#type, .. }) => type_is_indexable(r#type),
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
					input.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string())),
					output.iter().map(|c| c.0.borrow().get_name().map(|e| e.to_string()))
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
	Inequality,
	GreaterThan,
	LessThanOrEqual,
	GreaterThanOrEqual,
	LogicalAnd,
	LogicalOr,
}

#[derive(Clone, Debug)]
pub enum Expressions {
	Return {
		value: Option<NodeReference>,
	},
	Continue,
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
