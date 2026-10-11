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

/// Places layout layers under a parsed module so the module can reference their declarations.
///
/// A layout layer is BESL source without a `main` function. It holds the declarations several shaders share, such as
/// structs, descriptors, a push constant block, constants, and helper functions. Use layers to declare a descriptor
/// set or a whole pipeline layout once, then compile each shader module against the layers it needs.
///
/// `layers` yields each layer's name and source. Layers stack in that order: each layer can reference the layers
/// before it, and the module can reference all of them. A layer's name is used only in error messages. A declaration
/// name may appear only once across the module and its layers, so a stale copy left in a shader fails instead of
/// silently diverging from the layer.
///
/// Next, pass the returned node to [`lex`].
pub fn stack_layers<'a>(
	mut module: parser::Node<'a>,
	layers: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<parser::Node<'a>, CompilationError> {
	/// Returns the name a top-level declaration occupies. Push constant blocks share one reserved name.
	fn declared_name<'a>(node: &parser::Node<'a>) -> Option<&'a str> {
		match node.node() {
			parser::Nodes::PushConstant { .. } => Some("push_constant"),
			_ => node.name(),
		}
	}

	let invalid = |message: String| CompilationError::Lex(lexer::LexError::invalid(message));

	// Every layer declaration in stacking order, and the name and layer of each one that has a name.
	let mut stacked = Vec::new();
	let mut declared: Vec<(&str, &str)> = Vec::new();

	for (layer, source) in layers {
		let mut root = parse(source).map_err(|error| {
			invalid(format!(
				"Layout layer `{layer}` did not parse ({error:?}). The most likely cause is invalid BESL syntax in the layer."
			))
		})?;

		// `parse` always returns a scope.
		let parser::Nodes::Scope { children, .. } = root.node_mut() else {
			continue;
		};

		for declaration in children.drain(..) {
			if let Some(name) = declared_name(&declaration) {
				if name == "main" {
					return Err(invalid(format!(
						"Layout layer `{layer}` declares `main`. The most likely cause is that a shader was listed as a layer; a layer holds only shared declarations."
					)));
				}

				if let Some((_, owner)) = declared.iter().find(|(declared, _)| *declared == name) {
					return Err(invalid(format!(
						"Layout layer `{layer}` redeclares `{name}`, which layer `{owner}` already declares. The most likely cause is that both layers describe the same resource; keep the declaration in one of them."
					)));
				}

				declared.push((name, layer));
			}

			stacked.push(declaration);
		}
	}

	let parser::Nodes::Scope { children, .. } = module.node_mut() else {
		return Err(invalid(
			"Invalid shader module: the parsed node is not a scope. The most likely cause is that the node is not the root that besl::parse returns.".to_string(),
		));
	};

	for name in children.iter().filter_map(declared_name) {
		if let Some((_, owner)) = declared.iter().find(|(declared, _)| *declared == name) {
			return Err(invalid(format!(
				"The shader redeclares `{name}`, which layout layer `{owner}` already declares. The most likely cause is a declaration left in the shader after it moved to the layer; delete the shader's copy."
			)));
		}
	}

	// Layer declarations go first so the lexer links them before the module declarations that reference them.
	children.splice(0..0, stacked);

	Ok(module)
}

#[derive(Debug)]
pub enum CompilationError {
	Tokenization,
	Parsing(parser::ParsingFailReasons),
	Lex(lexer::LexError),
}
