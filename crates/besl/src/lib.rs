//! Use this crate to parse, link, and execute Byte Engine Shader Language (BESL) source.
//!
//! Call [`compile_to_besl`] for the normal parse-and-link path. Next, pass the
//! linked [`NodeReference`] to the resource-management shader generator or use
//! [`vm`] when tests need to execute BESL semantics directly.
//!
//! See the [BESL language reference](/docs/reference/besl)
//! for syntax, interfaces, stages, sidecar settings, and supported operations.

// Parser and VM workflows are intentionally explicit; keep heuristic style lints quiet until those pipelines are redesigned.
#![allow(
	clippy::cognitive_complexity,
	clippy::excessive_nesting,
	clippy::mutable_key_type,
	clippy::too_many_lines
)]

pub mod lexer;
mod modules;
pub mod optimization;
pub mod parser;
pub mod sampler;
mod signatures;
mod tokenizer;
pub mod vm;
mod writer;

pub use lexer::Expressions;
pub use lexer::Node;
pub use lexer::Nodes;
pub use lexer::Operators;
pub use lexer::UnaryOperators;
pub use lexer::infer_expression_type;

pub use crate::lexer::NodeReference;
pub use crate::lexer::{
	BindingTypes, BufferMemoryClass, CallTarget, ElseBranch, FixedArray, MAX_DESCRIPTOR_SETS, MatchArm, Placement,
	place_descriptors,
};
pub use crate::modules::{LoadedModule, ModuleLoader, ModuleQuery, attach_modules};
pub use crate::sampler::SamplerState;
pub use crate::writer::write;

/// The base of every link to the online documentation. Set `BYTE_ENGINE_DOCS_BASE_URL` at build time to link elsewhere.
///
/// This mirrors `online_docs_url` in `byte-engine-utils`, kept here so BESL doesn't depend on that crate.
const ONLINE_DOCS_BASE_URL: &str = match option_env!("BYTE_ENGINE_DOCS_BASE_URL") {
	Some(url) => url,
	None => "https://byte-engine.0x44491229.dev/docs",
};

/// Builds a link to one online documentation page, such as the language rule a compile error points to.
pub(crate) fn online_docs_url(path: &str) -> String {
	format!(
		"{}/{}",
		ONLINE_DOCS_BASE_URL.trim_end_matches('/'),
		path.trim_start_matches('/')
	)
}

/// Names the symbol a vertex shader's `input.vertex_index` lowers to: the current invocation's vertex index.
///
/// Each graphics backend binds it to its native vertex system value, and the VM to
/// [`builtin_vertex_index_slot`](crate::vm::builtin_vertex_index_slot). The `_besl_stage_` prefix, shared by every
/// stage-input symbol, keeps the field's own name free for the shader's variables, so a `let vertex_index` can't
/// collide with it after lowering.
pub const VERTEX_INDEX_BUILTIN: &str = "_besl_stage_vertex_index";

/// Names the symbol a vertex shader's `input.instance_index` lowers to: the current invocation's instance index.
///
/// Each graphics backend binds it to its native instance system value, and the VM to
/// [`builtin_instance_index_slot`](crate::vm::builtin_instance_index_slot).
pub const INSTANCE_INDEX_BUILTIN: &str = "_besl_stage_instance_index";

/// Names the symbol a fragment shader's `input.primitive_index` lowers to: the index of the primitive that produced
/// the current fragment.
///
/// It counts the primitives of one draw from zero. Each graphics backend binds it to its native primitive ID, and the
/// VM to [`builtin_primitive_index_slot`](crate::vm::builtin_primitive_index_slot).
pub const PRIMITIVE_INDEX_BUILTIN: &str = "_besl_stage_primitive_index";

/// Names the collision-free semantic output used for a structural vertex `position` field.
pub const STRUCTURAL_POSITION_OUTPUT: &str = "_besl_interface_position";

/// Names the vertex interface field that picks the render target array layer a primitive rasterizes into.
///
/// A vertex shader returns it like `position`: it takes no varying location, the next stage doesn't declare it, and
/// each graphics backend writes it to its native layer output. Mesh shaders pick the layer with
/// `set_mesh_primitive_render_target_array_index` instead.
pub const RENDER_TARGET_ARRAY_INDEX_FIELD: &str = "render_target_array_index";

/// Names the collision-free semantic output used for a structural vertex [`RENDER_TARGET_ARRAY_INDEX_FIELD`] field.
pub const STRUCTURAL_RENDER_TARGET_ARRAY_INDEX_OUTPUT: &str = "_besl_interface_render_target_array_index";

/// Names the built-in struct that `intersect` returns and `invoke` takes.
pub const HIT_STRUCT: &str = "Hit";

/// The members of the built-in [`HIT_STRUCT`], as name and type pairs in declaration order.
///
/// The lexer declares the struct from this list, and backends that declare the struct themselves, such as the MSL
/// generator, read it too, so both agree on the layout. Shaders rebuild a hit with its constructor, which takes the
/// members in this order.
pub const HIT_MEMBERS: [(&str, &str); 8] = [
	("record", "u32"),
	("distance", "f32"),
	("custom_index", "u32"),
	("geometry_index", "u32"),
	("primitive_index", "u32"),
	("barycentrics", "vec2f"),
	("origin", "vec3f"),
	("direction", "vec3f"),
];

/// Reports whether an emitted output name carries the native vertex position.
///
/// Structural entry points use a reserved symbol so `return { position }` can read a local
/// named `position`. The legacy flat interface spelling remains valid during shader migration.
pub fn is_position_output(name: &str) -> bool {
	matches!(name, "position" | STRUCTURAL_POSITION_OUTPUT)
}

/// Reports whether an emitted output name carries the native render target array layer.
///
/// Backends call this where they call [`is_position_output`], to write the output to their layer builtin instead of a
/// varying.
pub fn is_render_target_array_index_output(name: &str) -> bool {
	name == STRUCTURAL_RENDER_TARGET_ARRAY_INDEX_OUTPUT
}

/// Returns the `output` declaration that `expression` names, looking through parentheses, or `None` when it names
/// anything else.
///
/// A tile shader's `write` takes one of its outputs where other stages pass a storage image, so backends call this to
/// tell the two apart.
pub fn referenced_output(expression: &NodeReference) -> Option<NodeReference> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => referenced_output(&elements[0]),
		Nodes::Expression(Expressions::Member { source, .. }) if matches!(source.borrow().node(), Nodes::Output { .. }) => {
			Some(source.clone())
		}
		_ => None,
	}
}

/// Returns the binding declaration that `expression` names, looking through parentheses, member access, and array
/// indexing, or `None` when it names anything else.
///
/// Backends call this to find the declaration behind an intrinsic's resource argument, such as `images[index]`.
pub fn referenced_binding(expression: &NodeReference) -> Option<NodeReference> {
	let borrowed = expression.borrow();
	match borrowed.node() {
		Nodes::Binding { .. } => Some(expression.clone()),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => referenced_binding(&elements[0]),
		Nodes::Expression(Expressions::Member { source, .. }) => referenced_binding(source),
		Nodes::Expression(Expressions::Accessor { left, .. }) => referenced_binding(left),
		_ => None,
	}
}

/// A shared parser node used by BESL syntax trees.
pub type ParserNode<'a> = parser::Node<'a>;

/// Parses BESL source and returns the root syntax node.
///
/// This function tokenizes the source and builds a syntax tree. Call [`lex`] to
/// resolve the tree's named references before compilation.
pub fn parse<'a>(source: &'a str) -> Result<parser::Node<'a>, CompilationError> {
	let mut program = parser::parse(&tokenizer::tokenize(source)).map_err(CompilationError::Parsing)?;
	modules::gather_root_modules(&mut program);
	Ok(program)
}

/// Resolves a parsed syntax tree and returns its linked root node.
///
/// The linked tree contains the resolved relationships needed by later
/// compilation stages. Next, give the returned [`NodeReference`] to a shader
/// generator or to [`vm`] for semantic execution.
pub fn lex(node: parser::Node) -> Result<NodeReference, CompilationError> {
	let besl = lexer::lex_with_root(Node::root(), node).map_err(CompilationError::Lex)?;

	Ok(besl)
}

/// Parses and links BESL source into a JSPD.
///
/// When `parent` is present, the compiled source can resolve names from that
/// parent scope. Next, pass the returned [`NodeReference`] to the active shader
/// generator, or use [`vm`] to validate behavior in a test.
pub fn compile_to_besl(source: &str, parent: Option<Node>) -> Result<NodeReference, CompilationError> {
	if source.split_whitespace().next().is_none() {
		return Ok(lexer::Node::scope("".to_string()).into());
	}

	let parser_root_node = parse(source)?;

	lexer::lex_with_root(parent.unwrap_or_else(Node::root), parser_root_node).map_err(CompilationError::Lex)
}

#[derive(Debug)]
pub enum CompilationError {
	Tokenization,
	Parsing(parser::ParsingFailReasons),
	Lex(lexer::LexError),
}
