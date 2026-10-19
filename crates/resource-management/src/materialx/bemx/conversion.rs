//! Walks a resolved MaterialX graph and adds the BEMX nodes that reproduce it.

use std::collections::HashMap;

use super::Texture;
use super::error::ConvertError;
use super::nodes;
use crate::bemx::{self, BinaryOperator, GraphBuilder, Type};
use crate::materialx::{Dag, DataType, DeclarationId, GraphId, Input, NodeId, PortIndex, Source, Value};

/// The deepest chain of node graph instantiations the conversion follows before it gives up.
///
/// MaterialX forbids a node graph from instantiating itself, so this only trips on a document that
/// nests graphs far past anything an authoring tool writes. It also bounds every walk that follows a
/// connection across scopes, because a chain of those cannot be longer than the nesting itself.
const INLINING_LIMIT: usize = 64;

/// The `Argument` struct holds one interface input of an instantiated node scope.
///
/// The binding is kept as written rather than converted on the spot, because a node definition
/// declares inputs its implementation may never read, and some of them, such as a file name, carry no
/// value a material can compute with at all.
#[derive(Clone, Default)]
struct Argument<'a> {
	/// The frame the binding was written in, and the port that wrote it.
	binding: Option<(usize, Input<'a>)>,
	/// The converted binding, kept so an input read more than once becomes one BEMX node.
	value: Option<bemx::NodeId>,
}

/// The `Frame` struct records one instantiation of a node scope so reads of its interface resolve.
///
/// A node graph can be instantiated more than once with different inputs, so a converted value
/// belongs to a frame rather than to the graph, and every node inside it converts once per
/// instantiation.
struct Frame<'a> {
	graph: GraphId,
	parent: Option<usize>,
	/// What the instantiation bound to each of the graph's interface inputs.
	///
	/// An entry left unbound falls back to whatever the graph itself wrote on that input, which is how
	/// a compound node graph carries its own bindings and how a node definition carries its defaults.
	arguments: Vec<Argument<'a>>,
}

/// The `Geometry` enum names the geometric BEMX nodes a conversion shares between every node that reads them.
#[derive(Clone, Copy)]
enum Geometry {
	Position,
	Normal,
	Tangent,
	Bitangent,
	/// BEMX's view direction, from the viewer toward the shaded point.
	ViewDirection,
	/// The direction from the shaded point toward the viewer, which MaterialX's `Vworld` reads.
	TowardViewer,
	/// Texture coordinate set zero.
	TexCoord,
}

/// The `Converter` struct accumulates the BEMX graph one MaterialX node at a time.
///
/// Nodes are converted on demand from the material outward, so a node nothing reads costs nothing
/// and every node reached becomes exactly one BEMX node per instantiation. Call
/// [`Converter::source`] or [`Converter::port`] to convert whatever drives an input, then take the
/// builder and the images with [`Converter::finish`].
pub(super) struct Converter<'a, 'd> {
	pub dag: &'d Dag<'a>,
	builder: GraphBuilder,
	/// The images the graph samples, in the slot order the renderer binds them.
	textures: Vec<Texture<'a>>,
	frames: Vec<Frame<'a>>,
	/// The frame each node scope was instantiated into, so one scope read twice is converted once.
	scopes: HashMap<(usize, GraphId), usize>,
	/// The frame each expanded node was instantiated into, for the same reason.
	expansions: HashMap<(usize, NodeId), usize>,
	/// The node graph implementing each declaration, looked up once per declaration.
	implementations: HashMap<DeclarationId, Option<GraphId>>,
	/// The values already converted, keyed by the frame that owns them.
	values: HashMap<(usize, NodeId, PortIndex), bemx::NodeId>,
	/// Every constant added so far, keyed by its width and the bits of its components, so equal constants share one node.
	constants: HashMap<(usize, [u32; 4]), bemx::NodeId>,
	/// The geometric nodes added so far, indexed by [`Geometry`].
	geometry: [Option<bemx::NodeId>; 7],
}

impl<'a, 'd> Converter<'a, 'd> {
	/// The frame holding the nodes written directly inside `<materialx>`.
	pub const ROOT_FRAME: usize = 0;

	/// Starts a conversion whose first frame is the document's own scope.
	pub fn new(dag: &'d Dag<'a>) -> Self {
		Converter {
			dag,
			builder: GraphBuilder::new(),
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
			constants: HashMap::new(),
			geometry: [None; 7],
		}
	}

	/// Returns the graph builder and the images its image nodes sample, in slot order.
	pub fn finish(self) -> (GraphBuilder, Vec<Texture<'a>>) {
		(self.builder, self.textures)
	}

	/// Adds one BEMX node, naming the MaterialX node `hint` when it does not type-check.
	pub fn add(&mut self, hint: &str, node: bemx::Node) -> Result<bemx::NodeId, ConvertError> {
		self.builder.add(node).map_err(|error| ConvertError::Graph {
			node: hint.to_string(),
			error,
		})
	}

	/// Adds a constant, reusing the node of an equal constant added before.
	pub fn constant(&mut self, value: bemx::Value) -> bemx::NodeId {
		let components = value.components();
		let mut bits = [0; 4];

		for (bit, component) in bits.iter_mut().zip(components) {
			*bit = component.to_bits();
		}

		let builder = &mut self.builder;

		*self
			.constants
			.entry((components.len(), bits))
			.or_insert_with(|| builder.constant(value))
	}

	/// Adds a constant of type `type` that holds `value` in every component.
	pub fn splat(&mut self, value: f32, r#type: Type) -> bemx::NodeId {
		let components = [value; 4];
		let value =
			bemx::Value::from_components(&components[..r#type.width()]).expect("Every type holds one to four components.");

		self.constant(value)
	}

	/// Returns the type a converted value carries.
	pub fn r#type(&self, value: bemx::NodeId) -> Type {
		self.builder
			.r#type(value)
			.expect("Converted values are never the surface node, which is added last.")
	}

	/// Converts whatever drives one resolved port, applying any channel selection it carries.
	pub fn port(&mut self, frame: usize, input: &Input<'a>) -> Result<bemx::NodeId, ConvertError> {
		let value = self.source(frame, &input.source, input.data_type, input.name)?;

		match input.channels {
			// Documents written before 1.39 select channels on the connection rather than with a node.
			Some(channels) => {
				let result = bemx_type(input.data_type, input.name)?;

				nodes::swizzle(self, input.name, value, channels, result)
			}
			None => Ok(value),
		}
	}

	/// Converts whatever drives one port.
	///
	/// `hint` names the element the port belongs to and only reaches diagnostics.
	pub fn source(
		&mut self,
		frame: usize,
		source: &Source<'a>,
		data_type: DataType<'a>,
		hint: &str,
	) -> Result<bemx::NodeId, ConvertError> {
		match source {
			Source::Value(value) => self.value(value, data_type, hint),
			Source::Node { node, output } => self.node_output(frame, *node, *output),
			Source::Graph { graph, output } => {
				let instance = self.scope(frame, *graph)?;

				self.graph_result(instance, *graph, *output)
			}
			Source::Interface { graph, input } => self.interface(frame, *graph, *input, hint),
			Source::GeomProp(property) => self.geometry(property, hint),
			// An unconnected value input is MaterialX's zero.
			Source::Unconnected => self.zero(data_type, hint),
		}
	}

	/// Reads one input of a node, following its declaration when the document leaves the input out.
	///
	/// Returns nothing when neither the document nor the declaration says what the input carries, which
	/// lets the caller supply the constant the node's own semantics fall back to.
	pub fn input(&mut self, frame: usize, node: NodeId, name: &str) -> Result<Option<bemx::NodeId>, ConvertError> {
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
			return self.geometry(property, name).map(Some);
		}

		let Some(value) = &port.default else {
			return Ok(None);
		};

		self.value(value, port.data_type, name).map(Some)
	}

	/// Converts one output of a node, reusing the BEMX node it already became.
	fn node_output(&mut self, frame: usize, node: NodeId, output: PortIndex) -> Result<bemx::NodeId, ConvertError> {
		if let Some(value) = self.values.get(&(frame, node, output)) {
			return Ok(*value);
		}

		let value = nodes::convert(self, frame, node, output)?;

		self.values.insert((frame, node, output), value);

		Ok(value)
	}

	/// Converts the node behind one of a scope's outputs, in a frame that scope was instantiated into.
	pub fn graph_result(&mut self, instance: usize, graph: GraphId, output: PortIndex) -> Result<bemx::NodeId, ConvertError> {
		let scope = self.dag.graph(graph);
		let port = scope.outputs.get(output.index()).ok_or_else(|| ConvertError::UnknownOutput {
			node: scope.name.to_string(),
			output: output.index() as u32,
		})?;

		self.port(instance, port)
	}

	/// Opens the chain of scopes a node was written inside, and returns the frame it belongs to.
	///
	/// A node written inside a node graph reads that graph's interface, so it can only be converted from
	/// a frame that graph has been instantiated into.
	pub fn enter(&mut self, graph: GraphId) -> Result<usize, ConvertError> {
		if graph == GraphId::ROOT {
			return Ok(Self::ROOT_FRAME);
		}

		let outer = self.enter(self.dag.graph(graph).parent.unwrap_or(GraphId::ROOT))?;

		self.scope(outer, graph)
	}

	/// Returns the frame a node scope read from `frame` is instantiated into, opening one on first read.
	///
	/// A scope read this way binds its own interface, so every read from one frame shares an
	/// instantiation and the nodes inside it convert once.
	fn scope(&mut self, frame: usize, graph: GraphId) -> Result<usize, ConvertError> {
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
	pub fn expansion(&mut self, frame: usize, node: NodeId, graph: GraphId) -> Result<usize, ConvertError> {
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
	fn open(&mut self, frame: usize, graph: GraphId, arguments: Vec<Argument<'a>>, hint: &str) -> Result<usize, ConvertError> {
		if self.depth(frame) >= INLINING_LIMIT {
			return Err(ConvertError::InliningLimitExceeded { node: hint.to_string() });
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

	/// Converts a read of one of the enclosing scope's interface inputs.
	fn interface(&mut self, frame: usize, graph: GraphId, input: PortIndex, hint: &str) -> Result<bemx::NodeId, ConvertError> {
		let owner = self.owner(frame, graph).unwrap_or(frame);
		let port = input.index();

		if let Some(value) = self.frames[owner].arguments.get(port).and_then(|argument| argument.value) {
			return Ok(value);
		}

		let Some((caller, bound)) = self.binding(frame, graph, input) else {
			let scope = self.dag.graph(graph);
			let declared = scope.interface.get(port).ok_or_else(|| ConvertError::UnknownOutput {
				node: scope.name.to_string(),
				output: port as u32,
			})?;

			return self.zero(declared.data_type, hint);
		};

		let bound = bound.clone();
		let value = self.port(caller, &bound)?;

		if let Some(argument) = self.frames[owner].arguments.get_mut(port) {
			argument.value = Some(value);
		}

		Ok(value)
	}

	/// Returns the constant one input of a node carries, following interface bindings to reach it.
	///
	/// File names, channel strings and indices are read rather than computed, so they are traced through
	/// a node graph's interface without converting anything.
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
	/// instead of being converted into a node.
	pub fn shader(&mut self, frame: usize, source: &Source<'a>) -> Result<Option<(usize, NodeId)>, ConvertError> {
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
	pub fn resolve(&mut self, frame: usize, node: NodeId) -> Result<(usize, NodeId), ConvertError> {
		// A material definition carries its material on its first output.
		let Some((expansion, behind)) = self.through(frame, node, 0)? else {
			return Ok((frame, node));
		};

		Ok(self.shader(expansion, &behind)?.unwrap_or((frame, node)))
	}

	/// Expands a node the document defines with a node graph, returning what sits behind one output.
	pub fn through(&mut self, frame: usize, node: NodeId, output: usize) -> Result<Option<(usize, Source<'a>)>, ConvertError> {
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

	/// Records one image and returns the texture parameter its image nodes sample.
	///
	/// Images naming the same file in the same colour space share one slot, because the renderer binds
	/// one texture per slot and sampling it twice would cost two descriptors.
	pub fn texture(&mut self, texture: Texture<'a>) -> String {
		let slot = self.textures.iter().position(|entry| *entry == texture).unwrap_or_else(|| {
			self.textures.push(texture);
			self.textures.len() - 1
		});

		bemx::material_texture_parameter_name(slot)
	}

	/// Adds the value a type falls back to when nothing drives it.
	fn zero(&mut self, data_type: DataType<'a>, hint: &str) -> Result<bemx::NodeId, ConvertError> {
		let r#type = bemx_type(data_type, hint)?;

		Ok(self.splat(0.0, r#type))
	}

	/// Adds one MaterialX constant.
	fn value(&mut self, value: &Value<'a>, data_type: DataType<'a>, hint: &str) -> Result<bemx::NodeId, ConvertError> {
		let value = match value {
			// BEMX computes in floats alone, so a boolean becomes the number a comparison would produce.
			Value::Boolean(value) => bemx::Value::Float(f32::from(u8::from(*value))),
			Value::Integer(value) => bemx::Value::Float(*value as f32),
			Value::Float(value) => bemx::Value::Float(*value),
			Value::Vector2(value) => bemx::Value::Vector2(*value),
			Value::Color3(value) | Value::Vector3(value) => bemx::Value::Vector3(*value),
			Value::Color4(value) | Value::Vector4(value) => bemx::Value::Vector4(*value),
			// A shader-semantic input written as the empty string is how MaterialX leaves it unplugged.
			Value::Opaque("") => return self.zero(data_type, hint),
			_ => {
				return Err(ConvertError::UnsupportedType {
					node: hint.to_string(),
					data_type: data_type.name().to_string(),
				});
			}
		};

		Ok(self.constant(value))
	}

	/// Returns the BEMX node carrying one MaterialX geometric property.
	///
	/// BEMX reads geometry in world space, so an object-space or model-space property reads the same
	/// value as its world-space companion.
	pub fn geometry(&mut self, property: &str, hint: &str) -> Result<bemx::NodeId, ConvertError> {
		let geometry = match property {
			"Pworld" | "Pobject" | "Pmodel" => Geometry::Position,
			"Nworld" | "Nobject" | "Nmodel" => Geometry::Normal,
			"Tworld" | "Tobject" | "Tmodel" => Geometry::Tangent,
			"Bworld" | "Bobject" | "Bmodel" => Geometry::Bitangent,
			"Vworld" => Geometry::TowardViewer,
			property if super::nodes::TEXTURE_COORDINATES.contains(&property) => Geometry::TexCoord,
			_ => {
				return Err(ConvertError::UnknownGeometricProperty {
					node: hint.to_string(),
					property: property.to_string(),
				});
			}
		};

		self.geometry_node(geometry, hint)
	}

	/// Returns BEMX's view direction, which points from the viewer toward the shaded point.
	pub fn view_direction(&mut self, hint: &str) -> Result<bemx::NodeId, ConvertError> {
		self.geometry_node(Geometry::ViewDirection, hint)
	}

	/// Returns the node for one geometric value, adding it on first use.
	fn geometry_node(&mut self, geometry: Geometry, hint: &str) -> Result<bemx::NodeId, ConvertError> {
		if let Some(node) = self.geometry[geometry as usize] {
			return Ok(node);
		}

		// Every value but the one toward the viewer is a node of its own.
		let read = match geometry {
			Geometry::Position => Some(bemx::Node::Position),
			Geometry::Normal => Some(bemx::Node::Normal),
			Geometry::Tangent => Some(bemx::Node::Tangent),
			Geometry::Bitangent => Some(bemx::Node::Bitangent),
			Geometry::ViewDirection => Some(bemx::Node::ViewDirection),
			Geometry::TexCoord => Some(bemx::Node::TexCoord { index: 0 }),
			Geometry::TowardViewer => None,
		};

		let node = match read {
			Some(node) => self.add(hint, node)?,
			None => {
				// MaterialX's `Vworld` points at the viewer, the opposite of BEMX's view direction.
				let view = self.geometry_node(Geometry::ViewDirection, hint)?;
				let zero = self.splat(0.0, Type::Vector3);

				self.add(
					hint,
					bemx::Node::Binary {
						operator: BinaryOperator::Subtract,
						left: zero,
						right: view,
					},
				)?
			}
		};

		self.geometry[geometry as usize] = Some(node);

		Ok(node)
	}
}

/// Returns the BEMX type that holds a MaterialX type, or reports that no BEMX type does.
///
/// Colours and vectors of the same width share one BEMX type, because MaterialX only tells them apart
/// to pick node overloads. Integers and booleans become floats, because BEMX computes in floats alone.
pub(super) fn bemx_type(data_type: DataType<'_>, hint: &str) -> Result<Type, ConvertError> {
	Ok(match data_type {
		DataType::Float | DataType::Integer | DataType::Boolean => Type::Float,
		DataType::Vector2 => Type::Vector2,
		DataType::Color3 | DataType::Vector3 => Type::Vector3,
		DataType::Color4 | DataType::Vector4 => Type::Vector4,
		_ => {
			return Err(ConvertError::UnsupportedType {
				node: hint.to_string(),
				data_type: data_type.name().to_string(),
			});
		}
	})
}
