//! Read MaterialX documents into the shading graph they describe.
//!
//! [MaterialX](https://materialx.org) is an open standard for exchanging node-based material
//! definitions. A `.mtlx` file is XML holding node instances, the connections between them, and the
//! declarations that give those nodes an interface. This module reads that file and hands back the
//! directed acyclic graph, with every name already resolved to an edge.
//!
//! Reading happens in three layers, so a caller can stop after any of them:
//!
//! 1. [`Tree::parse_in`] reads the markup.
//! 2. [`Document::parse`] turns elements into typed MaterialX elements. References are still names,
//!    so this layer never needs anything the file does not contain.
//! 3. [`Dag::resolve`] looks those names up, matches each node instance to its declaration, checks
//!    connected ports agree on their type, and proves the graph is acyclic.
//!
//! [`parse`] runs all three when a caller wants only the graph.
//!
//! Each layer names its own elements, so [`document::Node`] is a node as the file wrote it while
//! [`dag::Node`] is the same node with its references resolved. The names re-exported here are the
//! resolved ones, because those are what a consumer of the graph works with.
//!
//! # Borrowing and storage
//!
//! Nothing here copies the source. Every name and every value is a slice of the text the caller
//! passed in, and every collection comes from the [`Alloc`] the caller supplies, so a whole document
//! and its graph cost one arena that is released in a single step.
//!
//! The graph borrows only from those two, never from the intermediate tree or document, so keep the
//! source and the arena alive for as long as the graph is in use and let everything else go.
//! Diagnostics are the exception: they own their text, so a failure can still be reported after the
//! arena and the source are gone.
//!
//! # Reading a document
//!
//! ```
//! use resource_management::materialx;
//!
//! let source = r#"
//!     <?xml version="1.0"?>
//!     <materialx version="1.39">
//!       <standard_surface name="gold" type="surfaceshader">
//!         <input name="base_color" type="color3" value="0.944, 0.776, 0.373"/>
//!         <input name="metalness" type="float" value="1"/>
//!       </standard_surface>
//!       <surfacematerial name="Mgold" type="material">
//!         <input name="surfaceshader" type="surfaceshader" nodename="gold"/>
//!       </surfacematerial>
//!     </materialx>
//! "#;
//!
//! let arena = bumpalo::Bump::new();
//! // A bump arena is an allocator through its reference, so it is passed by double reference.
//! let allocator = &&arena;
//!
//! let dag = materialx::parse(source, allocator)?;
//!
//! // Materials are the entry points; walk upstream from them to reach the shading network.
//! let material = dag.node(dag.materials()[0]);
//!
//! assert_eq!(material.category, "surfacematerial");
//! assert!(matches!(
//!     material.input("surfaceshader").map(|input| &input.source),
//!     Some(materialx::Source::Node { .. })
//! ));
//! # Ok::<(), materialx::Error>(())
//! ```
//!
//! # Documents split across files
//!
//! MaterialX splits libraries across files with `<xi:include>`. Reading files is a storage concern,
//! so this module reports the references in [`Document::includes`] and leaves fetching them to the
//! caller. Read each referenced document with [`Document::parse`], then fold it in with
//! [`Document::merge`] before resolving. Every document merged this way must borrow from the same
//! allocator, and each one's source text has to outlive the graph.
//! Node declarations that a document never includes simply resolve to no declaration, which leaves
//! the node's written inputs intact but its defaults unknown.
//!
//! # What this module covers
//!
//! Everything that shapes the graph: nodes, nodegraphs both functional and compound, nodedefs and
//! their inheritance, implementations, typedefs, geometric property declarations, units, and
//! variants. Elements that assign materials to geometry, `<look>` and its companions, and elements
//! that only describe user interface layout are recognised and skipped, because neither changes the
//! graph.
//!
//! Next, convert the resolved [`Dag`] into whatever your renderer evaluates.

pub mod dag;
pub mod document;
pub mod error;
pub mod types;
pub mod xml;

use std::fmt::{Display, Formatter};

pub use dag::{Dag, Declaration, DeclarationId, Graph, GraphId, Input, Node, NodeId, Port, PortIndex, Source};
pub use document::{Document, Include};
pub use error::{ParseError, ResolveError, TextPosition, ValueError};
pub use types::{DataType, TypeSemantic, Value, Version};
pub use xml::{Tree, XmlError};

/// The allocator a parsed document and its resolved graph draw every collection from.
///
/// Every structure this module produces borrows its text from the source and its storage from here,
/// so a whole document can live in one arena that is released in a single step. Pass a
/// `bumpalo::Bump` for that, or `&std::alloc::Global` when a document does not warrant one.
pub type Alloc<'a> = &'a dyn std::alloc::Allocator;

/// The MaterialX specification version this parser implements.
pub const SUPPORTED_VERSION: Version = Version::new(1, 39);

/// The oldest MaterialX specification version this parser reads.
///
/// Version 1.38 removed `<parameter>`, `<material>` and `<shaderref>`, so documents older than that
/// describe a different schema. Convert them with the MaterialX tools before baking them.
pub const MINIMUM_VERSION: Version = Version::new(1, 38);

/// The `Error` enum identifies any failure while turning MaterialX source text into a graph.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
	/// The source text is not a well-formed MaterialX document.
	Parse(ParseError),
	/// The document is well-formed but does not describe a valid graph.
	Resolve(ResolveError),
}

impl Display for Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			Error::Parse(error) => write!(f, "{error}"),
			Error::Resolve(error) => write!(f, "{error}"),
		}
	}
}

impl std::error::Error for Error {}

impl From<ParseError> for Error {
	fn from(value: ParseError) -> Self {
		Error::Parse(value)
	}
}

impl From<XmlError> for Error {
	fn from(value: XmlError) -> Self {
		Error::Parse(ParseError::Xml(value))
	}
}

impl From<ResolveError> for Error {
	fn from(value: ResolveError) -> Self {
		Error::Resolve(value)
	}
}

/// Reads MaterialX source text and resolves it into its graph in one step.
///
/// Use this for a document that stands on its own. A document that pulls declarations in with
/// `<xi:include>` needs the references resolved first: call [`Document::parse`], read each entry of
/// [`Document::includes`], fold them in with [`Document::merge`], and then call [`Dag::resolve`].
pub fn parse<'a>(source: &'a str, allocator: Alloc<'a>) -> Result<Dag<'a>, Error> {
	let document = Document::parse(source, allocator)?;

	Ok(Dag::resolve(&document, allocator)?)
}

#[cfg(test)]
mod tests;
