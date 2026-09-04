use std::fmt::{Display, Formatter};

use super::{types::Version, xml::XmlError};
use crate::online_docs_url;

/// The documentation page that explains how Byte-Engine consumes MaterialX documents.
pub(crate) const MATERIALX_DOCS_PATH: &str = "develop/resource-management/materialx";

/// The `TextPosition` struct points at a place in MaterialX source text so parse errors can be traced back to it.
///
/// Positions are produced by [`ParseError`] and are meant to be printed next to the document name.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextPosition {
	/// The one-based line number.
	pub line: u32,
	/// The one-based column number, counted in characters.
	pub column: u32,
}

impl TextPosition {
	/// Converts a byte offset into a one-based line and column.
	///
	/// Offsets past the end of `source` resolve to the end of the text.
	pub fn from_offset(source: &str, offset: usize) -> Self {
		let offset = offset.min(source.len());

		let mut line = 1u32;
		let mut line_start = 0usize;

		for (index, byte) in source.as_bytes()[..offset].iter().enumerate() {
			if *byte == b'\n' {
				line += 1;
				line_start = index + 1;
			}
		}

		// Count characters rather than bytes so columns line up with what an editor shows.
		let column = source[line_start..offset].chars().count() as u32 + 1;

		TextPosition { line, column }
	}
}

impl Display for TextPosition {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}:{}", self.line, self.column)
	}
}

/// The `ValueError` enum identifies text that does not spell a value of the MaterialX type it was written for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueError {
	/// A number could not be read.
	Number { text: String },
	/// A boolean was written as something other than `true` or `false`.
	Boolean { text: String },
	/// A fixed-size type was given the wrong number of components.
	ComponentCount { expected: usize, found: usize },
	/// An array of vectors was given a length that is not a whole number of elements.
	ArrayLength { stride: usize, found: usize },
	/// A struct value was not enclosed in braces.
	Struct { text: String },
}

impl Display for ValueError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			ValueError::Number { text } => write!(f, "'{text}' is not a number"),
			ValueError::Boolean { text } => write!(f, "'{text}' is not 'true' or 'false'"),
			ValueError::ComponentCount { expected, found } => write!(f, "expected {expected} components, found {found}"),
			ValueError::ArrayLength { stride, found } => {
				write!(f, "expected a multiple of {stride} components, found {found}")
			}
			ValueError::Struct { text } => write!(f, "'{text}' is not a brace-enclosed struct value"),
		}
	}
}

/// The `ParseError` enum identifies MaterialX source text that cannot be read into a [`Document`](super::Document).
///
/// Every variant carries the [`TextPosition`] of the offending markup so callers can report it with the document name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
	/// The document is not well-formed XML.
	Xml(XmlError),
	/// The root element is not `<materialx>`.
	Root { name: String, position: TextPosition },
	/// A required attribute is missing.
	MissingAttribute {
		element: String,
		attribute: &'static str,
		position: TextPosition,
	},
	/// The `version` attribute is not written as `major.minor`.
	MalformedVersion { value: String, position: TextPosition },
	/// The document declares a specification version this parser does not read.
	UnsupportedVersion { version: Version, position: TextPosition },
	/// The document uses an element that was removed from the MaterialX specification.
	RemovedElement { name: String, position: TextPosition },
	/// An element appears somewhere the specification does not allow.
	MisplacedElement {
		name: String,
		parent: String,
		position: TextPosition,
	},
	/// A value attribute does not spell a value of its declared type.
	MalformedValue {
		element: String,
		attribute: &'static str,
		error: ValueError,
		position: TextPosition,
	},
	/// An input declares more than one upstream reference.
	ConflictingConnection { element: String, position: TextPosition },
	/// A boolean attribute was written as something other than `true` or `false`.
	MalformedFlag {
		attribute: &'static str,
		value: String,
		position: TextPosition,
	},
}

impl ParseError {
	/// Returns where in the source text this error was found.
	pub fn position(&self) -> TextPosition {
		match self {
			ParseError::Xml(error) => error.position(),
			ParseError::Root { position, .. }
			| ParseError::MissingAttribute { position, .. }
			| ParseError::MalformedVersion { position, .. }
			| ParseError::UnsupportedVersion { position, .. }
			| ParseError::RemovedElement { position, .. }
			| ParseError::MisplacedElement { position, .. }
			| ParseError::MalformedValue { position, .. }
			| ParseError::ConflictingConnection { position, .. }
			| ParseError::MalformedFlag { position, .. } => *position,
		}
	}
}

impl Display for ParseError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			ParseError::Xml(error) => write!(f, "{error}"),
			ParseError::Root { name, position } => write!(
				f,
				"Invalid MaterialX root element '{name}' at {position}. The most likely cause is that the file is not a MaterialX document. See {}.",
				online_docs_url(MATERIALX_DOCS_PATH)
			),
			ParseError::MissingAttribute {
				element,
				attribute,
				position,
			} => write!(
				f,
				"Missing '{attribute}' attribute on <{element}> at {position}. The most likely cause is hand-edited markup that dropped a required attribute."
			),
			ParseError::MalformedVersion { value, position } => write!(
				f,
				"Malformed MaterialX version '{value}' at {position}. The most likely cause is a version written as something other than 'major.minor'."
			),
			ParseError::UnsupportedVersion { version, position } => write!(
				f,
				"Unsupported MaterialX version {version} at {position}. The most likely cause is a document written for a specification older than {} or newer than {}. See {}.",
				super::MINIMUM_VERSION,
				super::SUPPORTED_VERSION,
				online_docs_url(MATERIALX_DOCS_PATH)
			),
			ParseError::RemovedElement { name, position } => write!(
				f,
				"Removed MaterialX element <{name}> at {position}. The most likely cause is a pre-1.38 document that still uses <parameter>, <material>, <shaderref> or <bindinput>."
			),
			ParseError::MisplacedElement { name, parent, position } => write!(
				f,
				"Unexpected <{name}> inside <{parent}> at {position}. The most likely cause is markup nested in an element the specification does not allow it in."
			),
			ParseError::MalformedValue {
				element,
				attribute,
				error,
				position,
			} => write!(
				f,
				"Malformed '{attribute}' on <{element}> at {position}: {error}. The most likely cause is a value that does not match the element's declared type."
			),
			ParseError::ConflictingConnection { element, position } => write!(
				f,
				"Conflicting upstream connection on <{element}> at {position}. The most likely cause is an input carrying more than one of 'nodename', 'nodegraph' and 'interfacename'."
			),
			ParseError::MalformedFlag {
				attribute,
				value,
				position,
			} => write!(
				f,
				"Malformed '{attribute}' flag '{value}' at {position}. The most likely cause is a boolean attribute written as something other than 'true' or 'false'."
			),
		}
	}
}

impl std::error::Error for ParseError {}

impl From<XmlError> for ParseError {
	fn from(value: XmlError) -> Self {
		ParseError::Xml(value)
	}
}

/// The `ResolveError` enum identifies documents that parse but do not describe a valid directed acyclic graph.
///
/// These failures come from [`Dag::resolve`](super::Dag::resolve), after [`Document`](super::Document) reading succeeded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolveError {
	/// Two elements in the same scope share a name.
	DuplicateName { scope: String, name: String },
	/// An input references a node that does not exist in its scope.
	UnknownNode { referrer: String, name: String },
	/// An input references a nodegraph that does not exist.
	UnknownNodeGraph { referrer: String, name: String },
	/// A `nodedef` attribute names a declaration that does not exist.
	UnknownDeclaration { referrer: String, name: String },
	/// An `inherit` attribute names an element that does not exist.
	UnknownInheritance { referrer: String, name: String },
	/// An inheritance chain refers back to itself.
	InheritanceCycle { name: String },
	/// A reference selects an output the upstream element does not have.
	UnknownOutput {
		referrer: String,
		target: String,
		output: String,
	},
	/// A reference targets a multi-output element without selecting an output.
	UnselectedOutput { referrer: String, target: String },
	/// The outputs of a node are unknown because no declaration was found for it.
	UndeclaredOutputs { node: String, category: String },
	/// An `interfacename` is used where no interface is in scope.
	MissingInterface { referrer: String },
	/// An `interfacename` selects an interface input that does not exist.
	UnknownInterfaceInput { referrer: String, name: String },
	/// A connection joins ports of incompatible types.
	TypeMismatch {
		referrer: String,
		expected: String,
		found: String,
	},
	/// The graph contains a cycle, listing the nodes on it in traversal order.
	Cycle { nodes: Vec<String> },
}

impl Display for ResolveError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			ResolveError::DuplicateName { scope, name } => write!(
				f,
				"Duplicate MaterialX name '{name}' in '{scope}'. The most likely cause is two elements in one scope sharing a name, which the specification does not allow."
			),
			ResolveError::UnknownNode { referrer, name } => write!(
				f,
				"Unknown node '{name}' referenced by '{referrer}'. The most likely cause is a 'nodename' pointing outside the node's own graph."
			),
			ResolveError::UnknownNodeGraph { referrer, name } => write!(
				f,
				"Unknown nodegraph '{name}' referenced by '{referrer}'. The most likely cause is a missing <xi:include> for the file that defines it."
			),
			ResolveError::UnknownDeclaration { referrer, name } => write!(
				f,
				"Unknown nodedef '{name}' referenced by '{referrer}'. The most likely cause is a missing <xi:include> for the node library that declares it."
			),
			ResolveError::UnknownInheritance { referrer, name } => write!(
				f,
				"Unknown inherited element '{name}' referenced by '{referrer}'. The most likely cause is an 'inherit' attribute naming an element from an unloaded library."
			),
			ResolveError::InheritanceCycle { name } => write!(
				f,
				"Inheritance cycle at '{name}'. The most likely cause is two elements inheriting from each other."
			),
			ResolveError::UnknownOutput {
				referrer,
				target,
				output,
			} => write!(
				f,
				"Unknown output '{output}' on '{target}' referenced by '{referrer}'. The most likely cause is an 'output' attribute naming a port the upstream element does not declare."
			),
			ResolveError::UnselectedOutput { referrer, target } => write!(
				f,
				"Missing output selection for multi-output '{target}' referenced by '{referrer}'. The most likely cause is an input omitting the 'output' attribute required for multi-output nodes."
			),
			ResolveError::UndeclaredOutputs { node, category } => write!(
				f,
				"Unknown outputs for multi-output node '{node}' of category '{category}'. The most likely cause is a missing <xi:include> for the library that declares the node."
			),
			ResolveError::MissingInterface { referrer } => write!(
				f,
				"Unavailable interface referenced by '{referrer}'. The most likely cause is 'interfacename' used inside a nodegraph that declares neither a nodedef nor its own inputs."
			),
			ResolveError::UnknownInterfaceInput { referrer, name } => write!(
				f,
				"Unknown interface input '{name}' referenced by '{referrer}'. The most likely cause is an 'interfacename' that does not match any input of the enclosing graph's interface."
			),
			ResolveError::TypeMismatch {
				referrer,
				expected,
				found,
			} => write!(
				f,
				"Type mismatch on '{referrer}': expected '{expected}', found '{found}'. The most likely cause is connecting an upstream output to an input of a different MaterialX type."
			),
			ResolveError::Cycle { nodes } => write!(
				f,
				"Cycle in MaterialX graph through {}. The most likely cause is a node network that feeds one of its own upstream nodes, which a directed acyclic graph cannot express.",
				nodes.join(" -> ")
			),
		}
	}
}

impl std::error::Error for ResolveError {}

#[cfg(test)]
mod tests {
	use super::TextPosition;

	#[test]
	fn text_positions_count_lines_and_characters() {
		let source = "<materialx>\n  <añadir/>\n";

		assert_eq!(TextPosition::from_offset(source, 0), TextPosition { line: 1, column: 1 });
		assert_eq!(TextPosition::from_offset(source, 12), TextPosition { line: 2, column: 1 });
		// The accented character occupies two bytes but counts as a single column.
		assert_eq!(TextPosition::from_offset(source, 19), TextPosition { line: 2, column: 7 });
	}
}
