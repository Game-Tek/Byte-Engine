//! Read a MaterialX document into typed elements, without resolving any reference.
//!
//! This layer is purely syntactic: it turns markup into the elements the specification defines and
//! reports where malformed markup is, but it never looks a `nodename` up or checks a type. Feed the
//! result to [`Dag::resolve`](super::Dag::resolve) to get the connected graph.
//!
//! Every element borrows its text from the [`Tree`] it was read from and takes its storage from the
//! allocator the caller supplies, so a document costs one arena and no copies of the source.

use super::{
	Alloc, MINIMUM_VERSION, SUPPORTED_VERSION,
	error::ParseError,
	types::{DataType, TypeSemantic, Value, Version},
	xml::{ElementRef, Tree},
};

/// Elements the specification gives a meaning of their own, which therefore never name a node category.
///
/// Every other element inside a document or nodegraph is an instance of the node whose category is
/// the element name, which is how MaterialX spells node instantiation.
const RESERVED_ELEMENTS: &[&str] = &[
	"materialx",
	"nodedef",
	"nodegraph",
	"implementation",
	"typedef",
	"member",
	"unittypedef",
	"unitdef",
	"unit",
	"geompropdef",
	"attributedef",
	"targetdef",
	"variantset",
	"variant",
	"look",
	"lookgroup",
	"collection",
	"geominfo",
	"geomprop",
	"geomtoken",
	"tokendefault",
	"propertyset",
	"property",
	"propertyassign",
	"materialassign",
	"variantassign",
	"visibility",
	"backdrop",
	"uifolder",
	"input",
	"output",
	"token",
];

/// Elements that older MaterialX versions defined and that version 1.38 removed.
///
/// Naming them explicitly turns a stale document into a clear error instead of a node of an
/// unknown category.
const REMOVED_ELEMENTS: &[&str] = &[
	"parameter",
	"shaderref",
	"bindinput",
	"bindparam",
	"bindtoken",
	"bindgeomprop",
	"override",
	"materialinherit",
	"geomattr",
	"geomattrvalue",
];

/// Elements this parser knows about but does not model, because they describe geometry assignment
/// and user interface layout rather than the shading graph.
const IGNORED_ELEMENTS: &[&str] = &[
	"attributedef",
	"targetdef",
	"look",
	"lookgroup",
	"collection",
	"geominfo",
	"propertyset",
	"property",
	"backdrop",
	"uifolder",
];

/// The `Named` trait identifies the MaterialX elements the specification addresses by name.
///
/// Names are unique within a scope, which is what [`merge_by_name`] and reference resolution key on.
pub(crate) trait Named {
	fn name(&self) -> &str;

	/// Returns the namespace qualifying this element's name, for the elements that carry one.
	fn scope(&self) -> Option<&str> {
		None
	}
}

impl<T: Named> Named for &T {
	fn name(&self) -> &str {
		(*self).name()
	}

	fn scope(&self) -> Option<&str> {
		(*self).scope()
	}
}

macro_rules! named {
	($($element:ty),* $(,)?) => {
		$(
			impl Named for $element {
				fn name(&self) -> &str {
					self.name
				}
			}
		)*
	};
}

named!(
	Input<'_>,
	Output<'_>,
	Node<'_>,
	Implementation<'_>,
	TypeDef<'_>,
	GeomPropDef<'_>,
	UnitTypeDef<'_>,
	UnitDef<'_>,
	VariantSet<'_>,
);

impl Named for NodeDef<'_> {
	fn name(&self) -> &str {
		self.name
	}

	fn scope(&self) -> Option<&str> {
		self.namespace
	}
}

impl Named for NodeGraph<'_> {
	fn name(&self) -> &str {
		self.name
	}

	fn scope(&self) -> Option<&str> {
		self.namespace
	}
}

/// Appends elements, keeping any definition already present under the same name and namespace.
pub(crate) fn merge_by_name<T: Named>(target: &mut Vec<T, Alloc<'_>>, incoming: impl IntoIterator<Item = T>) {
	for element in incoming {
		if !target
			.iter()
			.any(|existing| existing.scope() == element.scope() && existing.name() == element.name())
		{
			target.push(element);
		}
	}
}

/// The `Include` struct records one `<xi:include>` the document asks for.
///
/// Resolving the reference is the caller's job, because reading files is a storage concern. Read the
/// referenced document, then fold it in with [`Document::merge`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Include<'a> {
	/// The referenced document, as written; it may be relative to the including document.
	pub href: &'a str,
}

/// The `Connection` enum records the upstream reference an input or output was authored with.
///
/// References are still names at this stage. [`Dag::resolve`](super::Dag::resolve) turns them into
/// [`Source`](super::dag::Source) edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Connection<'a> {
	/// `nodename`: the output of another node in the same scope.
	Node { node: &'a str, output: Option<&'a str> },
	/// `nodegraph`: an output of a nodegraph.
	NodeGraph { node_graph: &'a str, output: Option<&'a str> },
	/// `interfacename`: an input of the enclosing graph's interface.
	Interface { input: &'a str },
}

/// The `Input` struct records one authored input port on a node, nodegraph, nodedef, or variant.
#[derive(Clone, Debug, PartialEq)]
pub struct Input<'a> {
	pub name: &'a str,
	pub data_type: DataType<'a>,
	/// The authored constant, which on a nodedef input is the default for instances that leave it unset.
	pub value: Option<Value<'a>>,
	pub connection: Option<Connection<'a>>,
	/// The channel selection of pre-1.39 documents; 1.39 replaces it with explicit channel nodes.
	pub channels: Option<&'a str>,
	pub colorspace: Option<&'a str>,
	pub file_prefix: Option<&'a str>,
	pub unit: Option<&'a str>,
	pub unit_type: Option<&'a str>,
	/// Whether the input only accepts values that do not vary across a surface.
	pub uniform: bool,
	/// The geometric property that supplies this input when nothing else does.
	pub default_geom_prop: Option<&'a str>,
	/// The rendering target this input is meant for, when it is target specific.
	pub target: Option<&'a str>,
}

/// The `Output` struct records one authored output port on a nodegraph, nodedef, or node instance.
#[derive(Clone, Debug, PartialEq)]
pub struct Output<'a> {
	pub name: &'a str,
	pub data_type: DataType<'a>,
	/// The node or nodegraph whose result this output carries; nodedef outputs never declare one.
	pub connection: Option<Connection<'a>>,
	/// The input a node passes through when the reader has no implementation for it.
	pub default_input: Option<&'a str>,
	/// The constant a node produces when the reader has no implementation for it.
	pub default_value: Option<Value<'a>>,
	pub colorspace: Option<&'a str>,
	pub uniform: bool,
}

/// The `Node` struct records one authored node instance, which is the unit a MaterialX graph is built from.
#[derive(Clone, Debug, PartialEq)]
pub struct Node<'a> {
	/// The node category, which is the element name, such as `image`, `multiply` or `standard_surface`.
	pub category: &'a str,
	pub name: &'a str,
	/// The type this instance produces, or [`DataType::MultiOutput`] when its declaration has several outputs.
	pub data_type: DataType<'a>,
	/// The exact declaration this instance asks for, when it does not rely on signature matching.
	pub node_def: Option<&'a str>,
	/// The declaration version this instance asks for.
	pub version: Option<&'a str>,
	/// Another node instance of the same category whose input values this one starts from.
	pub inherit: Option<&'a str>,
	pub colorspace: Option<&'a str>,
	pub file_prefix: Option<&'a str>,
	pub inputs: Vec<Input<'a>, Alloc<'a>>,
	/// The `<token>` ports, whose values are substituted into filenames inside the node's implementation.
	pub tokens: Vec<Input<'a>, Alloc<'a>>,
	/// Outputs written on the instance itself, which let a multi-output node be read without its declaration.
	pub outputs: Vec<Output<'a>, Alloc<'a>>,
}

/// The `NodeGraph` struct records one `<nodegraph>`: a named scope holding nodes and the outputs they feed.
///
/// A graph that names a `node_def` implements that declaration and takes its interface from it; a
/// graph that declares its own `inputs` is a compound graph that wraps the nodes it contains.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeGraph<'a> {
	pub name: &'a str,
	pub node_def: Option<&'a str>,
	pub namespace: Option<&'a str>,
	pub colorspace: Option<&'a str>,
	pub file_prefix: Option<&'a str>,
	pub inputs: Vec<Input<'a>, Alloc<'a>>,
	/// The `<token>` ports, whose values are substituted into filenames inside the graph.
	pub tokens: Vec<Input<'a>, Alloc<'a>>,
	pub nodes: Vec<Node<'a>, Alloc<'a>>,
	pub node_graphs: Vec<NodeGraph<'a>, Alloc<'a>>,
	pub outputs: Vec<Output<'a>, Alloc<'a>>,
}

/// The `NodeDef` struct records one `<nodedef>`: the interface a node category presents to the graphs that use it.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDef<'a> {
	pub name: &'a str,
	/// The node category this declaration defines, empty when the document left it out.
	///
	/// A declaration with no category matches no node instance, which is how a document that
	/// repurposes `<nodedef>` as a property bag still reads without failing.
	pub node: &'a str,
	/// Another declaration whose ports this one starts from.
	pub inherit: Option<&'a str>,
	pub node_group: Option<&'a str>,
	pub version: Option<&'a str>,
	/// Whether instances that ask for no version get this declaration.
	pub is_default_version: bool,
	pub targets: Vec<&'a str, Alloc<'a>>,
	pub namespace: Option<&'a str>,
	/// The geometric properties the node reads internally, which code generators must make available.
	pub internal_geom_props: Vec<&'a str, Alloc<'a>>,
	pub inputs: Vec<Input<'a>, Alloc<'a>>,
	/// The `<token>` ports this declaration accepts, which its implementation substitutes into filenames.
	pub tokens: Vec<Input<'a>, Alloc<'a>>,
	pub outputs: Vec<Output<'a>, Alloc<'a>>,
}

/// The `Implementation` struct records where the source code for a declaration lives.
///
/// Byte-Engine does not use these bodies, but keeping them lets a caller tell a nodegraph-backed
/// declaration apart from one that only exists as native code for some other renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct Implementation<'a> {
	pub name: &'a str,
	pub node_def: &'a str,
	pub node_graph: Option<&'a str>,
	pub impl_name: Option<&'a str>,
	pub file: Option<&'a str>,
	pub source_code: Option<&'a str>,
	pub function: Option<&'a str>,
	pub targets: Vec<&'a str, Alloc<'a>>,
	pub format: Option<&'a str>,
}

/// The `TypeMember` struct records one field of a custom struct type.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeMember<'a> {
	pub name: &'a str,
	pub data_type: DataType<'a>,
	pub value: Option<Value<'a>>,
}

/// The `TypeDef` struct records one `<typedef>`, which introduces a data type beyond the standard set.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeDef<'a> {
	pub name: &'a str,
	pub semantic: TypeSemantic<'a>,
	/// The rendering context a shader-semantic type is evaluated in, such as `surface`.
	pub context: Option<&'a str>,
	pub inherit: Option<&'a str>,
	pub hint: Option<&'a str>,
	pub members: Vec<TypeMember<'a>, Alloc<'a>>,
}

/// The `GeomPropDef` struct records one `<geompropdef>`: a named property read off the geometry being shaded.
#[derive(Clone, Debug, PartialEq)]
pub struct GeomPropDef<'a> {
	pub name: &'a str,
	pub data_type: DataType<'a>,
	/// The standard property this one is derived from, such as `texcoord`.
	pub geom_prop: Option<&'a str>,
	/// The coordinate space the property is reported in: `model`, `object` or `world`.
	pub space: Option<&'a str>,
	pub index: Option<i32>,
	pub uniform: bool,
	pub unit: Option<&'a str>,
	pub unit_type: Option<&'a str>,
}

/// The `UnitTypeDef` struct records one `<unittypedef>`, which names a family of units such as `distance`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnitTypeDef<'a> {
	pub name: &'a str,
}

/// The `Unit` struct records one unit and how it scales relative to the others of its type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unit<'a> {
	pub name: &'a str,
	pub scale: f32,
}

/// The `UnitDef` struct records one `<unitdef>`, which lists the units of a single unit type.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitDef<'a> {
	pub name: &'a str,
	pub unit_type: &'a str,
	pub units: Vec<Unit<'a>, Alloc<'a>>,
}

/// The `Variant` struct records one named set of values a look may apply to a material.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant<'a> {
	pub name: &'a str,
	pub inputs: Vec<Input<'a>, Alloc<'a>>,
	pub tokens: Vec<Input<'a>, Alloc<'a>>,
}

/// The `VariantSet` struct records one `<variantset>`, whose variants are alternatives to one another.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantSet<'a> {
	pub name: &'a str,
	/// The node categories these variants are meant for.
	pub nodes: Vec<&'a str, Alloc<'a>>,
	/// The declarations these variants are meant for.
	pub node_defs: Vec<&'a str, Alloc<'a>>,
	pub variants: Vec<Variant<'a>, Alloc<'a>>,
}

/// The `Document` struct holds one parsed MaterialX document, with every reference still a name.
///
/// Read a [`Tree`] with [`Document::read`], fold in any [`Document::includes`] with
/// [`Document::merge`], then resolve the result into a graph with [`Dag::resolve`](super::Dag::resolve).
#[derive(Clone, Debug, PartialEq)]
pub struct Document<'a> {
	/// The specification version the document declares.
	pub version: Version,
	/// The working color space every color value in the document is written in.
	pub colorspace: Option<&'a str>,
	/// The namespace that qualifies the names this document defines.
	pub namespace: Option<&'a str>,
	/// The path prepended to every `filename` value in the document.
	pub file_prefix: Option<&'a str>,
	pub includes: Vec<Include<'a>, Alloc<'a>>,
	/// Inputs written at document scope, which form the interface the root scope's nodes reference.
	pub inputs: Vec<Input<'a>, Alloc<'a>>,
	/// Tokens written at document scope.
	pub tokens: Vec<Input<'a>, Alloc<'a>>,
	pub type_defs: Vec<TypeDef<'a>, Alloc<'a>>,
	pub unit_type_defs: Vec<UnitTypeDef<'a>, Alloc<'a>>,
	pub unit_defs: Vec<UnitDef<'a>, Alloc<'a>>,
	pub geom_prop_defs: Vec<GeomPropDef<'a>, Alloc<'a>>,
	pub node_defs: Vec<NodeDef<'a>, Alloc<'a>>,
	pub implementations: Vec<Implementation<'a>, Alloc<'a>>,
	pub node_graphs: Vec<NodeGraph<'a>, Alloc<'a>>,
	/// Node instances written at document scope, which is where shaders and materials usually live.
	pub nodes: Vec<Node<'a>, Alloc<'a>>,
	pub outputs: Vec<Output<'a>, Alloc<'a>>,
	pub variant_sets: Vec<VariantSet<'a>, Alloc<'a>>,
}

impl<'a> Document<'a> {
	/// Reads MaterialX source text into a document that borrows its text and draws its storage from
	/// `allocator`.
	///
	/// Next, resolve any `<xi:include>` the document lists, then call
	/// [`Dag::resolve`](super::Dag::resolve) to connect the graph.
	pub fn parse(source: &'a str, allocator: Alloc<'a>) -> Result<Self, ParseError> {
		Self::read(&Tree::parse_in(source, allocator)?, allocator)
	}

	/// Reads an already parsed XML tree into a document.
	///
	/// The document borrows from the source text and the allocator, never from the tree, so the tree
	/// may be dropped as soon as this returns. Use [`Document::parse`] unless the tree is needed for
	/// something else too.
	pub fn read(tree: &Tree<'a>, allocator: Alloc<'a>) -> Result<Self, ParseError> {
		let root = tree.root();

		if root.name() != "materialx" {
			return Err(ParseError::Root {
				name: root.name().to_string(),
				position: root.position(),
			});
		}

		let mut document = Document {
			version: read_version(root)?,
			colorspace: root.attribute("colorspace"),
			namespace: root.attribute("namespace"),
			file_prefix: root.attribute("fileprefix"),
			includes: Vec::new_in(allocator),
			inputs: Vec::new_in(allocator),
			tokens: Vec::new_in(allocator),
			type_defs: Vec::new_in(allocator),
			unit_type_defs: Vec::new_in(allocator),
			unit_defs: Vec::new_in(allocator),
			geom_prop_defs: Vec::new_in(allocator),
			node_defs: Vec::new_in(allocator),
			implementations: Vec::new_in(allocator),
			node_graphs: Vec::new_in(allocator),
			nodes: Vec::new_in(allocator),
			outputs: Vec::new_in(allocator),
			variant_sets: Vec::new_in(allocator),
		};

		for child in root.children() {
			document.read_child(child, allocator)?;
		}

		Ok(document)
	}

	fn read_child(&mut self, element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<(), ParseError> {
		reject_removed(element)?;

		match element.name() {
			"xi:include" => self.includes.push(Include {
				href: required(element, "href")?,
			}),
			"typedef" => self.type_defs.push(read_type_def(element, allocator)?),
			"unittypedef" => self.unit_type_defs.push(UnitTypeDef {
				name: required(element, "name")?,
			}),
			"unitdef" => self.unit_defs.push(read_unit_def(element, allocator)?),
			"geompropdef" => self.geom_prop_defs.push(read_geom_prop_def(element, allocator)?),
			"nodedef" => self.node_defs.push(read_node_def(element, allocator)?),
			"implementation" => self.implementations.push(read_implementation(element, allocator)?),
			"nodegraph" => self.node_graphs.push(read_node_graph(element, allocator)?),
			"variantset" => self.variant_sets.push(read_variant_set(element, allocator)?),
			"output" => self.outputs.push(read_output(element, allocator)?),
			"input" => self.inputs.push(read_input(element, allocator)?),
			"token" => self.tokens.push(read_token(element, allocator)?),
			name if IGNORED_ELEMENTS.contains(&name) => {}
			name if RESERVED_ELEMENTS.contains(&name) => {
				return Err(ParseError::MisplacedElement {
					name: element.name().to_string(),
					parent: "materialx".to_string(),
					position: element.position(),
				});
			}
			// Any element that is not reserved names a node category, which is how nodes are instantiated.
			_ => self.nodes.push(read_node(element, allocator)?),
		}

		Ok(())
	}

	/// Returns the semantic declared for a type, falling back to the standard library's semantics.
	pub fn semantic(&self, data_type: &DataType<'a>) -> TypeSemantic<'a> {
		self.type_defs
			.iter()
			.find(|type_def| type_def.name == data_type.name())
			.map(|type_def| type_def.semantic)
			.or_else(|| data_type.standard_semantic())
			.unwrap_or(TypeSemantic::Default)
	}

	/// Folds an included document into this one, as `<xi:include>` requires.
	///
	/// The included document's working color space, namespace, and file prefix are pushed onto the
	/// elements it defines, so those elements keep their original meaning inside this document.
	/// Elements whose name is already taken are dropped, which makes including the same library
	/// twice, directly and through another file, produce the same result as including it once.
	pub fn merge(&mut self, mut other: Document<'a>) {
		other.distribute_scope();

		self.includes.append(&mut other.includes);

		merge_by_name(&mut self.inputs, other.inputs);
		merge_by_name(&mut self.tokens, other.tokens);
		merge_by_name(&mut self.type_defs, other.type_defs);
		merge_by_name(&mut self.unit_type_defs, other.unit_type_defs);
		merge_by_name(&mut self.unit_defs, other.unit_defs);
		merge_by_name(&mut self.geom_prop_defs, other.geom_prop_defs);
		merge_by_name(&mut self.node_defs, other.node_defs);
		merge_by_name(&mut self.implementations, other.implementations);
		merge_by_name(&mut self.node_graphs, other.node_graphs);
		merge_by_name(&mut self.nodes, other.nodes);
		merge_by_name(&mut self.outputs, other.outputs);
		merge_by_name(&mut self.variant_sets, other.variant_sets);
	}

	/// Copies this document's root scope onto the elements that do not state one of their own.
	///
	/// Doing this before merging keeps an included document's colors, names, and file paths meaning
	/// the same thing after they move into another document's root scope.
	fn distribute_scope(&mut self) {
		for node in &mut self.nodes {
			node.colorspace = node.colorspace.or(self.colorspace);
			node.file_prefix = node.file_prefix.or(self.file_prefix);
		}

		for graph in &mut self.node_graphs {
			graph.colorspace = graph.colorspace.or(self.colorspace);
			graph.file_prefix = graph.file_prefix.or(self.file_prefix);
			graph.namespace = graph.namespace.or(self.namespace);
		}

		for node_def in &mut self.node_defs {
			node_def.namespace = node_def.namespace.or(self.namespace);
		}
	}
}

fn read_version(root: ElementRef<'_, '_>) -> Result<Version, ParseError> {
	let text = required(root, "version")?;

	let version = Version::parse(text).ok_or_else(|| ParseError::MalformedVersion {
		value: text.to_string(),
		position: root.attribute_position("version"),
	})?;

	if version < MINIMUM_VERSION || version > SUPPORTED_VERSION {
		return Err(ParseError::UnsupportedVersion {
			version,
			position: root.attribute_position("version"),
		});
	}

	Ok(version)
}

fn reject_removed(element: ElementRef<'_, '_>) -> Result<(), ParseError> {
	if REMOVED_ELEMENTS.contains(&element.name()) {
		return Err(ParseError::RemovedElement {
			name: element.name().to_string(),
			position: element.position(),
		});
	}

	Ok(())
}

fn required<'a>(element: ElementRef<'_, 'a>, attribute: &'static str) -> Result<&'a str, ParseError> {
	element.attribute(attribute).ok_or_else(|| ParseError::MissingAttribute {
		element: element.name().to_string(),
		attribute,
		position: element.position(),
	})
}

/// Reads a comma-separated attribute such as `target` or `internalgeomprops`.
fn text_list<'a>(element: ElementRef<'_, 'a>, attribute: &str, allocator: Alloc<'a>) -> Vec<&'a str, Alloc<'a>> {
	let mut entries = Vec::new_in(allocator);

	if let Some(value) = element.attribute(attribute) {
		entries.extend(value.split(',').map(str::trim).filter(|entry| !entry.is_empty()));
	}

	entries
}

fn flag(element: ElementRef<'_, '_>, attribute: &'static str) -> Result<bool, ParseError> {
	match element.attribute(attribute) {
		None => Ok(false),
		Some("true") => Ok(true),
		Some("false") => Ok(false),
		Some(value) => Err(ParseError::MalformedFlag {
			attribute,
			value: value.to_string(),
			position: element.attribute_position(attribute),
		}),
	}
}

fn data_type<'a>(element: ElementRef<'_, 'a>) -> Result<DataType<'a>, ParseError> {
	Ok(DataType::parse(required(element, "type")?))
}

fn read_value<'a>(
	element: ElementRef<'_, 'a>,
	attribute: &'static str,
	data_type: &DataType<'a>,
	allocator: Alloc<'a>,
) -> Result<Option<Value<'a>>, ParseError> {
	let Some(text) = element.attribute(attribute) else {
		return Ok(None);
	};

	Value::parse(data_type, text, allocator)
		.map(Some)
		.map_err(|error| ParseError::MalformedValue {
			element: element.name().to_string(),
			attribute,
			error,
			position: element.attribute_position(attribute),
		})
}

/// Reads the single upstream reference an input or output may carry.
///
/// The specification allows only one of `nodename`, `nodegraph` and `interfacename`, so carrying
/// more than one is an authoring error rather than a precedence question.
fn read_connection<'a>(element: ElementRef<'_, 'a>) -> Result<Option<Connection<'a>>, ParseError> {
	let node = element.attribute("nodename");
	let node_graph = element.attribute("nodegraph");
	let interface = element.attribute("interfacename");

	if [node.is_some(), node_graph.is_some(), interface.is_some()]
		.iter()
		.filter(|present| **present)
		.count()
		> 1
	{
		return Err(ParseError::ConflictingConnection {
			element: element.name().to_string(),
			position: element.position(),
		});
	}

	let output = element.attribute("output");

	Ok(match (node, node_graph, interface) {
		(Some(node), ..) => Some(Connection::Node { node, output }),
		(_, Some(node_graph), _) => Some(Connection::NodeGraph { node_graph, output }),
		(_, _, Some(input)) => Some(Connection::Interface { input }),
		_ => None,
	})
}

fn read_input<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<Input<'a>, ParseError> {
	let data_type = data_type(element)?;

	Ok(Input {
		name: required(element, "name")?,
		value: read_value(element, "value", &data_type, allocator)?,
		connection: read_connection(element)?,
		channels: element.attribute("channels"),
		colorspace: element.attribute("colorspace"),
		file_prefix: element.attribute("fileprefix"),
		unit: element.attribute("unit"),
		unit_type: element.attribute("unittype"),
		uniform: flag(element, "uniform")?,
		default_geom_prop: element.attribute("defaultgeomprop"),
		target: element.attribute("target"),
		data_type,
	})
}

/// Reads a `<token>`, which carries the same attributes as an input and is always uniform.
fn read_token<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<Input<'a>, ParseError> {
	Ok(Input {
		uniform: true,
		..read_input(element, allocator)?
	})
}

fn read_output<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<Output<'a>, ParseError> {
	let data_type = data_type(element)?;

	// `default` is only a hint for readers that cannot evaluate the node, and the standard library
	// writes geometric property names there, so text that is not a value is kept as written.
	let default_value = element
		.attribute("default")
		.map(|text| Value::parse(&data_type, text, allocator).unwrap_or(Value::Opaque(text)));

	Ok(Output {
		name: required(element, "name")?,
		connection: read_connection(element)?,
		default_input: element.attribute("defaultinput"),
		default_value,
		colorspace: element.attribute("colorspace"),
		uniform: flag(element, "uniform")?,
		data_type,
	})
}

/// Reads the `<input>`, `<token>` and `<output>` children shared by nodes, nodegraphs and nodedefs.
///
/// Returns whether the child was one of those three, so callers can handle the rest themselves.
fn read_ports<'a>(
	inputs: &mut Vec<Input<'a>, Alloc<'a>>,
	tokens: &mut Vec<Input<'a>, Alloc<'a>>,
	outputs: &mut Vec<Output<'a>, Alloc<'a>>,
	child: ElementRef<'_, 'a>,
	allocator: Alloc<'a>,
) -> Result<bool, ParseError> {
	match child.name() {
		"input" => inputs.push(read_input(child, allocator)?),
		"token" => tokens.push(read_token(child, allocator)?),
		"output" => outputs.push(read_output(child, allocator)?),
		_ => return Ok(false),
	}

	Ok(true)
}

fn misplaced(child: ElementRef<'_, '_>, parent: ElementRef<'_, '_>) -> ParseError {
	ParseError::MisplacedElement {
		name: child.name().to_string(),
		parent: parent.name().to_string(),
		position: child.position(),
	}
}

fn read_node<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<Node<'a>, ParseError> {
	let mut node = Node {
		category: element.name(),
		name: required(element, "name")?,
		data_type: data_type(element)?,
		node_def: element.attribute("nodedef"),
		version: element.attribute("version"),
		inherit: element.attribute("inherit"),
		colorspace: element.attribute("colorspace"),
		file_prefix: element.attribute("fileprefix"),
		inputs: Vec::new_in(allocator),
		tokens: Vec::new_in(allocator),
		outputs: Vec::new_in(allocator),
	};

	for child in element.children() {
		reject_removed(child)?;

		if !read_ports(&mut node.inputs, &mut node.tokens, &mut node.outputs, child, allocator)? {
			return Err(misplaced(child, element));
		}
	}

	Ok(node)
}

fn read_node_graph<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<NodeGraph<'a>, ParseError> {
	let mut graph = NodeGraph {
		name: required(element, "name")?,
		node_def: element.attribute("nodedef"),
		namespace: element.attribute("namespace"),
		colorspace: element.attribute("colorspace"),
		file_prefix: element.attribute("fileprefix"),
		inputs: Vec::new_in(allocator),
		tokens: Vec::new_in(allocator),
		nodes: Vec::new_in(allocator),
		node_graphs: Vec::new_in(allocator),
		outputs: Vec::new_in(allocator),
	};

	for child in element.children() {
		reject_removed(child)?;

		if read_ports(&mut graph.inputs, &mut graph.tokens, &mut graph.outputs, child, allocator)? {
			continue;
		}

		match child.name() {
			"nodegraph" => graph.node_graphs.push(read_node_graph(child, allocator)?),
			name if IGNORED_ELEMENTS.contains(&name) => {}
			name if RESERVED_ELEMENTS.contains(&name) => return Err(misplaced(child, element)),
			_ => graph.nodes.push(read_node(child, allocator)?),
		}
	}

	Ok(graph)
}

fn read_node_def<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<NodeDef<'a>, ParseError> {
	let mut node_def = NodeDef {
		name: required(element, "name")?,
		node: element.attribute("node").unwrap_or_default(),
		inherit: element.attribute("inherit"),
		node_group: element.attribute("nodegroup"),
		version: element.attribute("version"),
		is_default_version: flag(element, "isdefaultversion")?,
		targets: text_list(element, "target", allocator),
		namespace: element.attribute("namespace"),
		internal_geom_props: text_list(element, "internalgeomprops", allocator),
		inputs: Vec::new_in(allocator),
		tokens: Vec::new_in(allocator),
		outputs: Vec::new_in(allocator),
	};

	for child in element.children() {
		reject_removed(child)?;

		if read_ports(
			&mut node_def.inputs,
			&mut node_def.tokens,
			&mut node_def.outputs,
			child,
			allocator,
		)? {
			continue;
		}

		if !IGNORED_ELEMENTS.contains(&child.name()) {
			return Err(misplaced(child, element));
		}
	}

	Ok(node_def)
}

fn read_implementation<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<Implementation<'a>, ParseError> {
	Ok(Implementation {
		name: required(element, "name")?,
		node_def: required(element, "nodedef")?,
		node_graph: element.attribute("nodegraph"),
		impl_name: element.attribute("implname"),
		file: element.attribute("file"),
		source_code: element.attribute("sourcecode"),
		function: element.attribute("function"),
		targets: text_list(element, "target", allocator),
		format: element.attribute("format"),
	})
}

fn read_type_def<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<TypeDef<'a>, ParseError> {
	let mut type_def = TypeDef {
		name: required(element, "name")?,
		semantic: element
			.attribute("semantic")
			.map_or(TypeSemantic::Default, TypeSemantic::parse),
		context: element.attribute("context"),
		inherit: element.attribute("inherit"),
		hint: element.attribute("hint"),
		members: Vec::new_in(allocator),
	};

	for child in element.children() {
		if child.name() != "member" {
			return Err(misplaced(child, element));
		}

		let member_type = data_type(child)?;

		type_def.members.push(TypeMember {
			name: required(child, "name")?,
			value: read_value(child, "value", &member_type, allocator)?,
			data_type: member_type,
		});
	}

	Ok(type_def)
}

fn read_unit_def<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<UnitDef<'a>, ParseError> {
	let mut unit_def = UnitDef {
		name: required(element, "name")?,
		unit_type: required(element, "unittype")?,
		units: Vec::new_in(allocator),
	};

	for child in element.children() {
		if child.name() != "unit" {
			return Err(misplaced(child, element));
		}

		let Some(Value::Float(scale)) = read_value(child, "scale", &DataType::Float, allocator)? else {
			return Err(ParseError::MissingAttribute {
				element: child.name().to_string(),
				attribute: "scale",
				position: child.position(),
			});
		};

		unit_def.units.push(Unit {
			name: required(child, "name")?,
			scale,
		});
	}

	Ok(unit_def)
}

fn read_geom_prop_def<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<GeomPropDef<'a>, ParseError> {
	let index = match read_value(element, "index", &DataType::Integer, allocator)? {
		Some(Value::Integer(index)) => Some(index),
		_ => None,
	};

	Ok(GeomPropDef {
		name: required(element, "name")?,
		data_type: data_type(element)?,
		geom_prop: element.attribute("geomprop"),
		space: element.attribute("space"),
		index,
		uniform: flag(element, "uniform")?,
		unit: element.attribute("unit"),
		unit_type: element.attribute("unittype"),
	})
}

fn read_variant_set<'a>(element: ElementRef<'_, 'a>, allocator: Alloc<'a>) -> Result<VariantSet<'a>, ParseError> {
	let mut variant_set = VariantSet {
		name: required(element, "name")?,
		nodes: text_list(element, "node", allocator),
		node_defs: text_list(element, "nodedef", allocator),
		variants: Vec::new_in(allocator),
	};

	for child in element.children() {
		reject_removed(child)?;

		if child.name() != "variant" {
			return Err(misplaced(child, element));
		}

		let mut variant = Variant {
			name: required(child, "name")?,
			inputs: Vec::new_in(allocator),
			tokens: Vec::new_in(allocator),
		};

		let mut outputs = Vec::new_in(allocator);

		for port in child.children() {
			if !read_ports(&mut variant.inputs, &mut variant.tokens, &mut outputs, port, allocator)? {
				return Err(misplaced(port, child));
			}
		}

		variant_set.variants.push(variant);
	}

	Ok(variant_set)
}
