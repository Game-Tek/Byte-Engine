//! Resolves parsed BESL syntax into a linked semantic tree for compilation.

use std::num::{NonZeroU32, NonZeroUsize};
use std::ops::{Index, IndexMut};

use super::lowering::Lexer;
use super::resolution::{DescendantSearch, find_descendant};
use crate::parser;

/// The `NodeId` struct names one node of a linked [`Program`], so nodes can point at each other, such as a call at the
/// function it calls, without owning one another.
///
/// It is only meaningful for the program that issued it: read the node with `program[id]`. Two ids are equal when
/// they name the same node, which is what deduplication by identity needs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
	/// Returns the node's position in its program, a stable key for tables that index nodes densely.
	pub(crate) fn index(self) -> usize {
		self.0 as usize
	}
}

/// Returns the shared index of built-in names, which [`Program::builtins_named`] reads.
fn builtin_index() -> &'static rustc_hash::FxHashMap<String, Vec<NodeId>> {
	static INDEX: std::sync::OnceLock<rustc_hash::FxHashMap<String, Vec<NodeId>>> = std::sync::OnceLock::new();
	INDEX.get_or_init(|| {
		let program = Program::new();
		let mut index = rustc_hash::FxHashMap::<String, Vec<NodeId>>::default();
		for id in program.builtins() {
			if let Some(name) = program.name(*id) {
				index.entry(name.to_string()).or_default().push(*id);
			}
		}
		#[cfg(debug_assertions)]
		assert_builtins_hide_no_declarations(&program, &index);
		index
	})
}

/// Asserts what lets root lookups skip every built-in whose name differs from the one they look for: a lookup that
/// reaches into a built-in finds nothing that isn't itself a built-in by that name, and no built-in holds a declaration
/// a call could target.
///
/// A lookup can only return a node it reaches by name, and every node a built-in reaches is in `program`, so checking
/// each name `program` holds, other than the built-ins' own, covers every name a shader can look up.
#[cfg(debug_assertions)]
fn assert_builtins_hide_no_declarations(program: &Program, index: &rustc_hash::FxHashMap<String, Vec<NodeId>>) {
	use super::resolution::{DescendantSearch, find_descendant};

	let hidden: std::collections::BTreeSet<&str> = program
		.nodes
		.iter()
		.filter_map(Node::get_name)
		.filter(|name| !index.contains_key(*name))
		.collect();
	for id in program.builtins() {
		let node = program[id].node();
		let declares_calls = match node {
			Nodes::Scope { .. } | Nodes::PushConstant { .. } => true,
			Nodes::Struct { fields, .. } => fields.iter().any(|field| !matches!(program[field].node(), Nodes::Member { .. })),
			_ => false,
		};
		assert!(!declares_calls, "Built-in `{:?}` holds declarations a call could target.", program.name(*id));
		for name in &hidden {
			assert!(
				find_descendant(program, id, name, DescendantSearch::NonIntrinsic).is_none(),
				"Built-in `{:?}` lets a lookup reach `{name}`, which the built-in index can't answer for.",
				program.name(*id)
			);
		}
	}
}

/// The `NodeMap` type maps nodes of one program to values. Node ids are small dense integers, so it hashes them with
/// the Fx hash instead of the standard library's SipHash, which is built to resist untrusted keys.
pub type NodeMap<V> = std::collections::HashMap<NodeId, V, rustc_hash::FxBuildHasher>;

/// The `NodeSet` struct records which nodes of one program a walk has reached, with one bit per node.
///
/// Linking, reflection, and the shader backends walk programs whose nodes are shared, so each walk keeps a set of the
/// nodes it has visited. Node ids are dense, so a bit per node needs no hashing and one small allocation. Create one
/// with [`NodeSet::for_program`]; it grows when a walk adds nodes to the program.
#[derive(Clone, Debug, Default)]
pub struct NodeSet {
	words: Vec<u64>,
}

impl NodeSet {
	/// Creates an empty set with room for every node `program` holds now.
	pub fn for_program(program: &Program) -> Self {
		Self {
			words: vec![0; program.nodes.len().div_ceil(64)],
		}
	}

	/// Adds `node`, and returns whether it was absent, as [`std::collections::HashSet::insert`] does.
	pub fn insert(&mut self, node: NodeId) -> bool {
		let (word, bit) = (node.index() / 64, 1u64 << (node.index() % 64));
		if word >= self.words.len() {
			self.words.resize(word + 1, 0);
		}
		let absent = self.words[word] & bit == 0;
		self.words[word] |= bit;
		absent
	}

	/// Removes `node`, and returns whether it was present.
	pub fn remove(&mut self, node: NodeId) -> bool {
		let (word, bit) = (node.index() / 64, 1u64 << (node.index() % 64));
		let Some(word) = self.words.get_mut(word) else {
			return false;
		};
		let present = *word & bit != 0;
		*word &= !bit;
		present
	}

	/// Returns whether the set holds no node.
	pub fn is_empty(&self) -> bool {
		self.words.iter().all(|word| *word == 0)
	}

	/// Returns whether `node` is in the set.
	pub fn contains(&self, node: NodeId) -> bool {
		self.words
			.get(node.index() / 64)
			.is_some_and(|word| word & (1u64 << (node.index() % 64)) != 0)
	}
}

impl std::fmt::Debug for NodeId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "NodeId({})", self.0)
	}
}

/// The `Program` struct owns every node of one linked BESL program in a single arena, so linking allocates one growing
/// list instead of one shared cell per node, and dropping a program frees it at once.
///
/// Create one with [`Program::new`], which holds the built-in types and intrinsics under [`Program::root`], add any
/// declarations the source may reference, and pass it to [`crate::compile_to_besl`]. Next, hand the linked program to
/// a shader backend or to [`crate::vm::ExecutableProgram::compile`].
#[derive(Clone)]
pub struct Program {
	nodes: Vec<Node>,
	root: NodeId,
	/// How many of the root's first children are built-ins. The program's own declarations follow them, so built-in
	/// lookups stop here and never find a user type of the same name.
	builtins: usize,
}

impl Program {
	/// Returns the scope that holds the built-ins and, once linked, the program's declarations.
	pub fn root(&self) -> NodeId {
		self.root
	}

	/// Returns the built-in types and intrinsics that [`Program::new`] placed at the front of the root scope.
	pub(crate) fn builtins(&self) -> &[NodeId] {
		match self[self.root].node() {
			Nodes::Scope { children, .. } => &children[..self.builtins.min(children.len())],
			_ => &[],
		}
	}

	/// Returns the declarations of the root scope after the built-ins: the program's own, and the ones it imported.
	pub(crate) fn root_declarations(&self) -> &[NodeId] {
		match self[self.root].node() {
			Nodes::Scope { children, .. } => &children[self.builtins.min(children.len())..],
			_ => &[],
		}
	}

	/// Returns the built-ins named `name`, in the order the root scope declares them, so overload resolution sees
	/// them in the same order as a scan of the root would.
	///
	/// Every program's built-ins are the same nodes at the same ids, so one index, built the first time any program
	/// asks, answers for all of them. Lookups that reach the root take this instead of comparing against each of the
	/// about one thousand built-ins.
	pub(crate) fn builtins_named(&self, name: &str) -> &'static [NodeId] {
		if self.builtins == 0 {
			return &[];
		}
		let named = builtin_index().get(name).map_or(&[][..], Vec::as_slice);
		debug_assert!(
			named.iter().all(|id| self.builtins().contains(id) && self.name(*id) == Some(name)),
			"The built-ins of this program don't match the shared built-in index."
		);
		named
	}

	/// Moves `node` into the program and returns its id. The node is not reachable until a container holds the id;
	/// see [`Node::add_child`].
	pub fn add(&mut self, mut node: Node) -> NodeId {
		// A buffer whose only member is a fixed array is stored as a flat array binding, so every backend handles one
		// array shape. A descriptor array of buffers keeps its wrapper struct per resource; only single buffers lower.
		if let Nodes::Binding { r#type, count: None, .. } = &mut node.node {
			let lowered = self.lowered_single_array(std::mem::replace(r#type, BindingTypes::AccelerationStructure));
			*r#type = lowered;
		}
		let id = NodeId(u32::try_from(self.nodes.len()).expect(
			"A BESL program exceeded u32::MAX nodes. The most likely cause is runaway code generation. Split the program.",
		));
		self.nodes.push(node);
		id
	}

	/// Adds `node` to the root scope and returns its id, for declarations a host provides before compiling.
	pub fn add_to_root(&mut self, node: Node) -> NodeId {
		let id = self.add(node);
		let root = self.root;
		self[root].add_child(id);
		id
	}

	/// Returns the name of node `id`, if it has one.
	pub fn name(&self, id: NodeId) -> Option<&str> {
		self[id].get_name()
	}

	/// Returns the direct child of `parent` named `child_name`, such as a function in a scope or a field in a struct.
	///
	/// Declaration containers (scopes, structs and intrinsics) search their declarations. Every other node searches
	/// the executable parts that [`Nodes::children`] yields.
	pub fn get_child(&self, parent: NodeId, child_name: &str) -> Option<NodeId> {
		let node = &self[parent];
		let declarations: &[NodeId] = match node.node() {
			Nodes::Scope { children, .. }
			| Nodes::Struct { fields: children, .. }
			| Nodes::Intrinsic { elements: children, .. } => children,
			_ => &[],
		};
		declarations
			.iter()
			.chain(node.node().children())
			.find(|child| self.name(**child) == Some(child_name))
			.copied()
	}

	/// Recursively searches under `parent` for a node named `child_name`.
	pub fn get_descendant(&self, parent: NodeId, child_name: &str) -> Option<NodeId> {
		find_descendant(self, &parent, child_name, DescendantSearch::Any)
	}

	/// Returns the program's `main` entry-point function.
	///
	/// Only functions count, and the search walks nested scopes, never struct fields or function bodies, so a member
	/// or local named `main` is never returned.
	pub fn get_main(&self) -> Option<NodeId> {
		self.find_main(self.root)
	}

	fn find_main(&self, id: NodeId) -> Option<NodeId> {
		match self[id].node() {
			Nodes::Function { name, .. } if name == "main" => Some(id),
			Nodes::Scope { children, .. } => children.iter().find_map(|child| self.find_main(*child)),
			_ => None,
		}
	}

	/// Returns `function`, such as `main`, followed by every function it calls directly or transitively, each once, in
	/// first-call order.
	///
	/// Compilers use it to find the functions they must lower. It follows calls through [`Nodes::children`], but not
	/// through inlined intrinsic bodies or macro bodies, which backends expand on their own.
	pub(crate) fn reachable_functions(&self, function: NodeId) -> Vec<NodeId> {
		/// Adds `function` and walks its body once, so recursion and repeated calls add nothing new.
		fn visit_function(program: &Program, function: NodeId, seen: &mut NodeSet, functions: &mut Vec<NodeId>) {
			if !seen.insert(function) {
				return;
			}
			functions.push(function);
			visit_calls(program, function, seen, functions);
		}

		fn visit_calls(program: &Program, node: NodeId, seen: &mut NodeSet, functions: &mut Vec<NodeId>) {
			let children = match program[node].node() {
				Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
					if matches!(program[function].node(), Nodes::Function { .. }) {
						visit_function(program, *function, seen, functions);
					}
					parameters.as_slice()
				}
				// Intrinsic bodies and macro bodies are expanded by each backend, so calls inside them are not followed.
				Nodes::Expression(Expressions::IntrinsicCall { arguments, .. }) => arguments.as_slice(),
				Nodes::Expression(Expressions::Macro { .. }) => &[],
				other => {
					for child in other.children() {
						visit_calls(program, *child, seen, functions);
					}
					&[]
				}
			};
			for child in children {
				visit_calls(program, *child, seen, functions);
			}
		}

		let mut functions = Vec::new();
		visit_function(self, function, &mut NodeSet::for_program(self), &mut functions);
		functions
	}

	/// Lowers a buffer whose only member is a fixed array into [`BindingTypes::BufferArray`].
	fn lowered_single_array(&self, r#type: BindingTypes) -> BindingTypes {
		if let BindingTypes::Buffer { members } = &r#type
			&& let [member] = members.as_slice()
			&& let Nodes::Member {
				name,
				r#type: element,
				count: Some(count),
			} = self[member].node()
		{
			return BindingTypes::BufferArray {
				element: *element,
				fixed: Some(FixedArray {
					count: *count,
					alias: name.clone(),
				}),
			};
		}
		r#type
	}

	/// Returns a view of node `id` whose `Debug` output names the nodes it refers to, for error messages and tests.
	pub fn debug(&self, id: NodeId) -> DebugNode<'_> {
		DebugNode { program: self, id }
	}
}

impl Default for Program {
	fn default() -> Self {
		Self::new()
	}
}

impl Index<NodeId> for Program {
	type Output = Node;

	fn index(&self, id: NodeId) -> &Node {
		&self.nodes[id.index()]
	}
}

impl IndexMut<NodeId> for Program {
	fn index_mut(&mut self, id: NodeId) -> &mut Node {
		&mut self.nodes[id.index()]
	}
}

impl Index<&NodeId> for Program {
	type Output = Node;

	fn index(&self, id: &NodeId) -> &Node {
		&self.nodes[id.index()]
	}
}

impl IndexMut<&NodeId> for Program {
	fn index_mut(&mut self, id: &NodeId) -> &mut Node {
		&mut self.nodes[id.index()]
	}
}

impl std::fmt::Debug for Program {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.debug(self.root).fmt(f)
	}
}

/// The `DebugNode` struct prints one node of a [`Program`] with the names of the nodes it refers to, which a bare
/// [`NodeId`] can't show. Get one from [`Program::debug`].
pub struct DebugNode<'a> {
	program: &'a Program,
	id: NodeId,
}

/// Links a parsed program under the root of `program`, which supplies the built-in registry from [`Program::new`] and
/// any declarations the host added that the source may reference.
pub(crate) fn lex_with_root(mut program: Program, mut node: parser::Node) -> Result<Program, LexError> {
	crate::signatures::expand_signatures(&mut node)?;
	crate::modules::order_root_modules(&mut node)?;
	node.sort();

	let root = program.root();

	let parser::Nodes::Scope { name, .. } = &node.node else {
		return Err(LexError::invalid(
			"Invalid program root: the parsed node is not a scope. The most likely cause is that the node passed to the lexer is not the root that besl::parse returns.",
		));
	};
	assert_eq!(*name, "root");

	crate::modules::check_unique_names(&node)?;
	let parser::Nodes::Scope { children, .. } = &mut node.node else {
		unreachable!("The program root was checked to be a scope above");
	};

	// Each declaration links against the ones before it, so it can only use the declarations before it.
	let mut lexer = Lexer::new(&mut program, root);
	for index in 0..children.len() {
		if let parser::Nodes::Sampler { name, state } = children[index].node() {
			lexer.declare_sampler(name, *state);
			continue;
		}
		for declaration in super::entry::normalize_entry(lexer.program_mut(), &mut children[index], &root)? {
			lexer.program_mut().add_to_root(declaration);
		}
		let child = lexer
			.lex(&children[index])
			.map_err(|error| super::resolution::explain_declaration_order(error, &children[index..]))?;
		lexer.program_mut()[root].add_child(child);
	}
	drop(lexer);

	// Backends and the VM never see a layer parameter: they read the layers' members as plain declarations.
	super::layers::record_entry_layers(&mut program, &root)?;
	super::layers::place_descriptors(&mut program)?;
	super::layers::erase_layer_parameters(&mut program, &root);

	// Backends and the VM never see a `bool` member: they read the bytes that hold it.
	super::bools::pack_bool_members(&mut program, &root)?;

	Ok(program)
}

#[derive(Clone)]
pub struct Node {
	pub(super) node: Nodes,
}

impl Program {
	/// Creates a program whose root scope holds the built-in types and intrinsics, ready for a host to add
	/// declarations and pass to [`crate::compile_to_besl`].
	// Keep the built-in registry contiguous so overload ordering and shared type handles remain auditable together.
	#[allow(clippy::too_many_lines)]
	pub fn new() -> Program {
		// The built-ins take about a thousand nodes; reserve them, and room for a program, up front.
		let mut owned = Program {
			nodes: Vec::with_capacity(4096),
			root: NodeId(0),
			builtins: 0,
		};
		let program = &mut owned;
		let void = primitive_type(program, "void");
		let bool_t = primitive_type(program, "bool");
		let u8_t = primitive_type(program, "u8");
		let u16_t = primitive_type(program, "u16");
		let u32_t = primitive_type(program, "u32");
		let i32_t = primitive_type(program, "i32");
		let f16_t = primitive_type(program, "f16");
		let f32_t = primitive_type(program, "f32");

		let vec2u16 = vector_type(program, "vec2u16", &u16_t, 2);
		let vec4u16 = vector_type(program, "vec4u16", &u16_t, 4);
		let vec2u32 = vector_type(program, "vec2u", &u32_t, 2);
		let vec2i32 = vector_type(program, "vec2i", &i32_t, 2);
		let vec2f16 = vector_type(program, "vec2f16", &f16_t, 2);
		let vec2f32 = vector_type(program, "vec2f", &f32_t, 2);
		let vec3f16 = vector_type(program, "vec3f16", &f16_t, 3);
		let vec3f32 = vector_type(program, "vec3f", &f32_t, 3);
		let vec3u32 = vector_type(program, "vec3u", &u32_t, 3);
		let vec4u32 = vector_type(program, "vec4u", &u32_t, 4);
		let vec4f16 = vector_type(program, "vec4f16", &f16_t, 4);
		let vec4f32 = vector_type(program, "vec4f", &f32_t, 4);
		let mat4f32 = vector_type(program, "mat4f", &vec4f32, 4);
		let mat4x3f32 = vector_type(program, "mat4x3f", &vec3f32, 4);

		let texture_2d = primitive_type(program, "Texture2D");
		let texture_3d = primitive_type(program, "Texture3D");
		let texture_cube = primitive_type(program, "TextureCube");
		let texture_cube_array = primitive_type(program, "TextureCubeArray");
		let array_texture_2d = primitive_type(program, "ArrayTexture2D");
		let atomic_u32 = primitive_type(program, "atomicu32");
		let atomic_i32 = primitive_type(program, "atomici32");
		let acceleration_structure = primitive_type(program, "AccelerationStructure");
		// A hit is a plain struct, so shaders can store its members and rebuild it with its constructor, which takes the
		// members in `HIT_MEMBERS` order.
		let hit_members = crate::HIT_MEMBERS
			.iter()
			.map(|(name, r#type)| {
				let r#type = match *r#type {
					"u32" => u32_t,
					"f32" => f32_t,
					"vec2f" => vec2f32,
					"vec3f" => vec3f32,
					r#type => unreachable!("`HIT_MEMBERS` lists a `{type}` member, which the root scope doesn't declare"),
				};
				program.add(Node::member(name, r#type))
			})
			.collect();
		let hit = program.add(Node::r#struct(crate::HIT_STRUCT, hit_members));

		let mut builtins = vec![
			void,
			bool_t,
			u8_t,
			u16_t,
			u32_t,
			i32_t,
			f16_t,
			f32_t,
			vec2u16,
			vec4u16,
			vec2u32,
			vec2i32,
			vec2f16,
			vec2f32,
			vec3u32,
			vec3f16,
			vec3f32,
			vec4u32,
			vec4f16,
			vec4f32,
			mat4f32,
			mat4x3f32,
			texture_2d,
			texture_3d,
			texture_cube,
			texture_cube_array,
			array_texture_2d,
			atomic_u32,
			atomic_i32,
			acceleration_structure,
			hit,
			// Vertex invocation indices and the fragment primitive index are implicit BESL values. Their placeholder
			// locations are never exposed as stage attributes; backends and the VM map them to dedicated built-ins.
			program.add(Node::input(crate::PRIMITIVE_INDEX_BUILTIN, u32_t, u8::MAX - 2)),
			program.add(Node::input(crate::VERTEX_INDEX_BUILTIN, u32_t, u8::MAX - 1)),
			program.add(Node::input(crate::INSTANCE_INDEX_BUILTIN, u32_t, u8::MAX)),
			builtin_intrinsic(
				program,
				"sample",
				vec![("texture_sampler", texture_2d), ("uv", vec2f32)],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"texture_lod",
				vec![("texture", texture_2d), ("uv", vec2f32)],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"texture_lod",
				vec![
					("texture", texture_2d),
					("uv", vec2f32),
					("lod", f32_t),
				],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"texture_lod",
				vec![("texture", texture_3d), ("uv", vec3f32)],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"texture_lod",
				vec![
					("texture", texture_cube),
					("direction", vec3f32),
					("lod", f32_t),
				],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"texture_cube_array_lod",
				vec![
					("texture", texture_cube_array),
					("direction", vec3f32),
					("cube", u32_t),
					("lod", f32_t),
				],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"downsample_min",
				vec![
					("texture", texture_2d),
					("uv", vec2f32),
					("lod", f32_t),
				],
				f32_t,
			),
			builtin_intrinsic(
				program,
				"downsample_max",
				vec![
					("texture", texture_2d),
					("uv", vec2f32),
					("lod", f32_t),
				],
				f32_t,
			),
			builtin_intrinsic(
				program,
				"downsample_max",
				vec![
					("texture", array_texture_2d),
					("uv", vec2f32),
					("layer", u32_t),
					("lod", f32_t),
				],
				f32_t,
			),
			// `gather` returns the first channel of the four texels around a normalized UV in mip zero, in the order
			// every platform shares: x and y are the quad's second row, left then right, and z and w its first row,
			// right then left, with rows ordered by v.
			builtin_intrinsic(
				program,
				"gather",
				vec![("texture", texture_2d), ("uv", vec2f32)],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"gather",
				vec![
					("texture", array_texture_2d),
					("uv", vec2f32),
					("layer", u32_t),
				],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"fetch",
				vec![("texture", texture_2d), ("coord", vec2u32)],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"fetch",
				vec![
					("texture", array_texture_2d),
					("coord", vec2u32),
					("layer", u32_t),
				],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"fetch_u32",
				vec![("texture", texture_2d), ("coord", vec2u32)],
				u32_t,
			),
			builtin_intrinsic(
				program,
				"cross",
				vec![("left", vec3f32), ("right", vec3f32)],
				vec3f32,
			),
			builtin_intrinsic(program, "find_lsb", vec![("value", u32_t)], u32_t),
			builtin_intrinsic(program, "atan2", vec![("y", f32_t), ("x", f32_t)], f32_t),
			builtin_intrinsic(program, "sincos", vec![("value", f32_t)], vec2f32),
			builtin_intrinsic(program, "round_to_i32", vec![("value", vec2f32)], vec2i32),
			builtin_intrinsic(program, "subgroup_ballot", vec![("predicate", bool_t)], vec4u32),
			builtin_intrinsic(program, "subgroup_ballot_any", vec![("mask", vec4u32)], bool_t),
			builtin_intrinsic(program, "subgroup_ballot_find_lsb", vec![("mask", vec4u32)], u32_t),
			builtin_intrinsic(program, "subgroup_ballot_count", vec![("mask", vec4u32)], u32_t),
			builtin_intrinsic(
				program,
				"subgroup_ballot_and_not",
				vec![("mask", vec4u32), ("removed", vec4u32)],
				vec4u32,
			),
			builtin_intrinsic(
				program,
				"subgroup_broadcast_u32",
				vec![("value", u32_t), ("source_lane", u32_t)],
				u32_t,
			),
			builtin_intrinsic(
				program,
				"subgroup_broadcast_f32",
				vec![("value", f32_t), ("source_lane", u32_t)],
				f32_t,
			),
			builtin_intrinsic(
				program,
				"subgroup_shuffle_xor_f32",
				vec![("value", f32_t), ("mask", u32_t)],
				f32_t,
			),
			builtin_intrinsic(program, "workgroup_barrier", vec![], void),
			builtin_intrinsic(program, "set_task_mesh_output_count", vec![("count", u32_t)], void),
			builtin_intrinsic(program, "thread_id", vec![], vec2u32),
			builtin_intrinsic(
				program,
				"set_mesh_output_counts",
				vec![("vertex_count", u32_t), ("primitive_count", u32_t)],
				void,
			),
			builtin_intrinsic(
				program,
				"set_mesh_vertex_position",
				vec![("vertex_index", u32_t), ("position", vec4f32)],
				void,
			),
			builtin_intrinsic(
				program,
				"set_mesh_triangle",
				vec![("primitive_index", u32_t), ("triangle", vec3u32)],
				void,
			),
			builtin_intrinsic(
				program,
				"set_mesh_primitive_render_target_array_index",
				vec![("primitive_index", u32_t), ("array_index", u32_t)],
				void,
			),
			builtin_intrinsic(
				program,
				"image_load",
				vec![("image", texture_2d), ("coord", vec2u32)],
				vec4f32,
			),
			builtin_intrinsic(
				program,
				"image_load_u32",
				vec![("image", texture_2d), ("coord", vec2u32)],
				u32_t,
			),
			builtin_intrinsic(program, "texture_size", vec![("texture", texture_2d)], vec2u32),
			builtin_intrinsic(program, "texture_size", vec![("texture", array_texture_2d)], vec2u32),
			builtin_intrinsic(program, "image_size", vec![("image", texture_2d)], vec2u32),
			builtin_intrinsic(
				program,
				"guard_image_bounds",
				vec![("image", texture_2d), ("coord", vec2u32)],
				void,
			),
			builtin_intrinsic(
				program,
				"write",
				vec![
					("image", texture_2d),
					("coord", vec2u32),
					("value", vec4f32),
				],
				void,
			),
			// A tile stage writes its `vec4f` `output` declarations at a pixel. This comes after the storage-image
			// form, which a resource argument of unknown type matches first.
			builtin_intrinsic(
				program,
				"write",
				vec![
					("output", vec4f32),
					("pixel", vec2u32),
					("value", vec4f32),
				],
				void,
			),
			builtin_intrinsic(program, "output_extent", vec![], vec2u32),
			// Writes one unsigned texel of an `r32ui` storage image, the counterpart of `image_load_u32`.
			builtin_intrinsic(
				program,
				"write_u32",
				vec![
					("image", texture_2d),
					("coord", vec2u32),
					("value", u32_t),
				],
				void,
			),
			builtin_intrinsic(
				program,
				"image_atomic_or",
				vec![
					("image", texture_2d),
					("coord", vec2u32),
					("value", u32_t),
				],
				u32_t,
			),
			builtin_intrinsic(
				program,
				"image_atomic_max",
				vec![("image", texture_2d), ("coord", vec2u32), ("value", u32_t)],
				u32_t,
			),
		];

		// Families whose overloads differ only in type. Each list keeps its name's overload order, because call
		// resolution selects the first overload whose parameters match.
		builtins.extend(converted_overloads(
			program,
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
			program,
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
			program,
			"normalize",
			&["value"],
			[&vec4f32, &vec3f32, &vec2f32, &vec2f16, &vec3f16, &vec4f16],
		));
		// The float types every component-wise intrinsic accepts. Scalars come first so that a call whose arguments
		// are untyped literals, or a typed scalar and a literal, resolves to a scalar overload.
		let float_types = [&f32_t, &f16_t, &vec2f32, &vec3f32, &vec4f32, &vec2f16, &vec3f16, &vec4f16];
		// The ordering intrinsics also accept integers, for index and texel arithmetic.
		let ordered_types = float_types
			.into_iter()
			.chain([&i32_t, &u32_t, &vec2i32, &vec2u32, &vec3u32, &vec4u32]);
		builtins.extend(same_type_overloads(program, "max", &["left", "right"], ordered_types.clone()));
		builtins.extend(same_type_overloads(program, "min", &["left", "right"], ordered_types.clone()));
		builtins.extend(same_type_overloads(program, "clamp", &["value", "minimum", "maximum"], ordered_types));
		builtins.extend(same_type_overloads(program, "pow", &["value", "exponent"], float_types));
		builtins.extend(same_type_overloads(program, "step", &["edge", "value"], float_types));
		builtins.extend(same_type_overloads(program, "smoothstep", &["edge0", "edge1", "value"], float_types));
		builtins.extend(same_type_overloads(
			program,
			"fma",
			&["multiplicand", "multiplier", "addend"],
			float_types,
		));
		for name in [
			"abs",
			"sign",
			"sqrt",
			"inversesqrt",
			"exp",
			"exp2",
			"log2",
			"sin",
			"cos",
			"tan",
			"asin",
			"radians",
			"floor",
			"ceil",
			"round",
			"fract",
			"fwidth",
		] {
			builtins.extend(same_type_overloads(program, name, &["value"], float_types));
		}
		builtins.extend(converted_overloads(
			program,
			"distance",
			&["left", "right"],
			[
				(&vec2f32, &f32_t),
				(&vec3f32, &f32_t),
				(&vec4f32, &f32_t),
				(&vec2f16, &f16_t),
				(&vec3f16, &f16_t),
				(&vec4f16, &f16_t),
			],
		));
		builtins.extend(same_type_overloads(
			program,
			"reflect",
			&["incident", "normal"],
			[&vec2f32, &vec3f32, &vec4f32],
		));
		for predicate in ["is_nan", "is_infinite", "is_finite", "is_normal"] {
			builtins.extend(converted_overloads(
				program,
				predicate,
				&["value"],
				[(&f16_t, &bool_t), (&f32_t, &bool_t)],
			));
		}
		// `mix` blends two same-typed values by one scalar factor.
		for value in [&f32_t, &vec2f32, &vec3f32, &vec4f32] {
			builtins.push(builtin_intrinsic(
				program,
				"mix",
				vec![("left", *value), ("right", *value), ("factor", f32_t)],
				*value,
			));
		}
		// Conversions take one `value` of the source type and return the target type.
		let conversions: [(&str, &[&NodeId], &NodeId); 10] = [
			("f16", &[&f32_t, &f16_t, &u32_t, &i32_t], &f16_t),
			("u16", &[&u32_t], &u16_t),
			("u8", &[&u32_t], &u8_t),
			("f32", &[&f16_t, &u32_t, &i32_t], &f32_t),
			("vec2f16", &[&vec2f32, &vec2f16], &vec2f16),
			("vec3f16", &[&vec3f32, &vec3f16], &vec3f16),
			("vec4f16", &[&vec4f32, &vec4f16], &vec4f16),
			("vec2f", &[&vec2f16], &vec2f32),
			("vec3f", &[&vec3f16], &vec3f32),
			("vec4f", &[&vec4f16], &vec4f32),
		];
		for (name, sources, target) in conversions {
			builtins.extend(converted_overloads(
				program,
				name,
				&["value"],
				sources.iter().map(|source| (*source, target)),
			));
		}
		builtins.extend(converted_overloads(
			program,
			"u32",
			&["value"],
			[&u32_t, &u8_t, &u16_t, &i32_t, &f16_t, &f32_t].map(|source| (source, &u32_t)),
		));
		// Invocation builtins take no arguments and read the invocation's coordinates.
		for name in [
			"thread_idx",
			"subgroup_lane_index",
			"threadgroup_position",
			"thread_position",
			"thread_layer",
		] {
			builtins.push(builtin_intrinsic(program, name, vec![], u32_t));
		}
		// Ray queries share one signature and differ in what they report. `invoke` isn't registered here: its payload
		// type comes from the program, so the lexer instantiates it per program.
		for (name, r#return) in [("intersect", hit), ("trace_occlusion", bool_t)] {
			builtins.push(builtin_intrinsic(
				program,
				name,
				vec![
					("scene", acceleration_structure),
					("mask", u32_t),
					("origin", vec3f32),
					("direction", vec3f32),
					("t_min", f32_t),
					("t_max", f32_t),
				],
				r#return,
			));
		}
		// Ray tracing invocation builtins read the launch point of a ray generation shader or the hit a closest-hit
		// shader shades.
		let ray_tracing_builtins = [
			("launch_index", &vec3u32),
			("launch_size", &vec3u32),
			("hit_distance", &f32_t),
			("hit_custom_index", &u32_t),
			("hit_geometry_index", &u32_t),
			("hit_primitive_index", &u32_t),
			("hit_barycentrics", &vec2f32),
			("ray_origin", &vec3f32),
			("ray_direction", &vec3f32),
		];
		for (name, r#return) in ray_tracing_builtins {
			builtins.push(builtin_intrinsic(program, name, vec![], *r#return));
		}
		builtins.extend(atomic_intrinsics(program, atomic_u32, u32_t, void));
		builtins.extend(atomic_intrinsics(program, atomic_i32, i32_t, void));

		let builtin_count = builtins.len();
		let root = program.add(Node::new(Nodes::Scope {
			name: "root".to_string(),
			children: builtins,
		}));
		owned.root = root;
		owned.builtins = builtin_count;
		owned
	}
}

impl Node {

	/// Creates a `use` declaration that brings `target` in by `name`, or, with no `name`, every declaration of the
	/// module `target`.
	pub fn r#use(name: Option<String>, target: &NodeId) -> Node {
		Node {
			node: Nodes::Use { name, target: *target },
		}
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
	pub fn r#struct(name: &str, fields: Vec<NodeId>) -> Node {
		Node {
			node: Nodes::Struct {
				name: name.to_string(),
				template: None,
				fields,
				types: Vec::new(),
			},
		}
	}

	pub fn member(name: &str, r#type: NodeId) -> Node {
		Node {
			node: Nodes::Member {
				name: name.to_string(),
				r#type,
				count: None,
			},
		}
	}

	pub fn array(name: &str, r#type: NodeId, size: usize) -> Node {
		Node {
			node: Nodes::Member {
				name: name.to_string(),
				r#type,
				count: Some(NonZeroUsize::new(size).expect("Invalid size")),
			},
		}
	}

	pub fn function(
		name: &str,
		params: Vec<NodeId>,
		return_type: NodeId,
		statements: Vec<NodeId>,
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
	pub fn conditional(condition: NodeId, statements: Vec<NodeId>, else_branch: Option<ElseBranch>) -> Node {
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
	pub fn r#match(scrutinee: NodeId, r#type: NodeId, arms: Vec<MatchArm>, default: Vec<NodeId>) -> Node {
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
		initializer: NodeId,
		condition: NodeId,
		update: NodeId,
		statements: Vec<NodeId>,
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

	pub fn r#macro(name: &str, body: NodeId) -> Node {
		Node {
			node: Nodes::Expression(Expressions::Macro {
				name: name.to_string(),
				body,
			}),
		}
	}

	/// Builds a device-backed binding. Use [`Self::binding_in_memory`] for dispatch-shared constant data.
	pub fn binding(name: &str, r#type: BindingTypes, slot: u32, read: bool, write: bool) -> Node {
		Self::binding_in_memory(name, r#type, slot, read, write, BufferMemoryClass::Device, None)
	}

	/// Builds a binding whose memory class is independent from its read and write access. Pass a `count` to bind an
	/// array of that many resources, as [`Self::binding_array`] does.
	pub fn binding_in_memory(
		name: &str,
		r#type: BindingTypes,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: BufferMemoryClass,
		count: Option<NonZeroU32>,
	) -> Node {
		// `Program::add` lowers a single buffer whose only member is a fixed array.
		Node {
			node: Nodes::Binding {
				name: name.to_string(),
				r#type,
				slot,
				read,
				write,
				memory_class,
				count,
				layout: None,
				placement: None,
			},
		}
	}

	/// Moves a binding into the descriptor set layout named `layout`. Other nodes are returned unchanged.
	///
	/// A binding built without a layout belongs to its program's own layout. Bindings that share a layout name form
	/// one descriptor set, which every program that declares that layout binds as a whole.
	pub fn in_layout(mut self, layout: &str) -> Node {
		if let Nodes::Binding { layout: target, .. } = &mut self.node {
			*target = Some(layout.to_string());
		}
		self
	}

	pub fn binding_array(name: &str, r#type: BindingTypes, slot: u32, read: bool, write: bool, count: usize) -> Node {
		let count = u32::try_from(count)
			.expect("Invalid binding array count. The most likely cause is that a resource array exceeds u32::MAX elements.");
		let count = NonZeroU32::new(count).expect(
			"Invalid binding array count. The most likely cause is that a resource array was declared with zero elements.",
		);
		Self::binding_in_memory(name, r#type, slot, read, write, BufferMemoryClass::Device, Some(count))
	}

	pub fn push_constant(members: Vec<NodeId>) -> Node {
		Node {
			node: Nodes::PushConstant { members },
		}
	}

	pub fn intrinsic(name: &str, elements: Vec<NodeId>, r#return: NodeId) -> Node {
		Node {
			node: Nodes::Intrinsic {
				name: name.to_string(),
				elements,
				r#return,
			},
		}
	}

	/// Declares a host-selected constant whose backend index starts at `id`. See [`crate::parser::Node::specialization`].
	pub fn specialization(name: &str, r#type: NodeId, id: u32) -> Node {
		Node {
			node: Nodes::Specialization {
				name: name.to_string(),
				r#type,
				id,
			},
		}
	}

	pub fn constant(name: &str, r#type: NodeId, value: NodeId) -> Node {
		Node {
			node: Nodes::Const {
				name: name.to_string(),
				r#type,
				value,
			},
		}
	}

	pub fn input(name: &str, format: NodeId, location: u8) -> Node {
		Node {
			node: Nodes::Input {
				name: name.to_string(),
				format,
				location,
				per_primitive: false,
			},
		}
	}

	pub fn output(name: &str, format: NodeId, location: u8) -> Node {
		Self::output_array(name, format, location, None, false)
	}

	/// Declares a mesh output array of `count` elements, with one element per vertex, which rasterization
	/// interpolates, when `per_vertex` is set, or one flat element per primitive otherwise. Without a `count` it
	/// declares a plain stage output, like [`Self::output`].
	pub fn output_array(
		name: &str,
		format: NodeId,
		location: u8,
		count: Option<NonZeroUsize>,
		per_vertex: bool,
	) -> Node {
		Node {
			node: Nodes::Output {
				name: name.to_string(),
				format,
				location,
				count,
				per_vertex,
			},
		}
	}

	pub fn task_payload(name: &str, format: NodeId, count: NonZeroUsize) -> Node {
		Node {
			node: Nodes::TaskPayload {
				name: name.to_string(),
				format,
				count,
			},
		}
	}

	pub fn workgroup(name: &str, format: NodeId, count: Option<NonZeroUsize>) -> Node {
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

	/// Appends `child` to a container node, such as a scope's declarations or a function's statements, and returns it.
	/// Other nodes ignore it.
	pub fn add_child(&mut self, child: NodeId) -> NodeId {
		if let Nodes::Scope { children, .. }
		| Nodes::Struct { fields: children, .. }
		| Nodes::Function {
			statements: children, ..
		}
		| Nodes::PushConstant { members: children }
		| Nodes::Intrinsic { elements: children, .. } = &mut self.node
		{
			children.push(child);
		}
		child
	}

	pub fn add_children(&mut self, children: Vec<NodeId>) {
		for child in children {
			self.add_child(child);
		}
	}

	pub fn node(&self) -> &Nodes {
		&self.node
	}

	pub fn get_name(&self) -> Option<&str> {
		match &self.node {
			Nodes::Scope { name, .. }
			| Nodes::Layer { name, .. }
			| Nodes::Signature { name }
			| Nodes::Function { name, .. }
			| Nodes::Member { name, .. }
			| Nodes::Struct { name, .. }
			| Nodes::Intrinsic { name, .. }
			| Nodes::Binding { name, .. }
			| Nodes::Parameter { name, .. }
			| Nodes::Specialization { name, .. }
			| Nodes::Const { name, .. }
			| Nodes::Input { name, .. }
			| Nodes::Output { name, .. }
			| Nodes::TaskPayload { name, .. }
			| Nodes::Workgroup { name, .. } => Some(name),
			Nodes::PushConstant { .. } => Some("push_constant"),
			Nodes::Expression(Expressions::VariableDeclaration { name, .. } | Expressions::Member { name, .. }) => Some(name),
			_ => None,
		}
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
	/// Reports whether a binding of this type with `read` and `write` access is a sampled image: an image the shader
	/// reads and never writes.
	///
	/// Backends declare a sampled image as a read-only texture without a sampler, and reflection reports it as one, so
	/// the image it binds needs no storage usage, which would keep Metal from compressing it.
	pub fn is_sampled_image(&self, read: bool, write: bool) -> bool {
		matches!(self, Self::Image { .. }) && read && !write
	}
}

/// The most descriptor sets one shader can read. Every backend can bind this many: Metal binds them after its 16
/// reserved buffer indices, and Vulkan and DirectX 12 keep their heap bases in a fixed block of push data.
pub const MAX_DESCRIPTOR_SETS: u32 = 15;

/// The `Placement` struct locates one binding inside the descriptor sets of the program that declares it, so every
/// backend and the VM address the binding the same way without regrouping the program's sets.
///
/// A set's descriptors are stored densely in binding order: an array takes `count` consecutive descriptors and one
/// binding number, so it moves the storage of the bindings after it but never their numbers. Linking computes it with
/// [`crate::place_descriptors`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Placement {
	/// The position of the binding's set among the program's sets: the layers `main` takes, in parameter order, then the
	/// sets backends add, then the program's own descriptors.
	pub set: u32,
	/// How many descriptors the bindings before this one in its set hold: the index of its first descriptor.
	pub resource: u32,
	/// How many samplers the bindings before this one in its set hold. Only a texture whose sampler the application
	/// binds holds samplers, one per descriptor.
	pub sampler: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingTypes {
	Buffer {
		members: Vec<NodeId>,
	},
	/// A storage buffer holding a flat array of `element` values.
	///
	/// Runtime arrays (`type: T[]`) have no `fixed` size; the bound resource supplies their length. A buffer declared
	/// as a struct whose only member is a fixed array, such as `Meshes: struct { meshes: Mesh[1024] }`, is lowered to
	/// this form, so every backend handles one array shape.
	BufferArray {
		element: NodeId,
		fixed: Option<FixedArray>,
	},
	/// A texture shaders sample. `format` names the texture shape, or is empty for a 2D texture.
	CombinedImageSampler {
		format: String,
		/// The sampler state a `sampler` declaration fixes, or `None` when the application binds the sampler. See
		/// [`crate::sampler`].
		sampler: Option<crate::SamplerState>,
	},
	/// A storage image shaders load from or store to. `format` names its texel format, such as `rgba16f`, or is `None`
	/// for a formatless image whose format the bound image gives.
	Image {
		format: Option<String>,
	},
	/// A top-level acceleration structure that the ray queries `intersect` and `trace_occlusion` test rays against. It
	/// is read-only and binds one structure.
	AccelerationStructure,
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

impl BufferMemoryClass {
	/// Returns the value's source spelling, as the `memory` descriptor property writes it.
	pub fn token(self) -> &'static str {
		match self {
			Self::Constant => "constant",
			Self::Device => "device",
		}
	}
}

/// The `ElseBranch` enum keeps `else if` chains distinct from plain `else` blocks,
/// so backends can lower each form by structure. See [`Nodes::Conditional`].
#[derive(Clone, Debug)]
pub enum ElseBranch {
	/// An `else { ... }` block.
	Block(Vec<NodeId>),
	/// An `else if` link. The node is always a [`Nodes::Conditional`].
	If(NodeId),
}

impl ElseBranch {
	/// Returns the branch as a statement list. An `else if` link is one conditional statement.
	/// Use it in walkers that treat both forms alike.
	pub fn statements(&self) -> &[NodeId] {
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
	pub statements: Vec<NodeId>,
}

#[derive(Clone)]
pub enum Nodes {
	/// A module, or the program root. Code inside it sees its declarations by name, and code outside reaches them
	/// through a path such as `module::name` or a [`Nodes::Use`].
	Scope {
		name: String,
		children: Vec<NodeId>,
	},
	/// A `use` declaration. Name lookups in the scope that holds it see `target` by `name`. With no `name`, from
	/// `use module::*;`, they see every declaration of the module `target` instead.
	///
	/// A module can bring in one of its own ancestors. Backends emit nothing for it.
	Use { name: Option<String>, target: NodeId },
	/// A layer: the descriptors, push constant block, and tile targets a shader consumes through one parameter of the
	/// layer's type.
	///
	/// Its descriptors are bindings in the layout named `path`, the layer's full path, so they form one descriptor set.
	/// Code reaches a member through a layer value, such as `scene.views`, never by its bare name.
	Layer {
		name: String,
		/// The layer's path from the program root, such as `::byte_engine::layouts::scene::Scene`, which names its set.
		path: String,
		members: Vec<NodeId>,
		/// The name code reaches the push constant member by, which a push constant block doesn't carry itself.
		push_constant: Option<String>,
		/// Where `main` takes the layer among its layer parameters, or `None` when `main` doesn't take it. Reflection
		/// orders a shader's sets by it and leaves out the sets of layers `main` doesn't take.
		entry: Option<usize>,
	},
	/// A signature, a function type. Linking gives its parameters to the functions that implement it, so only its name
	/// is kept, for `use` declarations that name it.
	Signature {
		name: String,
	},
	Struct {
		name: String,
		template: Option<NodeId>,
		fields: Vec<NodeId>,
		types: Vec<NodeId>,
	},
	Member {
		name: String,
		r#type: NodeId,
		count: Option<NonZeroUsize>,
	},
	Function {
		name: String,
		params: Vec<NodeId>,
		return_type: NodeId,
		statements: Vec<NodeId>,
	},
	/// An `if` statement, with an optional `else` or `else if` branch.
	Conditional {
		condition: NodeId,
		statements: Vec<NodeId>,
		else_branch: Option<ElseBranch>,
	},
	/// A `match` statement over a `bool` or integer value.
	///
	/// The lexer resolves Rust's first-match-wins rule, so the arms' values are distinct and `default` holds the
	/// statements for every other value. Lower it to a `switch` whose `default` case runs `default`.
	Match {
		scrutinee: NodeId,
		/// The scrutinee's type. It is `bool`, `u8`, `u16`, `u32`, or `i32`.
		r#type: NodeId,
		arms: Vec<MatchArm>,
		default: Vec<NodeId>,
	},
	ForLoop {
		initializer: NodeId,
		condition: NodeId,
		update: NodeId,
		statements: Vec<NodeId>,
	},
	Specialization {
		name: String,
		r#type: NodeId,
		/// The first backend constant index. Vector components take consecutive indices.
		id: u32,
	},
	Expression(Expressions),
	Binding {
		name: String,
		/// The binding's number within its descriptor set, as `binding:` writes it. Each set numbers its bindings from 0.
		slot: u32,
		read: bool,
		write: bool,
		memory_class: BufferMemoryClass,
		r#type: BindingTypes,
		count: Option<NonZeroU32>,
		/// The name of the descriptor set layout that declares the binding: the path of the layer it is a member of, or `None`
		/// for the program's own layout. Bindings that share a layout form one descriptor set.
		layout: Option<String>,
		/// Where the binding sits among the program's descriptor sets, or `None` for a binding of a layer `main` doesn't
		/// take. Linking sets it; see [`crate::place_descriptors`].
		placement: Option<Placement>,
	},
	PushConstant {
		members: Vec<NodeId>,
	},
	Intrinsic {
		name: String,
		elements: Vec<NodeId>,
		r#return: NodeId,
	},
	Input {
		name: String,
		format: NodeId,
		location: u8,
		/// Whether a fragment input reads the value its mesh stage writes once per primitive, from an `output` array,
		/// instead of a value interpolated across the triangle. Vulkan links the two only when both declare it.
		per_primitive: bool,
	},
	Output {
		name: String,
		format: NodeId,
		location: u8,
		count: Option<NonZeroUsize>,
		/// Whether a mesh output array holds one element per vertex, interpolated across each triangle, instead of
		/// one flat element per primitive.
		per_vertex: bool,
	},
	TaskPayload {
		name: String,
		format: NodeId,
		count: NonZeroUsize,
	},
	Workgroup {
		name: String,
		format: NodeId,
		count: Option<NonZeroUsize>,
	},
	Parameter {
		name: String,
		r#type: NodeId,
	},
	/// A named module-level value known at compile time.
	Const {
		name: String,
		r#type: NodeId,
		value: NodeId,
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
	pub fn children(&self) -> impl Iterator<Item = &NodeId> {
		let (singles, first, second): ([Option<&NodeId>; 3], &[NodeId], &[NodeId]) = match self {
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
				Expressions::Unary { operand, .. } => ([Some(operand), None, None], &[], &[]),
				Expressions::Ternary {
					condition,
					if_true,
					if_false,
				} => ([Some(condition), Some(if_true), Some(if_false)], &[], &[]),
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
	pub fn branch_children(&self) -> impl Iterator<Item = &NodeId> {
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
		matches!(
			self,
			Nodes::Parameter { .. }
				| Nodes::Scope { .. }
				| Nodes::Intrinsic { .. }
				| Nodes::Member { .. }
				| Nodes::Expression(_)
		)
	}

	/// Reports whether a value this node names can be indexed with `[]`. Types are read from `program`.
	pub fn is_indexable(&self, program: &Program) -> bool {
		let type_is_indexable = |r#type: &NodeId| {
			let r#type = &program[r#type];
			matches!(r#type.node(), Nodes::Struct { template: Some(_), .. })
				|| r#type
					.get_name()
					.is_some_and(|name| name.starts_with("vec") || name.starts_with("mat"))
		};

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
			Nodes::Expression(Expressions::Member { source, .. }) => match program[source].node() {
				Nodes::Binding {
					r#type: BindingTypes::CombinedImageSampler { format, .. },
					count: None,
					..
				} => format == "ArrayTexture2D",
				source => source.is_indexable(program),
			},
			Nodes::Expression(Expressions::Accessor { right, .. }) => program[right].node().is_indexable(program),
			// A parenthesized value indexes like the value inside it, as in `(values)[i]`.
			Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => {
				program[elements[0]].node().is_indexable(program)
			}
			_ => false,
		}
	}

	/// Reports whether this node is, or reads a member of, a storage buffer binding.
	pub fn is_buffer_binding(&self, program: &Program) -> bool {
		match self {
			Nodes::Binding {
				r#type: BindingTypes::Buffer { .. } | BindingTypes::BufferArray { .. },
				..
			} => true,
			Nodes::Expression(Expressions::Member { source, .. }) => program[source].node().is_buffer_binding(program),
			_ => false,
		}
	}
}

/// Collects the names of `nodes` so Debug output lists children by name instead of printing whole subtrees.
fn names(program: &Program, nodes: &[NodeId]) -> Vec<Option<String>> {
	nodes.iter().map(|node| program.name(*node).map(str::to_string)).collect()
}

/// Prints a list of nodes through [`DebugNode`], so nested statements show their contents.
struct DebugList<'a>(&'a Program, &'a [NodeId]);

impl std::fmt::Debug for DebugList<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_list().entries(self.1.iter().map(|id| self.0.debug(*id))).finish()
	}
}

impl std::fmt::Debug for DebugNode<'_> {
	// Every node variant is formatted here so Debug output stays exhaustive when the AST grows.
	#[allow(clippy::too_many_lines)]
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let program = self.program;
		match program[self.id].node() {
			Nodes::Scope { name, children } => write!(f, "Scope {{ name: {name}, children: {:#?} }}", names(program, children)),
			Nodes::Layer { name, path, members, .. } => {
				write!(f, "Layer {{ name: {name}, path: {path}, members: {:?} }}", names(program, members))
			}
			Nodes::Signature { name } => write!(f, "Signature {{ name: {name} }}"),
			Nodes::Use { name, target } => write!(
				f,
				"Use {{ name: {name:?}, target: {:?} }}",
				program.name(*target)
			),
			Nodes::Struct { name, fields, .. } => write!(f, "Struct {{ name: {name}, fields: {:?} }}", names(program, fields)),
			Nodes::Member { name, r#type, .. } => {
				write!(f, "Member {{ name: {name}, type: {:?} }}", program.name(*r#type))
			}
			Nodes::Function {
				name,
				params,
				statements,
				..
			} => write!(
				f,
				"Function {{ name: {name}, parameters: {:?}, statements: {:?} }}",
				names(program, params),
				names(program, statements)
			),
			Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => write!(
				f,
				"Conditional {{ condition: {:?}, statements: {:?}, else_branch: {:?} }}",
				program.debug(*condition),
				DebugList(program, statements),
				else_branch.as_ref().map(|branch| DebugList(program, branch.statements()))
			),
			Nodes::Match {
				scrutinee,
				r#type,
				arms,
				default,
			} => write!(
				f,
				"Match {{ scrutinee: {:?}, type: {:?}, arms: {arms:?}, default: {:?} }}",
				program.debug(*scrutinee),
				program.name(*r#type),
				DebugList(program, default)
			),
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => write!(
				f,
				"ForLoop {{ initializer: {:?}, condition: {:?}, update: {:?}, statements: {:?} }}",
				program.debug(*initializer),
				program.debug(*condition),
				program.debug(*update),
				DebugList(program, statements)
			),
			Nodes::Specialization { name, r#type, id } => {
				write!(
					f,
					"Specialization {{ name: {name}, type: {:?}, id: {id} }}",
					program.name(*r#type)
				)
			}
			Nodes::Expression(expression) => write!(f, "Expression {{ {expression:?} }}"),
			Nodes::Binding {
				name,
				slot,
				read,
				write,
				memory_class,
				r#type,
				count,
				layout,
				placement,
			} => write!(
				f,
				"Binding {{ name: {name}, slot: {slot}, read: {read}, write: {write}, memory_class: {memory_class:?}, type: {:?}, count: {count:?}, layout: {layout:?}, placement: {placement:?} }}",
				r#type
			),
			Nodes::PushConstant { members } => write!(f, "PushConstant {{ members: {:?} }}", names(program, members)),
			Nodes::Intrinsic {
				name,
				elements,
				r#return,
			} => write!(
				f,
				"Intrinsic {{ name: {name}, elements: {:?}, return: {:?} }}",
				names(program, elements),
				program.name(*r#return)
			),
			Nodes::Parameter { name, r#type } => {
				write!(f, "Parameter {{ name: {name}, type: {:?} }}", program.name(*r#type))
			}
			Nodes::Input {
				name,
				format,
				location,
				per_primitive,
			} => write!(
				f,
				"Input {{ name: {name}, format: {:?}, location: {location}, per_primitive: {per_primitive} }}",
				program.name(*format)
			),
			Nodes::Output {
				name,
				format,
				location,
				count,
				per_vertex,
			} => write!(
				f,
				"Output {{ name: {name}, format: {:?}, location: {location}, count: {count:?}, per_vertex: {per_vertex} }}",
				program.name(*format)
			),
			Nodes::TaskPayload { name, format, count } => write!(
				f,
				"TaskPayload {{ name: {name}, format: {:?}, count: {count} }}",
				program.name(*format)
			),
			Nodes::Workgroup { name, format, count } => write!(
				f,
				"Workgroup {{ name: {name}, format: {:?}, count: {count:?} }}",
				program.name(*format)
			),
			Nodes::Const { name, r#type, value } => write!(
				f,
				"Const {{ name: {name}, type: {:?}, value: {:?} }}",
				program.name(*r#type),
				program.debug(*value)
			),
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
	BitwiseXor,
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

/// Pairs each operator with its source token and binding precedence. A lower precedence binds tighter. Member
/// access binds tightest, at 1, then the prefix [`UnaryOperators`], at [`UNARY_PRECEDENCE`].
const OPERATOR_TOKENS: [(&str, Operators, u8); 19] = [
	("=", Operators::Assignment, 11),
	("||", Operators::LogicalOr, 9),
	("|", Operators::BitwiseOr, 9),
	// XOR binds tighter than OR and looser than AND, matching C-family shading languages.
	("^", Operators::BitwiseXor, 8),
	("&&", Operators::LogicalAnd, 7),
	("&", Operators::BitwiseAnd, 7),
	("==", Operators::Equality, 6),
	("!=", Operators::Inequality, 6),
	("<", Operators::LessThan, 6),
	(">", Operators::GreaterThan, 6),
	("<=", Operators::LessThanOrEqual, 6),
	(">=", Operators::GreaterThanOrEqual, 6),
	("<<", Operators::ShiftLeft, 5),
	(">>", Operators::ShiftRight, 5),
	("+", Operators::Plus, 4),
	("-", Operators::Minus, 4),
	("*", Operators::Multiply, 3),
	("/", Operators::Divide, 3),
	("%", Operators::Modulo, 3),
];

/// How loosely the prefix [`UnaryOperators`] bind: tighter than every binary operator, so `-a * b` is `(-a) * b`, and
/// looser than member access, so `-a.b` is `-(a.b)`. See [`Operators::precedence`] for the scale.
pub(crate) const UNARY_PRECEDENCE: u8 = 2;

impl Operators {
	/// Reads the operator a source token spells, or returns `None` for any other token.
	pub fn from_token(token: &str) -> Option<Self> {
		OPERATOR_TOKENS
			.iter()
			.find(|(operator_token, ..)| *operator_token == token)
			.map(|(_, operator, _)| *operator)
	}

	/// Returns the token that spells this operator in BESL source.
	pub fn token(&self) -> &'static str {
		OPERATOR_TOKENS
			.iter()
			.find(|(_, operator, _)| operator == self)
			.map_or("", |(token, ..)| *token)
	}

	/// Returns how loosely this operator binds. The parser splits an expression at its loosest operator first.
	pub fn precedence(&self) -> u8 {
		OPERATOR_TOKENS
			.iter()
			.find(|(_, operator, _)| operator == self)
			.map_or(0, |(.., precedence)| *precedence)
	}
}

/// The `UnaryOperators` enum names the BESL prefix operators. The parser reads them from source tokens with
/// [`UnaryOperators::from_token`], and backends write them back with [`UnaryOperators::token`], which every target
/// shading language spells the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOperators {
	/// `-value` negates a float or integer scalar or vector. An unsigned value wraps, as in C-family shading languages.
	Negate,
	/// `!value` negates a `bool`.
	LogicalNot,
	/// `~value` flips every bit of an integer scalar or vector.
	BitwiseNot,
}

impl UnaryOperators {
	/// Reads the prefix operator a source token spells, or returns `None` for any other token.
	pub fn from_token(token: &str) -> Option<Self> {
		match token {
			"-" => Some(Self::Negate),
			"!" => Some(Self::LogicalNot),
			"~" => Some(Self::BitwiseNot),
			_ => None,
		}
	}

	/// Returns the token that spells this operator in BESL and in every target shading language.
	pub fn token(self) -> &'static str {
		match self {
			Self::Negate => "-",
			Self::LogicalNot => "!",
			Self::BitwiseNot => "~",
		}
	}
}

#[derive(Clone, Debug)]
pub enum Expressions {
	Return {
		value: Option<NodeId>,
	},
	Continue,
	/// Leaves the innermost enclosing loop.
	Break,
	Discard,
	Member {
		name: String,
		source: NodeId,
	},
	Expression {
		elements: Vec<NodeId>,
	},
	Literal {
		value: String,
	},
	FunctionCall {
		function: NodeId,
		parameters: Vec<NodeId>,
	},
	IntrinsicCall {
		intrinsic: NodeId,
		arguments: Vec<NodeId>,
		elements: Vec<NodeId>,
	},
	Operator {
		operator: Operators,
		left: NodeId,
		right: NodeId,
	},
	/// A prefix operator applied to `operand`, such as `-x`, `!flag`, or `~mask`.
	Unary {
		operator: UnaryOperators,
		operand: NodeId,
	},
	/// The value of an `if`/`else` whose branches are plain values: `if_true` when the `bool` `condition` holds, and
	/// `if_false` otherwise. Only the selected operand is evaluated, so backends write it as their `?:` operator. The
	/// lexer builds it from `if (condition) { if_true } else { if_false }`; an `if` whose branches run statements is
	/// lowered to statements instead.
	Ternary {
		condition: NodeId,
		if_true: NodeId,
		if_false: NodeId,
	},
	VariableDeclaration {
		name: String,
		r#type: NodeId,
	},
	Accessor {
		left: NodeId,
		right: NodeId,
	},
	Macro {
		name: String,
		body: NodeId,
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
	/// A bare name or a call that no declaration before it declares.
	UndeclaredName {
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

pub(super) fn builtin_intrinsic(
	program: &mut Program,
	name: &str,
	parameters: Vec<(&str, NodeId)>,
	r#return: NodeId,
) -> NodeId {
	let parameters = parameters
		.into_iter()
		.map(|(name, r#type)| {
			program.add(Node::new(Nodes::Parameter {
				name: name.to_string(),
				r#type,
			}))
		})
		.collect();
	program.add(Node::intrinsic(name, parameters, r#return))
}

/// Builds the relaxed scalar atomic surface for one signed or unsigned 32-bit type.
fn atomic_intrinsics(program: &mut Program, atomic: NodeId, scalar: NodeId, void: NodeId) -> Vec<NodeId> {
	fn binary(program: &mut Program, name: &str, operand_name: &str, atomic: NodeId, scalar: NodeId) -> NodeId {
		builtin_intrinsic(program, name, vec![("value", atomic), (operand_name, scalar)], scalar)
	}

	vec![
		builtin_intrinsic(program, "atomic_load", vec![("value", atomic)], scalar),
		builtin_intrinsic(program, "atomic_store", vec![("value", atomic), ("stored", scalar)], void),
		binary(program, "atomic_exchange", "stored", atomic, scalar),
		builtin_intrinsic(
			program,
			"atomic_compare_exchange",
			vec![("value", atomic), ("expected", scalar), ("desired", scalar)],
			scalar,
		),
		binary(program, "atomic_add", "increment", atomic, scalar),
		binary(program, "atomic_sub", "decrement", atomic, scalar),
		binary(program, "atomic_min", "candidate", atomic, scalar),
		binary(program, "atomic_max", "candidate", atomic, scalar),
		binary(program, "atomic_and", "mask", atomic, scalar),
		binary(program, "atomic_or", "mask", atomic, scalar),
		binary(program, "atomic_xor", "mask", atomic, scalar),
	]
}

/// Declares one `name` overload per `(operand, result)` signature, in order. Every parameter in `parameters` takes the
/// operand type. Keep each name's overloads in their intended order, because call resolution picks the first match.
fn converted_overloads<'a>(
	program: &mut Program,
	name: &str,
	parameters: &[&str],
	signatures: impl IntoIterator<Item = (&'a NodeId, &'a NodeId)>,
) -> Vec<NodeId> {
	signatures
		.into_iter()
		.map(|(operand, result)| {
			builtin_intrinsic(
				program,
				name,
				parameters.iter().map(|parameter| (*parameter, *operand)).collect(),
				*result,
			)
		})
		.collect()
}

/// Declares one `name` overload per type in `types`, where every parameter and the result share that type.
fn same_type_overloads<'a>(
	program: &mut Program,
	name: &str,
	parameters: &[&str],
	types: impl IntoIterator<Item = &'a NodeId>,
) -> Vec<NodeId> {
	converted_overloads(program, name, parameters, types.into_iter().map(|r#type| (r#type, r#type)))
}

/// Built-in scalar types with a byte representation in storage buffers. `bool`, `void`, and resource handles have none.
pub(crate) const STORABLE_SCALAR_TYPES: [&str; 6] = ["u8", "u16", "u32", "i32", "f16", "f32"];

fn primitive_type(program: &mut Program, name: &str) -> NodeId {
	program.add(Node::r#struct(name, Vec::new()))
}

/// Builds a vector or matrix type whose first `count` fields of `x`, `y`, `z`, and `w` share the `component` type.
fn vector_type(program: &mut Program, name: &str, component: &NodeId, count: usize) -> NodeId {
	let fields = ["x", "y", "z", "w"][..count]
		.iter()
		.map(|axis| program.add(Node::member(axis, *component)))
		.collect();
	program.add(Node::r#struct(name, fields))
}
