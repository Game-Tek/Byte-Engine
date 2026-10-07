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
pub mod optimization;
pub mod parser;
mod tokenizer;
pub mod vm;

pub use lexer::Expressions;
pub use lexer::Node;
pub use lexer::Nodes;
pub use lexer::Operators;
pub use lexer::UnaryOperators;
pub use lexer::infer_expression_type;

pub use crate::lexer::NodeReference;
pub use crate::lexer::{BindingTypes, BufferMemoryClass, CallTarget, ElseBranch, FixedArray, MatchArm};

/// Names the current vertex invocation's vertex index.
///
/// Vertex shaders can read this implicit `u32` value as `vertex_index` without declaring an
/// interface input. Each graphics backend supplies its native vertex system value.
pub const VERTEX_INDEX_BUILTIN: &str = "vertex_index";

/// Names the current vertex invocation's instance index.
///
/// Vertex shaders can read this implicit `u32` value as `instance_index` without declaring an
/// interface input. Each graphics backend supplies its native instance system value.
pub const INSTANCE_INDEX_BUILTIN: &str = "instance_index";

/// Names the collision-free semantic output used for a structural vertex `position` field.
pub const STRUCTURAL_POSITION_OUTPUT: &str = "_besl_interface_position";

/// Reports whether an emitted output name carries the native vertex position.
///
/// Structural entry points use a reserved symbol so `return { position }` can read a local
/// named `position`. The legacy flat interface spelling remains valid during shader migration.
pub fn is_position_output(name: &str) -> bool {
	matches!(name, "position" | STRUCTURAL_POSITION_OUTPUT)
}

/// A shared parser node used by BESL syntax trees.
pub type ParserNode<'a> = parser::Node<'a>;

/// Parses BESL source and returns the root syntax node.
///
/// This function tokenizes the source and builds a syntax tree. Call [`lex`] to
/// resolve the tree's named references before compilation.
pub fn parse<'a>(source: &'a str) -> Result<parser::Node<'a>, CompilationError> {
	parser::parse(&tokenizer::tokenize(source)).map_err(CompilationError::Parsing)
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
