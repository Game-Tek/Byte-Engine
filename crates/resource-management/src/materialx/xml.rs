//! Read the XML subset that MaterialX documents are written in.
//!
//! MaterialX files use a deliberately small slice of XML: one root element, nested elements with
//! quoted attributes, comments, and the five predefined character entities. This reader accepts that
//! slice and rejects everything else, so a malformed or hostile `.mtlx` file fails with a position
//! instead of being partly understood.
//!
//! Next, hand the [`Tree`] to [`Document::read`](super::Document::read) to turn elements into typed
//! MaterialX elements.

use std::{
	alloc::Global,
	fmt::{Display, Formatter},
};

use super::{Alloc, error::TextPosition};

/// The deepest element nesting the reader accepts, which keeps hostile documents from exhausting memory.
const DEPTH_LIMIT: usize = 256;

/// The most attributes one element may carry, which bounds the cost of the duplicate-name check.
const ATTRIBUTE_LIMIT: usize = 64;

/// The most elements one document may contain, which keeps element indices inside `u32`.
const ELEMENT_LIMIT: usize = 1 << 22;

/// The `XmlError` enum identifies text that is not the XML subset MaterialX documents are written in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XmlError {
	/// The reader needed a specific piece of markup and found something else.
	Unexpected { expected: &'static str, position: TextPosition },
	/// An element or attribute name does not start with a letter or underscore.
	InvalidName { position: TextPosition },
	/// An element was opened and never closed.
	UnclosedElement { name: String, position: TextPosition },
	/// An end tag closes a different element than the one that is open.
	MismatchedEndTag {
		expected: String,
		found: String,
		position: TextPosition,
	},
	/// An end tag appears where no element is open.
	UnexpectedEndTag { name: String, position: TextPosition },
	/// The document contains no element at all.
	MissingRootElement,
	/// The document contains more than one top-level element.
	MultipleRootElements { position: TextPosition },
	/// The document declares a DTD, which this reader refuses so entity expansion cannot be abused.
	DoctypeNotSupported { position: TextPosition },
	/// An attribute value references an entity that XML does not predefine.
	UnknownEntity { entity: String, position: TextPosition },
	/// One element carries the same attribute twice.
	DuplicateAttribute { name: String, position: TextPosition },
	/// The document nests elements deeper than this reader accepts.
	DepthLimitExceeded { position: TextPosition },
	/// The document carries more elements or attributes than the reader accepts.
	SizeLimitExceeded { position: TextPosition },
}

impl XmlError {
	/// Returns where in the source text this error was found.
	pub fn position(&self) -> TextPosition {
		match self {
			XmlError::Unexpected { position, .. }
			| XmlError::InvalidName { position }
			| XmlError::UnclosedElement { position, .. }
			| XmlError::MismatchedEndTag { position, .. }
			| XmlError::UnexpectedEndTag { position, .. }
			| XmlError::MultipleRootElements { position }
			| XmlError::DoctypeNotSupported { position }
			| XmlError::UnknownEntity { position, .. }
			| XmlError::DuplicateAttribute { position, .. }
			| XmlError::DepthLimitExceeded { position }
			| XmlError::SizeLimitExceeded { position } => *position,
			XmlError::MissingRootElement => TextPosition { line: 1, column: 1 },
		}
	}
}

impl Display for XmlError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			XmlError::Unexpected { expected, position } => write!(
				f,
				"Malformed XML at {position}: expected {expected}. The most likely cause is hand-edited markup with an unbalanced tag or a missing quote."
			),
			XmlError::InvalidName { position } => write!(
				f,
				"Invalid XML name at {position}. The most likely cause is a tag or attribute whose name starts with a digit or punctuation."
			),
			XmlError::UnclosedElement { name, position } => write!(
				f,
				"Unclosed XML element <{name}> opened at {position}. The most likely cause is a missing end tag."
			),
			XmlError::MismatchedEndTag {
				expected,
				found,
				position,
			} => write!(
				f,
				"Mismatched XML end tag </{found}> at {position}, expected </{expected}>. The most likely cause is overlapping tags."
			),
			XmlError::UnexpectedEndTag { name, position } => write!(
				f,
				"Unexpected XML end tag </{name}> at {position}. The most likely cause is an end tag without a matching start tag."
			),
			XmlError::MissingRootElement => write!(
				f,
				"Missing XML root element. The most likely cause is an empty file or one that holds only comments."
			),
			XmlError::MultipleRootElements { position } => write!(
				f,
				"Multiple XML root elements, the second at {position}. The most likely cause is concatenating two documents into one file."
			),
			XmlError::DoctypeNotSupported { position } => write!(
				f,
				"Unsupported XML document type declaration at {position}. The most likely cause is a document carrying a DTD, which MaterialX documents do not use."
			),
			XmlError::UnknownEntity { entity, position } => write!(
				f,
				"Unknown XML entity '&{entity};' at {position}. The most likely cause is a bare '&' that should have been written as '&amp;'."
			),
			XmlError::DuplicateAttribute { name, position } => write!(
				f,
				"Duplicate XML attribute '{name}' at {position}. The most likely cause is a copied attribute that was never edited."
			),
			XmlError::DepthLimitExceeded { position } => write!(
				f,
				"XML nesting deeper than {DEPTH_LIMIT} at {position}. The most likely cause is a generated document with runaway nesting."
			),
			XmlError::SizeLimitExceeded { position } => write!(
				f,
				"XML document larger than this reader accepts at {position}. The most likely cause is a generated document with more than {ELEMENT_LIMIT} elements."
			),
		}
	}
}

impl std::error::Error for XmlError {}

/// One name and value pair read from an element's start tag.
#[derive(Clone, Copy, Debug)]
struct Attribute<'a> {
	name: &'a str,
	value: &'a str,
	/// The byte offset of the attribute name in the source text.
	offset: usize,
}

/// One element in the flat arena a [`Tree`] stores.
#[derive(Clone, Debug)]
struct Element<'a> {
	name: &'a str,
	offset: usize,
	attributes_start: u32,
	attributes_len: u32,
	children_start: u32,
	children_len: u32,
}

/// The `Tree` struct holds one parsed XML document as a flat arena that borrows its text from the source.
///
/// Walk it from [`Tree::root`], then read attributes with [`ElementRef::attribute`].
#[derive(Clone, Debug)]
pub struct Tree<'a> {
	source: &'a str,
	elements: Vec<Element<'a>, Alloc<'a>>,
	attributes: Vec<Attribute<'a>, Alloc<'a>>,
	children: Vec<u32, Alloc<'a>>,
	root: u32,
}

impl<'a> Tree<'a> {
	/// Reads one XML document from source text, using the global allocator.
	///
	/// Prefer [`Tree::parse_in`] when the document's storage should share an arena with everything
	/// read from it.
	pub fn parse(source: &'a str) -> Result<Self, XmlError> {
		Self::parse_in(source, &Global)
	}

	/// Reads one XML document from source text, drawing its storage from `allocator`.
	///
	/// Every name and value the tree hands out borrows from `source` or from `allocator`, never from
	/// the tree itself, so the tree may be dropped as soon as it has been read.
	///
	/// Next, pass the tree to [`Document::read`](super::Document::read) with the same allocator.
	pub fn parse_in(source: &'a str, allocator: Alloc<'a>) -> Result<Self, XmlError> {
		Parser::new(source, allocator).run()
	}

	/// Returns the document's single root element.
	pub fn root(&self) -> ElementRef<'_, 'a> {
		ElementRef {
			tree: self,
			index: self.root,
		}
	}
}

/// The `ElementRef` struct borrows one element out of a [`Tree`] together with the tree it belongs to.
#[derive(Clone, Copy, Debug)]
pub struct ElementRef<'t, 'a> {
	tree: &'t Tree<'a>,
	index: u32,
}

impl<'t, 'a> ElementRef<'t, 'a> {
	fn element(&self) -> &'t Element<'a> {
		&self.tree.elements[self.index as usize]
	}

	/// Returns the element name exactly as written, including any namespace prefix.
	pub fn name(&self) -> &'a str {
		self.element().name
	}

	/// Returns the byte offset of the element's start tag in the source text.
	pub fn offset(&self) -> usize {
		self.element().offset
	}

	fn attributes(&self) -> &'t [Attribute<'a>] {
		let element = self.element();
		let start = element.attributes_start as usize;

		&self.tree.attributes[start..start + element.attributes_len as usize]
	}

	/// Returns the value of one attribute, or `None` when the element does not carry it.
	pub fn attribute(&self, name: &str) -> Option<&'a str> {
		self.attributes()
			.iter()
			.find(|attribute| attribute.name == name)
			.map(|attribute| attribute.value)
	}

	/// Returns the byte offset of one attribute, falling back to the element's own offset.
	pub fn attribute_offset(&self, name: &str) -> usize {
		self.attributes()
			.iter()
			.find(|attribute| attribute.name == name)
			.map_or_else(|| self.offset(), |attribute| attribute.offset)
	}

	/// Returns where this element's start tag begins, for error reporting.
	pub fn position(&self) -> TextPosition {
		TextPosition::from_offset(self.tree.source, self.offset())
	}

	/// Returns where one attribute begins, falling back to the element's own position.
	pub fn attribute_position(&self, name: &str) -> TextPosition {
		TextPosition::from_offset(self.tree.source, self.attribute_offset(name))
	}

	/// Returns this element's child elements, in source order.
	pub fn children(&self) -> impl Iterator<Item = ElementRef<'t, 'a>> + '_ {
		let element = self.element();
		let start = element.children_start as usize;
		let end = start + element.children_len as usize;

		self.tree.children[start..end].iter().map(|index| ElementRef {
			tree: self.tree,
			index: *index,
		})
	}
}

/// Tracks one element that has been opened but not yet closed.
struct Open<'a> {
	index: u32,
	name: &'a str,
	offset: usize,
	scratch_start: usize,
}

/// Reads source text into a [`Tree`] with a single pass and no backtracking.
struct Parser<'a> {
	source: &'a str,
	bytes: &'a [u8],
	offset: usize,
	allocator: Alloc<'a>,
	elements: Vec<Element<'a>, Alloc<'a>>,
	attributes: Vec<Attribute<'a>, Alloc<'a>>,
	children: Vec<u32, Alloc<'a>>,
	/// Child indices of the currently open elements, appended in order and drained when each closes.
	scratch: Vec<u32, Alloc<'a>>,
	stack: Vec<Open<'a>, Alloc<'a>>,
	root: Option<u32>,
}

impl<'a> Parser<'a> {
	fn new(source: &'a str, allocator: Alloc<'a>) -> Self {
		// A UTF-8 byte order mark is legal at the start of an XML document but is not markup.
		let offset = usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();

		Parser {
			source,
			bytes: source.as_bytes(),
			offset,
			allocator,
			elements: Vec::new_in(allocator),
			attributes: Vec::new_in(allocator),
			children: Vec::new_in(allocator),
			scratch: Vec::new_in(allocator),
			stack: Vec::new_in(allocator),
			root: None,
		}
	}

	fn position(&self, offset: usize) -> TextPosition {
		TextPosition::from_offset(self.source, offset)
	}

	fn unexpected(&self, expected: &'static str, offset: usize) -> XmlError {
		XmlError::Unexpected {
			expected,
			position: self.position(offset),
		}
	}

	/// Reads every top-level construct until the source is consumed.
	fn run(mut self) -> Result<Tree<'a>, XmlError> {
		loop {
			self.skip_text();

			if self.offset >= self.bytes.len() {
				break;
			}

			// `skip_text` stops on '<', so the next byte always opens some construct.
			match self.bytes.get(self.offset + 1) {
				Some(b'?') => self.skip_processing_instruction()?,
				Some(b'!') => self.skip_declaration()?,
				Some(b'/') => self.read_end_tag()?,
				_ => self.read_start_tag()?,
			}
		}

		if let Some(open) = self.stack.last() {
			return Err(XmlError::UnclosedElement {
				name: open.name.to_string(),
				position: self.position(open.offset),
			});
		}

		let root = self.root.ok_or(XmlError::MissingRootElement)?;

		Ok(Tree {
			source: self.source,
			elements: self.elements,
			attributes: self.attributes,
			children: self.children,
			root,
		})
	}

	/// Advances to the next '<', discarding character data, which MaterialX documents never rely on.
	fn skip_text(&mut self) {
		match self.source[self.offset..].find('<') {
			Some(index) => self.offset += index,
			None => self.offset = self.bytes.len(),
		}
	}

	fn skip_whitespace(&mut self) {
		while matches!(self.bytes.get(self.offset), Some(b' ' | b'\t' | b'\r' | b'\n')) {
			self.offset += 1;
		}
	}

	/// Skips `<?xml ... ?>` and any other processing instruction.
	fn skip_processing_instruction(&mut self) -> Result<(), XmlError> {
		let start = self.offset;

		match self.source[self.offset..].find("?>") {
			Some(index) => {
				self.offset += index + "?>".len();
				Ok(())
			}
			None => Err(self.unexpected("'?>'", start)),
		}
	}

	/// Skips comments and CDATA sections, and refuses document type declarations.
	fn skip_declaration(&mut self) -> Result<(), XmlError> {
		let start = self.offset;
		let rest = &self.source[self.offset..];

		if let Some(body) = rest.strip_prefix("<!--") {
			let end = body.find("-->").ok_or_else(|| self.unexpected("'-->'", start))?;
			self.offset += "<!--".len() + end + "-->".len();

			return Ok(());
		}

		if let Some(body) = rest.strip_prefix("<![CDATA[") {
			let end = body.find("]]>").ok_or_else(|| self.unexpected("']]>'", start))?;
			self.offset += "<![CDATA[".len() + end + "]]>".len();

			return Ok(());
		}

		// A DTD can define entities, so refusing it removes entity expansion as an attack surface.
		Err(XmlError::DoctypeNotSupported {
			position: self.position(start),
		})
	}

	fn read_start_tag(&mut self) -> Result<(), XmlError> {
		let start = self.offset;

		self.offset += 1;

		let name = self.read_name()?;

		let attributes_start = self.attributes.len();

		loop {
			self.skip_whitespace();

			match self.bytes.get(self.offset) {
				Some(b'>') => {
					self.offset += 1;

					break;
				}
				Some(b'/') => {
					if self.bytes.get(self.offset + 1) != Some(&b'>') {
						return Err(self.unexpected("'/>'", self.offset));
					}

					self.offset += 2;

					let index = self.push_element(name, start, attributes_start)?;

					return self.attach(index, start);
				}
				Some(_) => self.read_attribute(attributes_start)?,
				None => return Err(self.unexpected("'>'", self.offset)),
			}
		}

		if self.stack.len() >= DEPTH_LIMIT {
			return Err(XmlError::DepthLimitExceeded {
				position: self.position(start),
			});
		}

		let index = self.push_element(name, start, attributes_start)?;

		self.stack.push(Open {
			index,
			name,
			offset: start,
			scratch_start: self.scratch.len(),
		});

		Ok(())
	}

	fn read_end_tag(&mut self) -> Result<(), XmlError> {
		let start = self.offset;

		self.offset += 2;

		let name = self.read_name()?;

		self.skip_whitespace();

		if self.bytes.get(self.offset) != Some(&b'>') {
			return Err(self.unexpected("'>'", self.offset));
		}

		self.offset += 1;

		let open = self.stack.pop().ok_or_else(|| XmlError::UnexpectedEndTag {
			name: name.to_string(),
			position: self.position(start),
		})?;

		if open.name != name {
			return Err(XmlError::MismatchedEndTag {
				expected: open.name.to_string(),
				found: name.to_string(),
				position: self.position(start),
			});
		}

		// The children collected since this element opened are contiguous at the tail of the scratch buffer.
		let children_start = self.children.len() as u32;

		self.children.extend_from_slice(&self.scratch[open.scratch_start..]);

		let children_len = self.children.len() as u32 - children_start;

		self.scratch.truncate(open.scratch_start);

		let element = &mut self.elements[open.index as usize];

		element.children_start = children_start;
		element.children_len = children_len;

		self.attach(open.index, open.offset)
	}

	/// Records a finished element as a child of the element that encloses it, or as the document root.
	fn attach(&mut self, index: u32, offset: usize) -> Result<(), XmlError> {
		if self.stack.is_empty() {
			if self.root.is_some() {
				return Err(XmlError::MultipleRootElements {
					position: self.position(offset),
				});
			}

			self.root = Some(index);
		} else {
			self.scratch.push(index);
		}

		Ok(())
	}

	/// Records a finished start tag; its children are attached when its end tag is read.
	fn push_element(&mut self, name: &'a str, offset: usize, attributes_start: usize) -> Result<u32, XmlError> {
		if self.elements.len() >= ELEMENT_LIMIT {
			return Err(XmlError::SizeLimitExceeded {
				position: self.position(offset),
			});
		}

		let index = self.elements.len() as u32;

		self.elements.push(Element {
			name,
			offset,
			attributes_start: attributes_start as u32,
			attributes_len: (self.attributes.len() - attributes_start) as u32,
			children_start: 0,
			children_len: 0,
		});

		Ok(index)
	}

	fn read_attribute(&mut self, attributes_start: usize) -> Result<(), XmlError> {
		let offset = self.offset;

		let name = self.read_name()?;

		self.skip_whitespace();

		if self.bytes.get(self.offset) != Some(&b'=') {
			return Err(self.unexpected("'='", self.offset));
		}

		self.offset += 1;

		self.skip_whitespace();

		let quote = match self.bytes.get(self.offset) {
			Some(quote @ (b'"' | b'\'')) => *quote,
			_ => return Err(self.unexpected("a quoted attribute value", self.offset)),
		};

		self.offset += 1;

		let value_start = self.offset;
		let value_end = self.source[value_start..]
			.find(quote as char)
			.map(|index| value_start + index)
			.ok_or_else(|| self.unexpected("a closing quote", value_start))?;

		self.offset = value_end + 1;

		let value = self.decode(&self.source[value_start..value_end], value_start)?;

		// The attribute count is capped, so this scan stays bounded even for hostile input.
		if self.attributes.len() - attributes_start >= ATTRIBUTE_LIMIT {
			return Err(XmlError::SizeLimitExceeded {
				position: self.position(offset),
			});
		}

		if self.attributes[attributes_start..]
			.iter()
			.any(|attribute| attribute.name == name)
		{
			return Err(XmlError::DuplicateAttribute {
				name: name.to_string(),
				position: self.position(offset),
			});
		}

		self.attributes.push(Attribute { name, value, offset });

		Ok(())
	}

	fn read_name(&mut self) -> Result<&'a str, XmlError> {
		let start = self.offset;

		match self.bytes.get(self.offset) {
			Some(byte) if is_name_start(*byte) => self.offset += 1,
			_ => {
				return Err(XmlError::InvalidName {
					position: self.position(start),
				});
			}
		}

		while self.bytes.get(self.offset).is_some_and(|byte| is_name_char(*byte)) {
			self.offset += 1;
		}

		Ok(&self.source[start..self.offset])
	}

	/// Replaces the five predefined entities and numeric character references.
	///
	/// A value with no entity in it, which is nearly all of them, is borrowed straight from the
	/// source. One that does carry an entity is decoded into the allocator and left there, so the
	/// text outlives this tree exactly as a borrowed one does. With an arena that memory returns
	/// when the arena does; with the global allocator it is retained.
	fn decode(&self, raw: &'a str, offset: usize) -> Result<&'a str, XmlError> {
		if !raw.contains('&') {
			return Ok(raw);
		}

		let mut decoded = Vec::with_capacity_in(raw.len(), self.allocator);
		let mut character = [0u8; 4];
		let mut rest = raw;

		while let Some(index) = rest.find('&') {
			decoded.extend_from_slice(&rest.as_bytes()[..index]);

			let entity_offset = offset + (raw.len() - rest.len()) + index;
			let tail = &rest[index + 1..];

			let end = tail.find(';').ok_or_else(|| XmlError::UnknownEntity {
				entity: tail.chars().take(16).collect(),
				position: self.position(entity_offset),
			})?;

			let entity = &tail[..end];

			let unknown = || XmlError::UnknownEntity {
				entity: entity.to_string(),
				position: self.position(entity_offset),
			};

			let replacement = match entity {
				"amp" => '&',
				"lt" => '<',
				"gt" => '>',
				"quot" => '"',
				"apos" => '\'',
				_ => {
					let code = match entity.strip_prefix('#') {
						Some(digits) => match digits.strip_prefix(['x', 'X']) {
							Some(hex) => u32::from_str_radix(hex, 16).map_err(|_| unknown())?,
							None => digits.parse::<u32>().map_err(|_| unknown())?,
						},
						None => return Err(unknown()),
					};

					char::from_u32(code).ok_or_else(unknown)?
				}
			};

			decoded.extend_from_slice(replacement.encode_utf8(&mut character).as_bytes());

			rest = &tail[end + 1..];
		}

		decoded.extend_from_slice(rest.as_bytes());

		// Only source text and encoded characters are ever pushed, so this cannot fail.
		let decoded = std::str::from_utf8(decoded.leak()).expect("Decoded attribute text should be valid UTF-8");

		Ok(decoded)
	}
}

fn is_name_start(byte: u8) -> bool {
	// Bytes above ASCII are accepted so element names written in UTF-8 survive; MaterialX itself uses ASCII.
	byte.is_ascii_alphabetic() || byte == b'_' || byte == b':' || byte >= 0x80
}

fn is_name_char(byte: u8) -> bool {
	is_name_start(byte) || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
}

#[cfg(test)]
mod tests {
	use super::{Tree, XmlError};

	#[test]
	fn reads_nested_elements_and_attributes() {
		let source = r#"<?xml version="1.0"?>
			<materialx version="1.39">
				<!-- a comment -->
				<nodegraph name="NG">
					<image name="i" type='color3'/>
				</nodegraph>
			</materialx>"#;

		let tree = Tree::parse(source).expect("The document should parse");
		let root = tree.root();

		assert_eq!(root.name(), "materialx");
		assert_eq!(root.attribute("version"), Some("1.39"));

		let graph = root.children().next().expect("The root should have one child");

		assert_eq!(graph.name(), "nodegraph");
		assert_eq!(graph.attribute("name"), Some("NG"));

		let image = graph.children().next().expect("The graph should have one child");

		assert_eq!(image.name(), "image");
		assert_eq!(image.attribute("type"), Some("color3"));
		assert_eq!(image.children().count(), 0);
	}

	#[test]
	fn decodes_predefined_and_numeric_entities() {
		let source = r#"<materialx doc="&quot;a&amp;b&quot; &lt;c&gt; &#65;&#x42;"/>"#;

		let tree = Tree::parse(source).expect("The document should parse");

		assert_eq!(tree.root().attribute("doc"), Some(r#""a&b" <c> AB"#));
	}

	#[test]
	fn keeps_prefixed_names_without_a_namespace_declaration() {
		// MaterialX files routinely write <xi:include> without declaring the prefix.
		let source = r#"<materialx version="1.39"><xi:include href="lib.mtlx"/></materialx>"#;

		let tree = Tree::parse(source).expect("The document should parse");
		let include = tree.root().children().next().expect("The include should be read");

		assert_eq!(include.name(), "xi:include");
		assert_eq!(include.attribute("href"), Some("lib.mtlx"));
	}

	#[test]
	fn rejects_document_type_declarations() {
		let source = r#"<!DOCTYPE materialx [<!ENTITY x "y">]><materialx version="1.39"/>"#;

		assert!(matches!(Tree::parse(source), Err(XmlError::DoctypeNotSupported { .. })));
	}

	#[test]
	fn rejects_mismatched_end_tags() {
		let source = r#"<materialx version="1.39"><nodegraph name="NG"></materialx>"#;

		assert!(matches!(Tree::parse(source), Err(XmlError::MismatchedEndTag { .. })));
	}

	#[test]
	fn rejects_unknown_entities() {
		let source = r#"<materialx doc="a &nbsp; b"/>"#;

		assert!(matches!(Tree::parse(source), Err(XmlError::UnknownEntity { .. })));
	}

	#[test]
	fn rejects_duplicate_attributes() {
		let source = r#"<materialx version="1.39" version="1.38"/>"#;

		assert!(matches!(Tree::parse(source), Err(XmlError::DuplicateAttribute { .. })));
	}

	#[test]
	fn rejects_documents_without_a_root_element() {
		assert!(matches!(
			Tree::parse("<!-- only a comment -->"),
			Err(XmlError::MissingRootElement)
		));
	}

	#[test]
	fn rejects_nesting_past_the_depth_limit() {
		let mut source = String::new();

		for _ in 0..(super::DEPTH_LIMIT + 2) {
			source.push_str("<a>");
		}

		assert!(matches!(Tree::parse(&source), Err(XmlError::DepthLimitExceeded { .. })));
	}
}
