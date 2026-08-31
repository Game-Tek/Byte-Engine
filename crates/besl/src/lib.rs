//! Use this crate to parse, link, and execute Byte Engine Shader Language (BESL) source.
//!
//! Call [`compile_to_besl`] for the normal parse-and-link path. Next, pass the
//! linked [`NodeReference`] to the resource-management shader generator or use
//! [`vm`] when tests need to execute BESL semantics directly.
//!
//! See the [BESL language reference](/docs/reference/besl)
//! for syntax, interfaces, stages, sidecar settings, and supported operations.

pub mod lexer;
pub mod optimization;
pub mod parser;
mod tokenizer;
pub mod vm;

pub use besl_derive::BeslStruct;
pub use lexer::Expressions;
pub use lexer::Node;
pub use lexer::Nodes;
pub use lexer::Operators;

pub use crate::lexer::NodeReference;
pub use crate::lexer::{BindingTypes, BufferMemoryClass};

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

/// The `BeslStructDefinition` trait exposes a Rust struct as a BESL parser struct definition.
pub trait BeslStructDefinition {
	fn besl_struct_node() -> ParserNode<'static>;

	fn besl_definition(&self) -> ParserNode<'static> {
		Self::besl_struct_node()
	}
}

/// Builds a BESL parser struct node from Rust-style struct syntax.
#[macro_export]
macro_rules! besl_struct_node {
	(struct $name:ident { $($body:tt)* }) => {{
		let mut fields = Vec::new();
		$crate::besl_struct_node!(@fields fields [] $($body)*);

		$crate::ParserNode::r#struct(
			stringify!($name),
			fields,
		)
	}};
	(@fields $fields:ident [] ) => {};
	(@fields $fields:ident [$($field:tt)+] ) => {
		$crate::besl_struct_node!(@emit $fields [$($field)+]);
	};
	(@fields $fields:ident [$($field:tt)*] , $($rest:tt)*) => {
		$crate::besl_struct_node!(@emit $fields [$($field)*]);
		$crate::besl_struct_node!(@fields $fields [] $($rest)*);
	};
	(@fields $fields:ident [$($field:tt)*] $next:tt $($rest:tt)*) => {
		$crate::besl_struct_node!(@fields $fields [$($field)* $next] $($rest)*);
	};
	(@emit $fields:ident []) => {};
	(@emit $fields:ident [$field:ident : $($field_type:tt)+]) => {
		{
			let field_type = stringify!($($field_type)+).replace(' ', "");
			$fields.push($crate::ParserNode::member(stringify!($field), &field_type));
		}
	};
}

/// Parses BESL source and returns the root syntax node.
///
/// This function tokenizes the source and builds a syntax tree. Call [`lex`] to
/// resolve the tree's named references before compilation.
pub fn parse<'a>(source: &'a str) -> Result<parser::Node<'a>, CompilationError> {
	let tokens = tokenizer::tokenize(source).map_err(|_e| CompilationError::Tokenization)?;
	let parser_root_node = parser::parse(&tokens).map_err(CompilationError::Parsing)?;

	Ok(parser_root_node)
}

/// Resolves a parsed syntax tree and returns its linked root node.
///
/// The linked tree contains the resolved relationships needed by later
/// compilation stages. Next, give the returned [`NodeReference`] to a shader
/// generator or to [`vm`] for semantic execution.
pub fn lex(node: parser::Node) -> Result<NodeReference, CompilationError> {
	let besl = lexer::lex(node).map_err(CompilationError::Lex)?;

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

	let besl = if let Some(parent) = parent {
		lexer::lex_with_root(parent, parser_root_node).map_err(CompilationError::Lex)?
	} else {
		lexer::lex(parser_root_node).map_err(CompilationError::Lex)?
	};

	Ok(besl)
}

#[derive(Debug)]
pub enum CompilationError {
	Undefined,
	Tokenization,
	Parsing(parser::ParsingFailReasons),
	Lex(lexer::LexError),
}

#[cfg(test)]
mod tests {
	use crate::parser::Nodes;

	#[test]
	fn besl_struct_node_macro_builds_a_struct_node() {
		let mut node = crate::besl_struct_node!(struct Light {
			position: vec3f,
			color: vec3f,
			indices: u32[3],
		});

		match node.node_mut() {
			Nodes::Struct { name, fields } => {
				assert_eq!(*name, "Light");
				assert_eq!(fields.len(), 3);

				match fields[0].node_mut() {
					Nodes::Member { name, r#type } => {
						assert_eq!(*name, "position");
						assert_eq!(r#type, "vec3f");
					}
					_ => panic!("Expected member node."),
				}

				match fields[2].node_mut() {
					Nodes::Member { name, r#type } => {
						assert_eq!(*name, "indices");
						assert_eq!(r#type, "u32[3]");
					}
					_ => panic!("Expected member node."),
				}
			}
			_ => panic!("Expected struct node."),
		}
	}
}
