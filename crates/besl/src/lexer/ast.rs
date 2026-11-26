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
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NodeId(u32);

impl NodeId {
	/// Returns the node's position in its program, a stable key for tables that index nodes densely.
	pub(crate) fn index(self) -> usize {
		self.0 as usize
	}
}

/// The `Name` struct is an identifier interned in one [`Program`], such as a function, type, member, or local name.
///
/// Lookups and backends compare names on every reference, so a name is a small integer that compares without reading
/// text. Read its text with [`Program::text`], and create one with [`Program::intern`]. Two names of one program are
/// equal exactly when their text is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Name(u32);

impl Name {
	/// The name [`Node::get_name`] gives a push constant block, interned first in every program.
	pub(crate) const PUSH_CONSTANT: Name = Name(0);
}

/// The `Names` struct interns the identifiers of one program, so each distinct text is stored once and a [`Name`] can
/// stand in for it.
///
/// The texts live in one string arena, so interning a name allocates nothing of its own. A program's table is layered
/// over the built-ins' table: it holds only the program's own names, issues ids after the built-ins' ids, and asks the
/// built-ins' table first, so every program shares the built-ins' names without copying them.
#[derive(Clone)]
struct Names<'b> {
	/// Every text this table holds, one after another.
	arena: String,
	/// The byte range of each name's text in `arena`, indexed by the name's id minus `first`.
	spans: Vec<(u32, u32)>,
	/// The names this table holds, hashed and compared through `arena`.
	ids: hashbrown::HashTable<Name>,
	/// The table below this one, which answers for every name id under `first`.
	base: Option<&'b Names<'b>>,
	/// The id of this table's first name: the number of names its base holds.
	first: u32,
}

/// Hashes a name's text, the same way for lookups and for the texts already in a table.
fn hash_text(text: &str) -> u64 {
	use std::hash::BuildHasher;
	rustc_hash::FxBuildHasher.hash_one(text)
}

impl<'b> Names<'b> {
	/// Creates the bottom table, which holds the names other code relies on finding at fixed ids.
	fn new() -> Self {
		let mut names = Self::layered_over(None);
		let push_constant = names.intern("push_constant");
		debug_assert_eq!(push_constant, Name::PUSH_CONSTANT);
		names
	}

	/// Creates an empty table whose names follow the names of `base`.
	fn layered_over(base: Option<&'b Names<'b>>) -> Self {
		Self {
			arena: String::new(),
			spans: Vec::new(),
			ids: hashbrown::HashTable::new(),
			first: base.map_or(0, Names::len),
			base,
		}
	}

	/// Returns how many names this table and the tables below it hold.
	fn len(&self) -> u32 {
		self.first + u32::try_from(self.spans.len()).expect("A program holds fewer than 2^32 distinct names.")
	}

	/// Returns the text of `name`, which this table or one below it issued.
	fn text(&self, name: Name) -> &str {
		match name.0.checked_sub(self.first) {
			Some(own) => {
				let (start, end) = self.spans[own as usize];
				&self.arena[start as usize..end as usize]
			}
			None => self
				.base
				.expect("A name under a table's first id comes from its base.")
				.text(name),
		}
	}

	/// Returns the name whose text is `text`, or `None` when neither this table nor one below it holds it.
	fn find(&self, text: &str) -> Option<Name> {
		self.find_hashed(text, hash_text(text))
	}

	/// Looks `text` up by its precomputed `hash`, so a lookup through every layer hashes the text once.
	fn find_hashed(&self, text: &str, hash: u64) -> Option<Name> {
		if let Some(name) = self.base.and_then(|base| base.find_hashed(text, hash)) {
			return Some(name);
		}
		self.ids.find(hash, |name| self.text(*name) == text).copied()
	}

	/// Returns the name of `text`, storing the text the first time the program meets it.
	fn intern(&mut self, text: &str) -> Name {
		let hash = hash_text(text);
		if let Some(name) = self.find_hashed(text, hash) {
			return name;
		}
		let name = Name(self.len());
		let start = u32::try_from(self.arena.len()).expect("A program's names hold fewer than 2^32 bytes of text.");
		self.arena.push_str(text);
		let end = u32::try_from(self.arena.len()).expect("A program's names hold fewer than 2^32 bytes of text.");
		self.spans.push((start, end));
		// The table rehashes stored names when it grows, reading their text from the arena.
		let (arena, spans, first) = (&self.arena, &self.spans, self.first);
		self.ids.insert_unique(hash, name, |stored| {
			let (start, end) = spans[(stored.0 - first) as usize];
			hash_text(&arena[start as usize..end as usize])
		});
		name
	}
}

/// The `Builtins` struct holds the built-in types and intrinsics every [`Program`] starts from, with an index of them
/// by name.
///
/// Building the built-ins takes about a thousand nodes, so build one `Builtins` per compiler, shader generator, or
/// test, keep it in that owner, and lend it to each [`Program::new`]. Every program then has the same built-ins at the
/// same node ids, under the same names, and one index answers for all of them.
pub struct Builtins {
	program: Program<'static>,
	/// The built-ins named by each name, in root order, indexed by the name's id.
	by_name: Vec<Vec<NodeId>>,
}

impl Builtins {
	/// Builds the built-in types and intrinsics and indexes them by name. Next, pass it to [`Program::new`].
	pub fn new() -> Self {
		let program = Program::build_builtins();
		let mut by_name = vec![Vec::new(); program.names.len() as usize];
		for id in program.builtins() {
			if let Some(name) = program[id].get_name() {
				by_name[name.0 as usize].push(*id);
			}
		}
		#[cfg(debug_assertions)]
		assert_builtins_hide_no_declarations(&program, &by_name);
		Self { program, by_name }
	}
}

impl Default for Builtins {
	fn default() -> Self {
		Self::new()
	}
}

/// Asserts what lets root lookups skip every built-in whose name differs from the one they look for: a lookup that
/// reaches into a built-in finds nothing that isn't itself a built-in by that name, and no built-in holds a declaration
/// a call could target.
///
/// A lookup can only return a node it reaches by name, and every node a built-in reaches is in `program`, so checking
/// each name `program` holds, other than the built-ins' own, covers every name a shader can look up.
#[cfg(debug_assertions)]
fn assert_builtins_hide_no_declarations(program: &Program, builtins_by_name: &[Vec<NodeId>]) {
	use super::resolution::{DescendantSearch, find_descendant};

	let hidden: std::collections::BTreeSet<Name> = program
		.nodes
		.iter()
		.filter_map(Node::get_name)
		.filter(|name| builtins_by_name[name.0 as usize].is_empty())
		.collect();
	for id in program.builtins() {
		let node = program[id].node();
		let declares_calls = match node {
			Nodes::Scope { .. } | Nodes::PushConstant { .. } => true,
			Nodes::Struct { fields, .. } => fields
				.iter()
				.any(|field| !matches!(program[field].node(), Nodes::Member { .. })),
			_ => false,
		};
		assert!(
			!declares_calls,
			"Built-in `{:?}` holds declarations a call could target.",
			program.name(*id)
		);
		for name in &hidden {
			assert!(
				find_descendant(program, id, *name, DescendantSearch::NonIntrinsic).is_none(),
				"Built-in `{:?}` lets a lookup reach `{}`, which the built-in index can't answer for.",
				program.name(*id),
				program.text(*name)
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

/// The `Program` struct owns every node of one linked BESL program in a single arena, so linking allocates one growing
/// list instead of one shared cell per node, and dropping a program frees it at once.
///
/// Create one with [`Program::new`] over a [`Builtins`] its owner keeps, which places the built-in types and
/// intrinsics under [`Program::root`]. The program borrows those built-ins for its lifetime `'b`. Add any declarations
/// the source may reference, and pass it to [`crate::compile_to_besl`]. Next, hand the linked program to a shader
/// backend or to [`crate::vm::ExecutableProgram::compile`].
#[derive(Clone)]
pub struct Program<'b> {
	nodes: Vec<Node>,
	root: NodeId,
	/// The identifiers the program's nodes name, each stored once.
	names: Names<'b>,
	/// How many of the root's first children are built-ins. The program's own declarations follow them, so built-in
	/// lookups stop here and never find a user type of the same name.
	builtins: usize,
	/// The built-ins this program started from, which answer lookups of the built-ins by name. `None` only while the
	/// built-ins themselves are built.
	shared: Option<&'b Builtins>,
}

impl<'b> Program<'b> {
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
	/// Lookups that reach the root take this instead of comparing against each of the about one thousand built-ins.
	pub(crate) fn builtins_named(&self, name: Name) -> &'b [NodeId] {
		let Some(shared) = self.shared else {
			return &[];
		};
		let named = shared.by_name.get(name.0 as usize).map_or(&[][..], Vec::as_slice);
		debug_assert!(
			named
				.iter()
				.all(|id| self.builtins().contains(id) && self[*id].get_name() == Some(name)),
			"The built-ins of this program don't match the shared built-in index."
		);
		named
	}

	/// Returns the name whose text is `text`, interning it if the program doesn't hold it yet.
	pub fn intern(&mut self, text: &str) -> Name {
		self.names.intern(text)
	}

	/// Returns the name whose text is `text`, or `None` when no node of the program can be named `text`.
	pub fn find_name(&self, text: &str) -> Option<Name> {
		self.names.find(text)
	}

	/// Returns the text of `name`.
	pub fn text(&self, name: Name) -> &str {
		self.names.text(name)
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
		self[id].get_name().map(|name| self.text(name))
	}

	/// Returns the direct child of `parent` named `child_name`, such as a function in a scope or a field in a struct.
	///
	/// Declaration containers (scopes, structs and intrinsics) search their declarations. Every other node searches
	/// the executable parts that [`Nodes::children`] yields.
	pub fn get_child(&self, parent: NodeId, child_name: &str) -> Option<NodeId> {
		// A name no node carries names no child, so the search compares interned names only.
		let child_name = self.find_name(child_name)?;
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
			.find(|child| self[*child].get_name() == Some(child_name))
			.copied()
	}

	/// Recursively searches under `parent` for a node named `child_name`.
	pub fn get_descendant(&self, parent: NodeId, child_name: &str) -> Option<NodeId> {
		find_descendant(self, &parent, self.find_name(child_name)?, DescendantSearch::Any)
	}

	/// Returns the program's `main` entry-point function.
	///
	/// Only functions count, and the search walks nested scopes, never struct fields or function bodies, so a member
	/// or local named `main` is never returned.
	pub fn get_main(&self) -> Option<NodeId> {
		self.find_main(self.root, self.find_name("main")?)
	}

	/// Finds the function named `main` under `id`. See [`Program::get_main`].
	fn find_main(&self, id: NodeId, main: Name) -> Option<NodeId> {
		match self[id].node() {
			Nodes::Function { name, .. } if *name == main => Some(id),
			Nodes::Scope { children, .. } => children.iter().find_map(|child| self.find_main(*child, main)),
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
					alias: self.text(*name).to_string(),
				}),
			};
		}
		r#type
	}
}

impl Index<NodeId> for Program<'_> {
	type Output = Node;

	fn index(&self, id: NodeId) -> &Node {
		&self.nodes[id.index()]
	}
}

impl IndexMut<NodeId> for Program<'_> {
	fn index_mut(&mut self, id: NodeId) -> &mut Node {
		&mut self.nodes[id.index()]
	}
}

impl Index<&NodeId> for Program<'_> {
	type Output = Node;

	fn index(&self, id: &NodeId) -> &Node {
		&self.nodes[id.index()]
	}
}

impl IndexMut<&NodeId> for Program<'_> {
	fn index_mut(&mut self, id: &NodeId) -> &mut Node {
		&mut self.nodes[id.index()]
	}
}

impl std::fmt::Debug for Program<'_> {
	/// Prints the program's own root declarations by name. The built-ins are the same in every program, so they are left
	/// out.
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_list()
			.entries(self.root_declarations().iter().map(|id| (self.name(*id), self[id].node())))
			.finish()
	}
}

/// Links a parsed program under the root of `program`, which supplies the built-in registry from [`Program::new`] and
/// any declarations the host added that the source may reference.
pub(crate) fn lex_with_root<'b>(mut program: Program<'b>, mut node: parser::Node) -> Result<Program<'b>, LexError> {
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

impl<'b> Program<'b> {
	/// Creates a program whose root scope holds a copy of `builtins`, ready for a host to add declarations and pass to
	/// [`crate::compile_to_besl`].
	pub fn new(builtins: &'b Builtins) -> Program<'b> {
		let prototype = &builtins.program;
		// Leave room for a typical shader's own nodes in the one allocation, so linking doesn't grow the arena step by
		// step.
		let mut nodes = Vec::with_capacity(prototype.nodes.len() * 3);
		nodes.extend_from_slice(&prototype.nodes);
		Program {
			nodes,
			root: prototype.root,
			// The program's own names go in a table over the built-ins' one, so the built-ins' names cost nothing here.
			names: Names::layered_over(Some(&prototype.names)),
			builtins: prototype.builtins,
			shared: Some(builtins),
		}
	}
}

impl Program<'static> {
	/// Builds the built-in types and intrinsics, which [`Builtins::new`] indexes and [`Program::new`] copies.
	// Keep the built-in registry contiguous so overload ordering and shared type handles remain auditable together.
	#[allow(clippy::too_many_lines)]
	fn build_builtins() -> Self {
		let mut owned = Program {
			nodes: Vec::with_capacity(2048),
			root: NodeId(0),
			names: Names::new(),
			builtins: 0,
			shared: None,
		};
		let program = &mut owned;
		let mut types = BuiltinTypes(Vec::with_capacity(32));
		for name in ["void", "bool", "u8", "u16", "u32", "i32", "f16", "f32"] {
			types.declare(program, name, Vec::new());
		}
		// Vectors and matrices, as the type name, the component type, and the component count.
		for (name, component, count) in [
			("vec2u16", "u16", 2),
			("vec4u16", "u16", 4),
			("vec2u", "u32", 2),
			("vec2i", "i32", 2),
			("vec2f16", "f16", 2),
			("vec2f", "f32", 2),
			("vec3f16", "f16", 3),
			("vec3f", "f32", 3),
			("vec3u", "u32", 3),
			("vec4u", "u32", 4),
			("vec4f16", "f16", 4),
			("vec4f", "f32", 4),
			("mat4f", "vec4f", 4),
			("mat4x3f", "vec3f", 4),
		] {
			let component = types.get(component);
			let fields = ["x", "y", "z", "w"][..count]
				.iter()
				.map(|axis| {
					let axis = program.intern(axis);
					program.add(Node::member(axis, component))
				})
				.collect();
			types.declare(program, name, fields);
		}
		for name in [
			"Texture2D",
			"Texture3D",
			"TextureCube",
			"TextureCubeArray",
			"ArrayTexture2D",
			"atomicu32",
			"atomici32",
			"AccelerationStructure",
		] {
			types.declare(program, name, Vec::new());
		}
		// A hit is a plain struct, so shaders can store its members and rebuild it with its constructor, which takes the
		// members in `HIT_MEMBERS` order.
		let hit_members = crate::HIT_MEMBERS
			.iter()
			.map(|(name, r#type)| {
				let r#type = types.get(r#type);
				let name = program.intern(name);
				program.add(Node::member(name, r#type))
			})
			.collect();
		types.declare(program, crate::HIT_STRUCT, hit_members);

		let mut builtins: Vec<NodeId> = types.0.iter().map(|(_, id)| *id).collect();
		// Vertex invocation indices and the fragment primitive index are implicit BESL values. Their placeholder
		// locations are never exposed as stage attributes; backends and the VM map them to dedicated built-ins.
		for (name, location) in [
			(crate::PRIMITIVE_INDEX_BUILTIN, u8::MAX - 2),
			(crate::VERTEX_INDEX_BUILTIN, u8::MAX - 1),
			(crate::INSTANCE_INDEX_BUILTIN, u8::MAX),
		] {
			let name = program.intern(name);
			builtins.push(program.add(Node::input(name, types.get("u32"), location)));
		}
		for signature in INTRINSICS {
			builtins.push(types.intrinsic(program, signature, &[]));
		}

		// Families whose overloads differ only in type: `T` stands for each operand type of the family's list and `R`
		// for the result type paired with it. Each list keeps its name's overload order, because call resolution
		// selects the first overload whose parameters match.
		let same = |types: &[&'static str]| types.iter().map(|r#type| (*r#type, *r#type)).collect::<Vec<_>>();
		// Vector reductions return the vector's scalar component type.
		let reduced =
			|types: [&'static str; 6]| types.map(|r#type| (r#type, if r#type.ends_with("16") { "f16" } else { "f32" }));
		let float_vectors = ["vec4f", "vec3f", "vec2f", "vec2f16", "vec3f16", "vec4f16"];
		// The float types every component-wise intrinsic accepts. Scalars come first so that a call whose arguments
		// are untyped literals, or a typed scalar and a literal, resolves to a scalar overload.
		let float_types = same(&["f32", "f16", "vec2f", "vec3f", "vec4f", "vec2f16", "vec3f16", "vec4f16"]);
		// The ordering intrinsics also accept integers, for index and texel arithmetic.
		let mut ordered_types = float_types.clone();
		ordered_types.extend(same(&["i32", "u32", "vec2i", "vec2u", "vec3u", "vec4u"]));
		let mut families = vec![
			(
				"dot(left: T, right: T) -> R",
				reduced(["vec2f", "vec4f", "vec3f", "vec2f16", "vec3f16", "vec4f16"]).to_vec(),
			),
			("length(value: T) -> R", reduced(float_vectors).to_vec()),
			("normalize(value: T) -> T", same(&float_vectors)),
			("max(left: T, right: T) -> T", ordered_types.clone()),
			("min(left: T, right: T) -> T", ordered_types.clone()),
			("clamp(value: T, minimum: T, maximum: T) -> T", ordered_types),
			("pow(value: T, exponent: T) -> T", float_types.clone()),
			("step(edge: T, value: T) -> T", float_types.clone()),
			("smoothstep(edge0: T, edge1: T, value: T) -> T", float_types.clone()),
			("fma(multiplicand: T, multiplier: T, addend: T) -> T", float_types.clone()),
		];
		families.extend(UNARY_FLOAT_INTRINSICS.map(|signature| (signature, float_types.clone())));
		families.extend([
			(
				"distance(left: T, right: T) -> R",
				reduced(["vec2f", "vec3f", "vec4f", "vec2f16", "vec3f16", "vec4f16"]).to_vec(),
			),
			("reflect(incident: T, normal: T) -> T", same(&["vec2f", "vec3f", "vec4f"])),
		]);
		families.extend(
			[
				"is_nan(value: T) -> bool",
				"is_infinite(value: T) -> bool",
				"is_finite(value: T) -> bool",
				"is_normal(value: T) -> bool",
			]
			.map(|signature| (signature, same(&["f16", "f32"]))),
		);
		// `mix` blends two same-typed values by one scalar factor.
		families.push((
			"mix(left: T, right: T, factor: f32) -> T",
			same(&["f32", "vec2f", "vec3f", "vec4f"]),
		));
		// Conversions take one `value` of the source type and return the target type.
		families.extend([
			("f16(value: T) -> f16", same(&["f32", "f16", "u32", "i32"])),
			("u16(value: T) -> u16", same(&["u32"])),
			("u8(value: T) -> u8", same(&["u32"])),
			("f32(value: T) -> f32", same(&["f16", "u32", "i32"])),
			("vec2f16(value: T) -> vec2f16", same(&["vec2f", "vec2f16"])),
			("vec3f16(value: T) -> vec3f16", same(&["vec3f", "vec3f16"])),
			("vec4f16(value: T) -> vec4f16", same(&["vec4f", "vec4f16"])),
			("vec2f(value: T) -> vec2f", same(&["vec2f16"])),
			("vec3f(value: T) -> vec3f", same(&["vec3f16"])),
			("vec4f(value: T) -> vec4f", same(&["vec4f16"])),
			("u32(value: T) -> u32", same(&["u32", "u8", "u16", "i32", "f16", "f32"])),
		]);
		for (signature, overloads) in families {
			for (operand, result) in overloads {
				builtins.push(types.intrinsic(program, signature, &[("T", operand), ("R", result)]));
			}
		}
		for signature in INVOCATION_INTRINSICS {
			builtins.push(types.intrinsic(program, signature, &[]));
		}
		// The relaxed scalar atomic surface, for the unsigned and the signed 32-bit type: `A` is the atomic type and
		// `T` its scalar type.
		for (atomic, scalar) in [("atomicu32", "u32"), ("atomici32", "i32")] {
			for signature in ATOMIC_INTRINSICS {
				builtins.push(types.intrinsic(program, signature, &[("A", atomic), ("T", scalar)]));
			}
		}

		let builtin_count = builtins.len();
		let root_name = program.intern("root");
		let root = program.add(Node::new(Nodes::Scope {
			name: root_name,
			children: builtins,
		}));
		owned.root = root;
		owned.builtins = builtin_count;
		owned
	}
}

/// The `BuiltinTypes` struct names the built-in types declared so far, so the intrinsic tables can name the types they
/// take and return.
struct BuiltinTypes(Vec<(&'static str, NodeId)>);

impl BuiltinTypes {
	/// Declares the built-in struct `name` with `fields`, which is empty for a primitive or resource type.
	fn declare(&mut self, program: &mut Program, name: &'static str, fields: Vec<NodeId>) {
		let interned = program.intern(name);
		self.0.push((name, program.add(Node::r#struct(interned, fields))));
	}

	fn get(&self, name: &str) -> NodeId {
		self.0
			.iter()
			.find(|(declared, _)| *declared == name)
			.map(|(_, id)| *id)
			.unwrap_or_else(|| panic!("Built-in type `{name}` isn't declared. The most likely cause is an intrinsic table entry that names a type declared after it, or a misspelled type. Declare the type first."))
	}

	/// Declares the intrinsic `signature` describes, written `name(parameter: type, ...) -> type`. A type named like a
	/// key of `substitutions` stands for that key's type.
	fn intrinsic(&self, program: &mut Program, signature: &str, substitutions: &[(&str, &str)]) -> NodeId {
		let r#type = |name: &str| {
			let name = substitutions
				.iter()
				.find(|(key, _)| *key == name)
				.map_or(name, |(_, r#type)| r#type);
			self.get(name)
		};
		let malformed =
			|| format!("Intrinsic signature `{signature}` is malformed. Write it as `name(parameter: type, ...) -> type`.");
		let (name, rest) = signature.split_once('(').unwrap_or_else(|| panic!("{}", malformed()));
		let (parameters, result) = rest.split_once(") -> ").unwrap_or_else(|| panic!("{}", malformed()));
		let parameters = parameters
			.split(", ")
			.filter(|parameter| !parameter.is_empty())
			.map(|parameter| {
				let (name, type_name) = parameter.split_once(": ").unwrap_or_else(|| panic!("{}", malformed()));
				(name, r#type(type_name))
			})
			.collect();
		builtin_intrinsic(program, name, parameters, r#type(result))
	}
}

/// The built-in intrinsics with one signature each, in overload order, as [`BuiltinTypes::intrinsic`] reads them.
const INTRINSICS: [&str; 46] = [
	"sample(texture_sampler: Texture2D, uv: vec2f) -> vec4f",
	"texture_lod(texture: Texture2D, uv: vec2f) -> vec4f",
	"texture_lod(texture: Texture2D, uv: vec2f, lod: f32) -> vec4f",
	"texture_lod(texture: Texture3D, uv: vec3f) -> vec4f",
	"texture_lod(texture: TextureCube, direction: vec3f, lod: f32) -> vec4f",
	"texture_cube_array_lod(texture: TextureCubeArray, direction: vec3f, cube: u32, lod: f32) -> vec4f",
	"downsample_min(texture: Texture2D, uv: vec2f, lod: f32) -> f32",
	"downsample_max(texture: Texture2D, uv: vec2f, lod: f32) -> f32",
	"downsample_max(texture: ArrayTexture2D, uv: vec2f, layer: u32, lod: f32) -> f32",
	// `gather` returns the first channel of the four texels around a normalized UV in mip zero, in the order every
	// platform shares: x and y are the quad's second row, left then right, and z and w its first row, right then left,
	// with rows ordered by v.
	"gather(texture: Texture2D, uv: vec2f) -> vec4f",
	"gather(texture: ArrayTexture2D, uv: vec2f, layer: u32) -> vec4f",
	"fetch(texture: Texture2D, coord: vec2u) -> vec4f",
	"fetch(texture: ArrayTexture2D, coord: vec2u, layer: u32) -> vec4f",
	"fetch_u32(texture: Texture2D, coord: vec2u) -> u32",
	"cross(left: vec3f, right: vec3f) -> vec3f",
	"find_lsb(value: u32) -> u32",
	"atan2(y: f32, x: f32) -> f32",
	"sincos(value: f32) -> vec2f",
	"round_to_i32(value: vec2f) -> vec2i",
	"subgroup_ballot(predicate: bool) -> vec4u",
	"subgroup_ballot_any(mask: vec4u) -> bool",
	"subgroup_ballot_find_lsb(mask: vec4u) -> u32",
	"subgroup_ballot_count(mask: vec4u) -> u32",
	"subgroup_ballot_and_not(mask: vec4u, removed: vec4u) -> vec4u",
	"subgroup_broadcast_u32(value: u32, source_lane: u32) -> u32",
	"subgroup_broadcast_f32(value: f32, source_lane: u32) -> f32",
	"subgroup_shuffle_xor_f32(value: f32, mask: u32) -> f32",
	"workgroup_barrier() -> void",
	"set_task_mesh_output_count(count: u32) -> void",
	"thread_id() -> vec2u",
	"set_mesh_output_counts(vertex_count: u32, primitive_count: u32) -> void",
	"set_mesh_vertex_position(vertex_index: u32, position: vec4f) -> void",
	"set_mesh_triangle(primitive_index: u32, triangle: vec3u) -> void",
	"set_mesh_primitive_render_target_array_index(primitive_index: u32, array_index: u32) -> void",
	"image_load(image: Texture2D, coord: vec2u) -> vec4f",
	"image_load_u32(image: Texture2D, coord: vec2u) -> u32",
	"texture_size(texture: Texture2D) -> vec2u",
	"texture_size(texture: ArrayTexture2D) -> vec2u",
	"image_size(image: Texture2D) -> vec2u",
	"guard_image_bounds(image: Texture2D, coord: vec2u) -> void",
	"write(image: Texture2D, coord: vec2u, value: vec4f) -> void",
	// A tile stage writes its `vec4f` `output` declarations at a pixel. This comes after the storage-image form, which a
	// resource argument of unknown type matches first.
	"write(output: vec4f, pixel: vec2u, value: vec4f) -> void",
	"output_extent() -> vec2u",
	// Writes one unsigned texel of an `r32ui` storage image, the counterpart of `image_load_u32`.
	"write_u32(image: Texture2D, coord: vec2u, value: u32) -> void",
	"image_atomic_or(image: Texture2D, coord: vec2u, value: u32) -> u32",
	"image_atomic_max(image: Texture2D, coord: vec2u, value: u32) -> u32",
];

/// The component-wise float intrinsics that take one value and return its type, declared for every float type.
const UNARY_FLOAT_INTRINSICS: [&str; 17] = [
	"abs(value: T) -> T",
	"sign(value: T) -> T",
	"sqrt(value: T) -> T",
	"inversesqrt(value: T) -> T",
	"exp(value: T) -> T",
	"exp2(value: T) -> T",
	"log2(value: T) -> T",
	"sin(value: T) -> T",
	"cos(value: T) -> T",
	"tan(value: T) -> T",
	"asin(value: T) -> T",
	"radians(value: T) -> T",
	"floor(value: T) -> T",
	"ceil(value: T) -> T",
	"round(value: T) -> T",
	"fract(value: T) -> T",
	"fwidth(value: T) -> T",
];

/// The intrinsics that read the invocation: its coordinates, and for ray tracing stages the launch point or the hit
/// being shaded. The ray queries share one signature and differ in what they report. `invoke` isn't here: its payload
/// type comes from the program, so the lexer instantiates it per program.
const INVOCATION_INTRINSICS: [&str; 16] = [
	"thread_idx() -> u32",
	"subgroup_lane_index() -> u32",
	"threadgroup_position() -> u32",
	"thread_position() -> u32",
	"thread_layer() -> u32",
	"intersect(scene: AccelerationStructure, mask: u32, origin: vec3f, direction: vec3f, t_min: f32, t_max: f32) -> Hit",
	"trace_occlusion(scene: AccelerationStructure, mask: u32, origin: vec3f, direction: vec3f, t_min: f32, t_max: f32) -> bool",
	"launch_index() -> vec3u",
	"launch_size() -> vec3u",
	"hit_distance() -> f32",
	"hit_custom_index() -> u32",
	"hit_geometry_index() -> u32",
	"hit_primitive_index() -> u32",
	"hit_barycentrics() -> vec2f",
	"ray_origin() -> vec3f",
	"ray_direction() -> vec3f",
];

/// The buffer and workgroup atomics, where `A` is an atomic type and `T` its scalar type.
const ATOMIC_INTRINSICS: [&str; 11] = [
	"atomic_load(value: A) -> T",
	"atomic_store(value: A, stored: T) -> void",
	"atomic_exchange(value: A, stored: T) -> T",
	"atomic_compare_exchange(value: A, expected: T, desired: T) -> T",
	"atomic_add(value: A, increment: T) -> T",
	"atomic_sub(value: A, decrement: T) -> T",
	"atomic_min(value: A, candidate: T) -> T",
	"atomic_max(value: A, candidate: T) -> T",
	"atomic_and(value: A, mask: T) -> T",
	"atomic_or(value: A, mask: T) -> T",
	"atomic_xor(value: A, mask: T) -> T",
];

impl Node {
	/// Creates a `use` declaration that brings `target` in by `name`, or, with no `name`, every declaration of the
	/// module `target`.
	pub fn r#use(name: Option<Name>, target: &NodeId) -> Node {
		Node {
			node: Nodes::Use { name, target: *target },
		}
	}

	/// Creates a scope that groups child nodes.
	pub fn scope(name: Name) -> Node {
		Node {
			node: Nodes::Scope {
				name,
				children: Vec::with_capacity(16),
			},
		}
	}

	/// Creates a named struct definition from its fields.
	pub fn r#struct(name: Name, fields: Vec<NodeId>) -> Node {
		Node {
			node: Nodes::Struct {
				name,
				template: None,
				fields,
				types: Vec::new(),
			},
		}
	}

	pub fn member(name: Name, r#type: NodeId) -> Node {
		Node {
			node: Nodes::Member {
				name,
				r#type,
				count: None,
			},
		}
	}

	pub fn array(name: Name, r#type: NodeId, size: usize) -> Node {
		Node {
			node: Nodes::Member {
				name,
				r#type,
				count: Some(NonZeroUsize::new(size).expect("Invalid size")),
			},
		}
	}

	pub fn function(name: Name, params: Vec<NodeId>, return_type: NodeId, statements: Vec<NodeId>) -> Node {
		Node {
			node: Nodes::Function {
				name,
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

	pub fn for_loop(initializer: NodeId, condition: NodeId, update: NodeId, statements: Vec<NodeId>) -> Node {
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
	pub fn binding(name: Name, r#type: BindingTypes, slot: u32, read: bool, write: bool) -> Node {
		Self::binding_in_memory(name, r#type, slot, read, write, BufferMemoryClass::Device, None)
	}

	/// Builds a binding whose memory class is independent from its read and write access. Pass a `count` to bind an
	/// array of that many resources, as [`Self::binding_array`] does.
	pub fn binding_in_memory(
		name: Name,
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
				name,
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

	pub fn binding_array(name: Name, r#type: BindingTypes, slot: u32, read: bool, write: bool, count: usize) -> Node {
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

	pub fn intrinsic(name: Name, elements: Vec<NodeId>, r#return: NodeId) -> Node {
		Node {
			node: Nodes::Intrinsic {
				name,
				elements,
				r#return,
			},
		}
	}

	/// Declares a host-selected constant whose backend index starts at `id`. See [`crate::parser::Node::specialization`].
	pub fn specialization(name: Name, r#type: NodeId, id: u32) -> Node {
		Node {
			node: Nodes::Specialization { name, r#type, id },
		}
	}

	pub fn constant(name: Name, r#type: NodeId, value: NodeId) -> Node {
		Node {
			node: Nodes::Const { name, r#type, value },
		}
	}

	pub fn input(name: Name, format: NodeId, location: u8) -> Node {
		Node {
			node: Nodes::Input {
				name,
				format,
				location,
				per_primitive: false,
			},
		}
	}

	pub fn output(name: Name, format: NodeId, location: u8) -> Node {
		Self::output_array(name, format, location, None, false)
	}

	/// Declares a mesh output array of `count` elements, with one element per vertex, which rasterization
	/// interpolates, when `per_vertex` is set, or one flat element per primitive otherwise. Without a `count` it
	/// declares a plain stage output, like [`Self::output`].
	pub fn output_array(name: Name, format: NodeId, location: u8, count: Option<NonZeroUsize>, per_vertex: bool) -> Node {
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

	pub fn task_payload(name: Name, format: NodeId, count: NonZeroUsize) -> Node {
		Node {
			node: Nodes::TaskPayload { name, format, count },
		}
	}

	pub fn workgroup(name: Name, format: NodeId, count: Option<NonZeroUsize>) -> Node {
		Node {
			node: Nodes::Workgroup { name, format, count },
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

	pub fn get_name(&self) -> Option<Name> {
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
			| Nodes::Workgroup { name, .. } => Some(*name),
			Nodes::PushConstant { .. } => Some(Name::PUSH_CONSTANT),
			Nodes::Expression(Expressions::VariableDeclaration { name, .. } | Expressions::Member { name, .. }) => Some(*name),
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
		/// Whether the image is a 2D array, a `StorageImageArray`, whose layer shaders select by indexing it, as
		/// `result[face]`. Backends bind it as one array view and pass the layer to each load and store.
		layered: bool,
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

#[derive(Clone, Debug)]
pub enum Nodes {
	/// A module, or the program root. Code inside it sees its declarations by name, and code outside reaches them
	/// through a path such as `module::name` or a [`Nodes::Use`].
	Scope {
		name: Name,
		children: Vec<NodeId>,
	},
	/// A `use` declaration. Name lookups in the scope that holds it see `target` by `name`. With no `name`, from
	/// `use module::*;`, they see every declaration of the module `target` instead.
	///
	/// A module can bring in one of its own ancestors. Backends emit nothing for it.
	Use {
		name: Option<Name>,
		target: NodeId,
	},
	/// A layer: the descriptors, push constant block, and tile targets a shader consumes through one parameter of the
	/// layer's type.
	///
	/// Its descriptors are bindings in the layout named `path`, the layer's full path, so they form one descriptor set.
	/// Code reaches a member through a layer value, such as `scene.views`, never by its bare name.
	Layer {
		name: Name,
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
		name: Name,
	},
	Struct {
		name: Name,
		template: Option<NodeId>,
		fields: Vec<NodeId>,
		types: Vec<NodeId>,
	},
	Member {
		name: Name,
		r#type: NodeId,
		count: Option<NonZeroUsize>,
	},
	Function {
		name: Name,
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
		name: Name,
		r#type: NodeId,
		/// The first backend constant index. Vector components take consecutive indices.
		id: u32,
	},
	Expression(Expressions),
	Binding {
		name: Name,
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
		name: Name,
		elements: Vec<NodeId>,
		r#return: NodeId,
	},
	Input {
		name: Name,
		format: NodeId,
		location: u8,
		/// Whether a fragment input reads the value its mesh stage writes once per primitive, from an `output` array,
		/// instead of a value interpolated across the triangle. Vulkan links the two only when both declare it.
		per_primitive: bool,
	},
	Output {
		name: Name,
		format: NodeId,
		location: u8,
		count: Option<NonZeroUsize>,
		/// Whether a mesh output array holds one element per vertex, interpolated across each triangle, instead of
		/// one flat element per primitive.
		per_vertex: bool,
	},
	TaskPayload {
		name: Name,
		format: NodeId,
		count: NonZeroUsize,
	},
	Workgroup {
		name: Name,
		format: NodeId,
		count: Option<NonZeroUsize>,
	},
	Parameter {
		name: Name,
		r#type: NodeId,
	},
	/// A named module-level value known at compile time.
	Const {
		name: Name,
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
					.map(|name| program.text(name))
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
		name: Name,
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
		name: Name,
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
			let name = program.intern(name);
			program.add(Node::new(Nodes::Parameter { name, r#type }))
		})
		.collect();
	let name = program.intern(name);
	program.add(Node::intrinsic(name, parameters, r#return))
}

/// Built-in scalar types with a byte representation in storage buffers. `bool`, `void`, and resource handles have none.
pub(crate) const STORABLE_SCALAR_TYPES: [&str; 6] = ["u8", "u16", "u32", "i32", "f16", "f32"];
