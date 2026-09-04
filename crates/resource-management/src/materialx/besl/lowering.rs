//! Walks a resolved MaterialX graph and records the BESL statements that reproduce it.

use std::collections::HashMap;

use besl::parser::Node;

use crate::materialx::{Dag, DataType, DeclarationId, GraphId, Input, NodeId, PortIndex, Source, Value};

use super::Texture;
use super::error::LowerError;
use super::nodes;
use super::syntax::{self, Expression};

/// The deepest chain of node graph instantiations the lowering follows before it gives up.
///
/// MaterialX forbids a node graph from instantiating itself, so this only trips on a document that
/// nests graphs far past anything an authoring tool writes. It also bounds every walk that follows a
/// connection across scopes, because a chain of those cannot be longer than the nesting itself.
const INLINING_LIMIT: usize = 64;

/// The `Argument` struct holds one interface input of an instantiated node scope.
///
/// The binding is kept as written rather than lowered on the spot, because a node definition declares
/// inputs its implementation may never read, and some of them, such as a file name, carry no value a
/// shader can compute with at all.
#[derive(Clone, Default)]
pub(super) struct Argument<'a> {
	/// The frame the binding was written in, and the port that wrote it.
	binding: Option<(usize, Input<'a>)>,
	/// The lowered binding, kept so an input read more than once is computed once.
	value: Option<Expression<'a>>,
}

/// The `Frame` struct records one instantiation of a node scope so reads of its interface resolve.
///
/// A node graph can be instantiated more than once with different inputs, so a lowered value belongs
/// to a frame rather than to the graph, and every node inside it lowers once per instantiation.
struct Frame<'a> {
	graph: GraphId,
	parent: Option<usize>,
	/// What the instantiation bound to each of the graph's interface inputs.
	///
	/// An entry left unbound falls back to whatever the graph itself wrote on that input, which is how
	/// a compound node graph carries its own bindings and how a node definition carries its defaults.
	arguments: Vec<Argument<'a>>,
}

/// The `Lowering` struct accumulates the BESL program one MaterialX node at a time.
///
/// Nodes are lowered on demand from the material outward, so a node nothing reads costs nothing and
/// every node reached lands in exactly one local. Call [`Lowering::source`] or [`Lowering::port`] to
/// lower whatever drives an input, then read [`Lowering::statements`] for the program body and
/// [`Lowering::textures`] for the images to bind.
pub(super) struct Lowering<'a, 'd> {
	pub dag: &'d Dag<'a>,
	/// The program body, in the order the values have to be computed.
	pub statements: Vec<Node<'a>>,
	/// The images the program samples, in the slot order the renderer binds them.
	pub textures: Vec<Texture<'a>>,
	frames: Vec<Frame<'a>>,
	/// The frame each node scope was instantiated into, so one scope read twice is lowered once.
	scopes: HashMap<(usize, GraphId), usize>,
	/// The frame each expanded node was instantiated into, for the same reason.
	expansions: HashMap<(usize, NodeId), usize>,
	/// The node graph implementing each declaration, looked up once per declaration.
	implementations: HashMap<DeclarationId, Option<GraphId>>,
	/// The values already lowered, keyed by the frame that owns them.
	values: HashMap<(usize, NodeId, PortIndex), Expression<'a>>,
	locals: u32,
}

impl<'a, 'd> Lowering<'a, 'd> {
	/// The frame holding the nodes written directly inside `<materialx>`.
	pub const ROOT_FRAME: usize = 0;

	/// Starts a lowering whose first frame is the document's own scope.
	pub fn new(dag: &'d Dag<'a>) -> Self {
		Lowering {
			dag,
			statements: Vec::with_capacity(64),
			textures: Vec::new(),
			frames: vec![Frame {
				graph: GraphId::ROOT,
				parent: None,
				arguments: Vec::new(),
			}],
			scopes: HashMap::new(),
			expansions: HashMap::new(),
			implementations: HashMap::new(),
			values: HashMap::new(),
			locals: 0,
		}
	}

	/// Declares a local holding `syntax` and returns an expression that reads it back.
	///
	/// Every node lands in a local so that component access, which repeats its operand once per
	/// component, never repeats the work behind it.
	pub fn bind(&mut self, hint: &str, data_type: DataType<'a>, syntax: Node<'a>) -> Result<Expression<'a>, LowerError> {
		let type_name = syntax::type_name(data_type, hint)?;
		let name = self.local(hint);

		self.statements.push(Node::let_assignment(name.clone(), type_name, syntax));

		Ok(Expression::new(Node::member_expression(name), data_type))
	}

	/// Returns a value component access can read more than once, binding it to a local first when it cannot.
	pub fn addressable(&mut self, hint: &str, value: Expression<'a>) -> Result<Expression<'a>, LowerError> {
		let readable = matches!(
			value.syntax.node(),
			besl::parser::Nodes::Expression(besl::parser::Expressions::Member { .. })
		);

		// A one-component value is never read through a component, so repeating it costs nothing either.
		if readable || value.width() == 1 {
			return Ok(value);
		}

		let data_type = value.data_type;

		self.bind(hint, data_type, value.syntax)
	}

	/// Builds a unique BESL identifier from a MaterialX name.
	///
	/// MaterialX names allow characters BESL identifiers do not, and two node graphs may each hold a
	/// node of the same name, so the name is both filtered and numbered.
	fn local(&mut self, hint: &str) -> String {
		let index = self.locals;
		self.locals += 1;

		let mut name = String::with_capacity(hint.len() + 12);
		name.push_str("mtlx_");

		for character in hint.chars() {
			name.push(if character.is_ascii_alphanumeric() { character } else { '_' });
		}

		name.push('_');
		name.push_str(&index.to_string());
		name
	}

	/// Lowers whatever drives one resolved port, applying any channel selection it carries.
	pub fn port(&mut self, frame: usize, input: &Input<'a>) -> Result<Expression<'a>, LowerError> {
		let value = self.source(frame, &input.source, input.data_type, input.name)?;

		match input.channels {
			// Documents written before 1.39 select channels on the connection rather than with a node.
			Some(channels) => nodes::swizzle(self, input.name, value, channels, input.data_type),
			None => Ok(value),
		}
	}

	/// Lowers whatever drives one port.
	///
	/// `hint` names the element the port belongs to and only reaches the generated program's local
	/// names and its diagnostics.
	pub fn source(
		&mut self,
		frame: usize,
		source: &Source<'a>,
		data_type: DataType<'a>,
		hint: &str,
	) -> Result<Expression<'a>, LowerError> {
		match source {
			Source::Value(value) => Ok(Expression::new(constant(value, data_type, hint)?, data_type)),
			Source::Node { node, output } => self.node_output(frame, *node, *output),
			Source::Graph { graph, output } => {
				let instance = self.scope(frame, *graph)?;

				self.graph_result(instance, *graph, *output)
			}
			Source::Interface { graph, input } => self.interface(frame, *graph, *input, hint),
			Source::GeomProp(property) => geometry(property, hint),
			// An unconnected value input is MaterialX's zero, which every width spells the same way.
			Source::Unconnected => Ok(Expression::new(zero(data_type, hint)?, data_type)),
		}
	}

	/// Reads one input of a node, following its declaration when the document leaves the input out.
	///
	/// Returns nothing when neither the document nor the declaration says what the input carries, which
	/// lets the caller supply the constant the node's own semantics fall back to.
	pub fn input(&mut self, frame: usize, node: NodeId, name: &str) -> Result<Option<Expression<'a>>, LowerError> {
		let dag = self.dag;
		let instance = dag.node(node);

		if let Some(input) = instance.input(name) {
			return self.port(frame, input).map(Some);
		}

		let Some(declaration) = instance.declaration else {
			return Ok(None);
		};

		let Some(port) = dag.declaration(declaration).inputs.iter().find(|port| port.name == name) else {
			return Ok(None);
		};

		if let Some(property) = port.default_geom_prop {
			return geometry(property, name).map(Some);
		}

		let Some(value) = &port.default else {
			return Ok(None);
		};

		Ok(Some(Expression::new(constant(value, port.data_type, name)?, port.data_type)))
	}

	/// Lowers one output of a node, reusing the local it already landed in.
	fn node_output(&mut self, frame: usize, node: NodeId, output: PortIndex) -> Result<Expression<'a>, LowerError> {
		if let Some(value) = self.values.get(&(frame, node, output)) {
			return Ok(value.clone());
		}

		let value = nodes::lower(self, frame, node, output)?;

		self.values.insert((frame, node, output), value.clone());

		Ok(value)
	}

	/// Lowers the node behind one of a scope's outputs, in a frame that scope was instantiated into.
	pub fn graph_result(&mut self, instance: usize, graph: GraphId, output: PortIndex) -> Result<Expression<'a>, LowerError> {
		let scope = self.dag.graph(graph);
		let port = scope.outputs.get(output.index()).ok_or_else(|| LowerError::UnknownOutput {
			node: scope.name.to_string(),
			output: output.index() as u32,
		})?;

		self.port(instance, port)
	}

	/// Opens the chain of scopes a node was written inside, and returns the frame it belongs to.
	///
	/// A node written inside a node graph reads that graph's interface, so it can only be lowered from a
	/// frame that graph has been instantiated into.
	pub fn enter(&mut self, graph: GraphId) -> Result<usize, LowerError> {
		if graph == GraphId::ROOT {
			return Ok(Self::ROOT_FRAME);
		}

		let outer = self.enter(self.dag.graph(graph).parent.unwrap_or(GraphId::ROOT))?;

		self.scope(outer, graph)
	}

	/// Returns the frame a node scope read from `frame` is instantiated into, opening one on first read.
	///
	/// A scope read this way binds its own interface, so every read from one frame shares an
	/// instantiation and the nodes inside it lower once.
	fn scope(&mut self, frame: usize, graph: GraphId) -> Result<usize, LowerError> {
		if let Some(instance) = self.scopes.get(&(frame, graph)) {
			return Ok(*instance);
		}

		let name = self.dag.graph(graph).name;
		let instance = self.open(frame, graph, Vec::new(), name)?;

		self.scopes.insert((frame, graph), instance);

		Ok(instance)
	}

	/// Returns the frame a node's implementing graph expands into, opening one on first read.
	///
	/// Two nodes of the same category bind the same graph differently, so an expansion is keyed by the
	/// node rather than by the graph.
	pub fn expansion(&mut self, frame: usize, node: NodeId, graph: GraphId) -> Result<usize, LowerError> {
		if let Some(instance) = self.expansions.get(&(frame, node)) {
			return Ok(*instance);
		}

		let dag = self.dag;
		let instance = dag.node(node);
		let name = instance.name;
		// The node's own inputs bind the graph's interface; anything it leaves out keeps the declared default.
		let arguments = dag
			.graph(graph)
			.interface
			.iter()
			.map(|port| Argument {
				binding: instance.input(port.name).map(|input| (frame, input.clone())),
				value: None,
			})
			.collect();

		let expansion = self.open(frame, graph, arguments, name)?;

		self.expansions.insert((frame, node), expansion);

		Ok(expansion)
	}

	/// Opens a frame for one instantiation of a node scope.
	fn open(
		&mut self,
		frame: usize,
		graph: GraphId,
		arguments: Vec<Argument<'a>>,
		hint: &str,
	) -> Result<usize, LowerError> {
		if self.depth(frame) >= INLINING_LIMIT {
			return Err(LowerError::InliningLimitExceeded { node: hint.to_string() });
		}

		let instance = self.frames.len();

		self.frames.push(Frame {
			graph,
			parent: Some(frame),
			arguments,
		});

		Ok(instance)
	}

	/// Returns the node graph implementing one declaration, when the document carries one.
	pub fn implementation(&mut self, declaration: Option<DeclarationId>) -> Option<GraphId> {
		let declaration = declaration?;
		let dag = self.dag;

		*self
			.implementations
			.entry(declaration)
			.or_insert_with(|| dag.implementation(declaration))
	}

	/// Resolves one read of a node scope's interface to the port that binds it and the frame it was written in.
	///
	/// The binding is the one the instantiation wrote; when it wrote none, the scope's own port applies,
	/// and that was written one frame further out. Nothing at all means the input has no default, which
	/// is what a node definition's own interface says about every input it declares.
	fn binding<'s>(&'s self, frame: usize, graph: GraphId, input: PortIndex) -> Option<(usize, &'s Input<'a>)> {
		// The read may come from a node nested below the scope that declares the interface.
		let owner = self.owner(frame, graph).unwrap_or(frame);

		if let Some(Argument {
			binding: Some((caller, port)),
			..
		}) = self.frames[owner].arguments.get(input.index())
		{
			return Some((*caller, port));
		}

		let declared = self.dag.graph(graph).interface.get(input.index())?;

		// An interface resolving to itself is how a node definition spells an input with no default.
		if matches!(declared.source, Source::Interface { .. }) {
			return None;
		}

		Some((self.frames[owner].parent.unwrap_or(Self::ROOT_FRAME), declared))
	}

	/// Lowers a read of one of the enclosing scope's interface inputs.
	fn interface(
		&mut self,
		frame: usize,
		graph: GraphId,
		input: PortIndex,
		hint: &str,
	) -> Result<Expression<'a>, LowerError> {
		let owner = self.owner(frame, graph).unwrap_or(frame);
		let port = input.index();

		if let Some(value) = self.frames[owner].arguments.get(port).and_then(|argument| argument.value.clone()) {
			return Ok(value);
		}

		let Some((caller, bound)) = self.binding(frame, graph, input) else {
			let scope = self.dag.graph(graph);
			let declared = scope.interface.get(port).ok_or_else(|| LowerError::UnknownOutput {
				node: scope.name.to_string(),
				output: port as u32,
			})?;

			return Ok(Expression::new(zero(declared.data_type, hint)?, declared.data_type));
		};

		let bound = bound.clone();
		let value = self.port(caller, &bound)?;

		if let Some(argument) = self.frames[owner].arguments.get_mut(port) {
			argument.value = Some(value.clone());
		}

		Ok(value)
	}

	/// Returns the constant one input of a node carries, following interface bindings to reach it.
	///
	/// File names, channel strings and indices are read rather than computed, so they are traced through
	/// a node graph's interface without lowering anything.
	pub fn parameter<'s>(&'s self, frame: usize, node: NodeId, name: &str) -> Option<&'s Value<'a>> {
		let instance = self.dag.node(node);

		let Some(input) = instance.input(name) else {
			// The document wrote nothing on the input, so the declaration's own default is the constant.
			return self
				.dag
				.declaration(instance.declaration?)
				.inputs
				.iter()
				.find(|port| port.name == name)?
				.default
				.as_ref();
		};

		let mut frame = frame;
		let mut source = &input.source;

		// A binding may pass through any number of interfaces; the nesting limit bounds the chain.
		for _ in 0..INLINING_LIMIT {
			match source {
				Source::Value(value) => return Some(value),
				Source::Interface { graph, input } => {
					let (caller, bound) = self.binding(frame, *graph, *input)?;

					frame = caller;
					source = &bound.source;
				}
				_ => return None,
			}
		}

		None
	}

	/// Finds the frame that instantiated one node scope, starting from a frame nested inside it.
	fn owner(&self, frame: usize, graph: GraphId) -> Option<usize> {
		self.ancestry(frame).find(|index| self.frames[*index].graph == graph)
	}

	/// Counts how many scopes a frame is nested inside.
	fn depth(&self, frame: usize) -> usize {
		self.ancestry(frame).count() - 1
	}

	/// Walks a frame and every frame it was instantiated from, outermost last.
	fn ancestry(&self, frame: usize) -> impl Iterator<Item = usize> + '_ {
		std::iter::successors(Some(frame), |index| self.frames[*index].parent)
	}

	/// Follows a shader-semantic connection to the node that produces it, and to the frame it lives in.
	///
	/// Shader and material inputs carry closures rather than values, so they are traced structurally
	/// instead of being lowered into an expression.
	pub fn shader(&mut self, frame: usize, source: &Source<'a>) -> Result<Option<(usize, NodeId)>, LowerError> {
		let mut frame = frame;
		let mut source = source.clone();

		// A shader may sit behind any number of node graphs; the nesting limit bounds the chain.
		for _ in 0..INLINING_LIMIT {
			let behind = match source {
				Source::Node { node, output } => match self.through(frame, node, output.index())? {
					Some(behind) => behind,
					None => return Ok(Some((frame, node))),
				},
				Source::Graph { graph, output } => {
					let instance = self.scope(frame, graph)?;
					let Some(port) = self.dag.graph(graph).outputs.get(output.index()) else {
						return Ok(None);
					};

					(instance, port.source.clone())
				}
				// A material written inside a node graph takes its shader from that graph's interface.
				Source::Interface { graph, input } => {
					let Some((caller, bound)) = self.binding(frame, graph, input) else {
						return Ok(None);
					};

					(caller, bound.source.clone())
				}
				_ => return Ok(None),
			};

			(frame, source) = behind;
		}

		Ok(None)
	}

	/// Follows a material node through the node graph that defines it, when one does.
	///
	/// A document may define a material of its own, whose implementation holds the `<surfacematerial>`
	/// that names the surface shader. Resolving it first lets one path read the shader off either.
	pub fn resolve(&mut self, frame: usize, node: NodeId) -> Result<(usize, NodeId), LowerError> {
		// A material definition carries its material on its first output.
		let Some((expansion, behind)) = self.through(frame, node, 0)? else {
			return Ok((frame, node));
		};

		Ok(self.shader(expansion, &behind)?.unwrap_or((frame, node)))
	}

	/// Expands a node the document defines with a node graph, returning what sits behind one output.
	pub fn through(
		&mut self,
		frame: usize,
		node: NodeId,
		output: usize,
	) -> Result<Option<(usize, Source<'a>)>, LowerError> {
		let instance = self.dag.node(node);

		if super::surface::is_shading_model(instance.category) {
			return Ok(None);
		}

		let Some(graph) = self.implementation(instance.declaration) else {
			return Ok(None);
		};

		let expansion = self.expansion(frame, node, graph)?;

		let Some(port) = self.dag.graph(graph).outputs.get(output) else {
			return Ok(None);
		};

		Ok(Some((expansion, port.source.clone())))
	}

	/// Records one image and returns the BESL variable the renderer binds it to.
	///
	/// Images naming the same file in the same colour space share one slot, because the renderer binds
	/// one texture per slot and sampling it twice would cost two descriptors.
	pub fn texture(&mut self, texture: Texture<'a>) -> Node<'a> {
		let slot = self.textures.iter().position(|entry| *entry == texture).unwrap_or_else(|| {
			self.textures.push(texture);
			self.textures.len() - 1
		});

		Node::member_expression(crate::pbr::material_texture_variable_name(slot as u32))
	}
}

/// Writes the value a type falls back to when nothing drives it.
fn zero<'a>(data_type: DataType<'a>, hint: &str) -> Result<Node<'a>, LowerError> {
	Ok(syntax::splat_literal(0.0, syntax::width(data_type, hint)?))
}

/// Writes one MaterialX constant as a BESL expression.
fn constant<'a>(value: &Value<'a>, data_type: DataType<'a>, hint: &str) -> Result<Node<'a>, LowerError> {
	Ok(match value {
		// A boolean lowers to the number a comparison would produce, so nothing has to convert it.
		Value::Boolean(value) => syntax::literal(if *value { 1.0 } else { 0.0 }),
		Value::Integer(value) => Node::literal_expression(value.to_string()),
		Value::Float(_)
		| Value::Color3(_)
		| Value::Color4(_)
		| Value::Vector2(_)
		| Value::Vector3(_)
		| Value::Vector4(_) => {
			// Every one of these arms carries its components, so the fallback never fires.
			let components = value.components().unwrap_or(&[0.0]);

			syntax::construct(components.iter().copied().map(syntax::literal).collect())
		}
		// A shader-semantic input written as the empty string is how MaterialX leaves it unplugged.
		Value::Opaque("") => zero(data_type, hint)?,
		_ => {
			return Err(LowerError::UnsupportedType {
				node: hint.to_string(),
				data_type: data_type.name().to_string(),
			});
		}
	})
}

/// Returns the material stage value carrying one MaterialX geometric property.
///
/// The stage reconstructs the shaded point in world space, so an object-space or model-space property
/// reads the same value as its world-space companion.
pub(super) fn geometry<'a>(property: &str, hint: &str) -> Result<Expression<'a>, LowerError> {
	let (name, data_type) = match property {
		"Pworld" | "Pobject" | "Pmodel" => ("world_space_vertex_position", DataType::Vector3),
		"Nworld" | "Nobject" | "Nmodel" => ("N", DataType::Vector3),
		"Tworld" | "Tobject" | "Tmodel" => ("T", DataType::Vector3),
		"Bworld" | "Bobject" | "Bmodel" => ("B", DataType::Vector3),
		"Vworld" => ("V", DataType::Vector3),
		"UV0" | "st" | "uv" => ("vertex_uv", DataType::Vector2),
		_ => {
			return Err(LowerError::UnknownGeometricProperty {
				node: hint.to_string(),
				property: property.to_string(),
			});
		}
	};

	Ok(Expression::new(Node::member_expression(name), data_type))
}
