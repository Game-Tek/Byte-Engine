//! Writes the body of a material-evaluation shader: the statements that compute a BEMX surface and hand it to the
//! lighting suffix.

use besl::parser::Node;
use resource_management::asset::handler::implementations::bema::{MaterialParameter, ParameterKind};
use resource_management::bemx::{
	self,
	besl::{Expression, Geometry, LowerError, Lowering, Stage, component, construct},
};

use super::ast::MaterialReconstructionFeatures;

/// The `MaterialStage` struct supplies what a BEMX graph reads from the pixel material evaluation reconstructs, and
/// records which optional attributes it reached.
///
/// The prefix only reconstructs texture coordinates and the tangent frame for a material that reads them, so the
/// stage notes every read while the graph is lowered.
struct MaterialStage<'a> {
	parameters: &'a [MaterialParameter],
	features: MaterialReconstructionFeatures,
}

impl<'a> MaterialStage<'a> {
	/// Returns a `vec3f` reading the half-precision tangent-frame axis `axis`.
	fn tangent_frame_axis(&mut self, axis: &'static str) -> Node<'a> {
		self.features.reads_tangent_frame = true;

		Node::call("vec3f", vec![Node::member_expression(axis)])
	}
}

impl<'a> Stage<'a> for MaterialStage<'a> {
	fn geometry(&mut self, geometry: Geometry) -> Option<Node<'a>> {
		Some(match geometry {
			Geometry::Position => Node::member_expression("world_space_vertex_position"),
			Geometry::Normal => Node::member_expression("N"),
			Geometry::View => Node::member_expression("V"),
			Geometry::Tangent => self.tangent_frame_axis("T"),
			Geometry::Bitangent => self.tangent_frame_axis("B"),
			Geometry::TexCoord => {
				self.features.reads_uv = true;

				Node::member_expression("vertex_uv")
			}
		})
	}

	fn sample(&mut self, texture: &'a str) -> Result<Node<'a>, LowerError> {
		let slot = texture_slot(self.parameters, texture)?;

		self.features.reads_uv = true;

		Ok(Node::call(
			"sample_texture_2d_array_grad",
			vec![
				scene_member("textures"),
				material_texture(Node::member_expression("material"), slot),
				Node::member_expression("vertex_uv"),
				Node::member_expression("uv_derivative_x"),
				Node::member_expression("uv_derivative_y"),
			],
		))
	}

	fn parameter(&mut self, name: &'a str, r#type: bemx::Type) -> Result<Node<'a>, LowerError> {
		parameter_read(
			self.parameters,
			name,
			r#type,
			Node::accessor(Node::member_expression("mesh"), Node::member_expression("material")),
		)
	}
}

/// The `CoverageStage` struct supplies what a BEMX graph's `opacity` reads in the masked raster passes, whose fragments
/// carry only the vertex UV.
///
/// The coverage prefix declares `vertex_uv`, the instance's `material_word`, and its `material_index`, and the stage
/// reads the material table entry's fields one at a time instead of copying the entry.
struct CoverageStage<'a> {
	parameters: &'a [MaterialParameter],
}

impl<'a> Stage<'a> for CoverageStage<'a> {
	fn geometry(&mut self, geometry: Geometry) -> Option<Node<'a>> {
		match geometry {
			Geometry::TexCoord => Some(Node::member_expression("vertex_uv")),
			Geometry::Position | Geometry::Normal | Geometry::Tangent | Geometry::Bitangent | Geometry::View => None,
		}
	}

	fn sample(&mut self, texture: &'a str) -> Result<Node<'a>, LowerError> {
		let slot = texture_slot(self.parameters, texture)?;
		let material = Node::accessor(
			Node::accessor(scene_member("materials"), Node::member_expression("materials")),
			Node::sentence(vec![Node::member_expression("material_index")]),
		);
		// A raster fragment samples with the derivatives the rasterizer computes.
		Ok(Node::call(
			"sample",
			vec![
				Node::accessor(
					scene_member("textures"),
					Node::sentence(vec![material_texture(material, slot)]),
				),
				Node::member_expression("vertex_uv"),
			],
		))
	}

	fn parameter(&mut self, name: &'a str, r#type: bemx::Type) -> Result<Node<'a>, LowerError> {
		parameter_read(self.parameters, name, r#type, Node::member_expression("material_word"))
	}
}

/// Returns the slot of texture parameter `texture` in its material's table entry.
///
/// Texture parameters take slots in the order the material declares them, which is the order its variant binds images
/// to `material.textures`.
fn texture_slot(parameters: &[MaterialParameter], texture: &str) -> Result<usize, LowerError> {
	parameters
		.iter()
		.filter(|parameter| parameter.kind == ParameterKind::Texture)
		.position(|parameter| parameter.name == texture)
		.ok_or_else(|| LowerError::UnknownTexture {
			name: texture.to_string(),
		})
}

/// Returns a read of the bindless texture index in slot `slot` of the material table entry `material`.
fn material_texture<'a>(material: Node<'a>, slot: usize) -> Node<'a> {
	// Index access keeps an expression on its right side so the linked AST can distinguish `textures[slot]` from
	// `.field`.
	let material_textures = Node::accessor(material, Node::member_expression("textures"));
	Node::accessor(
		material_textures,
		Node::sentence(vec![Node::literal_expression(format!("{slot}u"))]),
	)
}

/// Returns a read of the value parameter `name` of type `type`: the parameter's own name for a static one, which the
/// generator declares, or the instance's entry for a dynamic one. `material_word` is the instance record's
/// `material` word, whose high half names where its parameter block starts.
fn parameter_read<'a>(
	parameters: &[MaterialParameter],
	name: &'a str,
	r#type: bemx::Type,
	material_word: Node<'a>,
) -> Result<Node<'a>, LowerError> {
	// Dynamic parameters take entries of the instance's parameter block in the order the material declares them,
	// which is the order the renderer writes their values in.
	let mut entry = 0u32;
	for parameter in parameters {
		match parameter.kind {
			ParameterKind::Value(declared) if parameter.name == name && declared == r#type => {
				return Ok(Node::member_expression(name));
			}
			ParameterKind::Dynamic(declared) if parameter.name == name && declared == r#type => {
				let block = Node::operator(">>", material_word, Node::literal_expression("16u"));
				let index = Node::operator("+", block, Node::literal_expression(format!("{entry}u")));
				let value = Node::accessor(scene_member("material_parameters"), Node::sentence(vec![index]));

				return Ok(construct(
					(0..r#type.width()).map(|index| component(&value, index, 4)).collect(),
				));
			}
			ParameterKind::Dynamic(_) => entry += 1,
			_ => {}
		}
	}

	Err(LowerError::UnknownParameter {
		name: name.to_string(),
		r#type,
	})
}

/// Returns the statements that compute `graph`'s `opacity` into the `f32` local `opacity`, which the coverage prefix
/// declares, for the masked raster passes.
///
/// Only the nodes `opacity` reaches are lowered. A graph whose `opacity` reads geometry other than texture coordinates
/// fails with [`LowerError::UnavailableGeometry`].
pub(super) fn coverage_body<'a>(
	graph: &'a bemx::Graph,
	parameters: &'a [MaterialParameter],
) -> Result<Vec<Node<'a>>, LowerError> {
	let mut lowering = Lowering::new(graph, CoverageStage { parameters });
	let opacity = match graph.surface().opacity {
		Some(opacity) => lowering.value(opacity)?.syntax,
		None => bemx::besl::constant(&bemx::Surface::DEFAULT_OPACITY),
	};
	let (mut statements, _) = lowering.finish();
	statements.push(Node::member_assignment("opacity", opacity));
	Ok(statements)
}

/// Returns the statements that compute `graph`'s surface and assign it to the material outputs the lighting suffix
/// reads, with the pixel attributes they need reconstructed.
///
/// The outputs are the half-precision locals the prefix declares: `albedo`, `metalness`, `roughness`, `occlusion`,
/// `emission`, and `normal` in the tangent frame. Every output but the normal is written, with BEMX's default for a
/// surface input the graph leaves out, so the prefix's own defaults never decide what a material shades with. Each
/// output's statements come right before its assignment, so a sampled texel lives no longer than its first use needs.
pub(super) fn material_body<'a>(
	graph: &'a bemx::Graph,
	parameters: &'a [MaterialParameter],
) -> Result<(Vec<Node<'a>>, MaterialReconstructionFeatures), LowerError> {
	let surface = *graph.surface();
	let mut lowering = Lowering::new(
		graph,
		MaterialStage {
			parameters,
			features: MaterialReconstructionFeatures::default(),
		},
	);
	let mut statements = Vec::with_capacity(graph.nodes().len() + 6);

	let base_color = value_or_default(&mut lowering, surface.base_color, bemx::Surface::DEFAULT_BASE_COLOR)?;
	let base_color = lowering.addressable(base_color);
	let opacity = value_or_default(&mut lowering, surface.opacity, bemx::Surface::DEFAULT_OPACITY)?;
	let albedo = Node::call(
		"vec4f",
		vec![
			base_color.component(0),
			base_color.component(1),
			base_color.component(2),
			opacity.syntax,
		],
	);
	assign(&mut statements, &mut lowering, "albedo", "vec4f16", albedo);

	for (name, narrow, input, default) in [
		("metalness", "f16", surface.metalness, bemx::Surface::DEFAULT_METALNESS),
		("roughness", "f16", surface.roughness, bemx::Surface::DEFAULT_ROUGHNESS),
		("occlusion", "f16", surface.occlusion, bemx::Surface::DEFAULT_OCCLUSION),
		("emission", "vec3f16", surface.emission, bemx::Surface::DEFAULT_EMISSION),
	] {
		let value = value_or_default(&mut lowering, input, default)?;
		assign(&mut statements, &mut lowering, name, narrow, value.syntax);
	}

	if let Some(normal) = surface.normal {
		let tangent = match lowering.tangent_normal(normal)? {
			// A normal map already holds the normal in the tangent frame the suffix rotates it out of.
			Some(tangent) => tangent.syntax,
			// Any other normal is in world space; the frame is orthonormal, so projecting onto it is exact.
			None => {
				let world = lowering.value(normal)?;
				let world = lowering.addressable(world);
				let tangent = lowering.stage().tangent_frame_axis("T");
				let bitangent = lowering.stage().tangent_frame_axis("B");
				let geometric = Node::member_expression("N");

				Node::call(
					"vec3f",
					[tangent, bitangent, geometric]
						.into_iter()
						.map(|axis| Node::call("dot", vec![world.syntax.clone(), axis]))
						.collect(),
				)
			}
		};

		lowering.stage().features.writes_normal = true;
		assign(&mut statements, &mut lowering, "normal", "vec3f16", tangent);
	}

	let (remaining, stage) = lowering.finish();
	statements.extend(remaining);

	Ok((statements, stage.features))
}

/// Returns the statements that compute what a ray tracing hit reads of `graph`'s surface, with the hit attributes they
/// need reconstructed.
///
/// The outputs are the half-precision locals the hit prefix declares: `albedo`, the base color, `metalness`, and
/// `emission`. Rays carry only diffuse light, so the hit leaves out the normal, roughness, occlusion, and opacity, and
/// lowers none of the nodes only those read.
pub(super) fn material_hit_body<'a>(
	graph: &'a bemx::Graph,
	parameters: &'a [MaterialParameter],
) -> Result<(Vec<Node<'a>>, MaterialReconstructionFeatures), LowerError> {
	let surface = *graph.surface();
	let mut lowering = Lowering::new(
		graph,
		MaterialStage {
			parameters,
			features: MaterialReconstructionFeatures::default(),
		},
	);
	let mut statements = Vec::with_capacity(graph.nodes().len() + 3);

	let base_color = value_or_default(&mut lowering, surface.base_color, bemx::Surface::DEFAULT_BASE_COLOR)?;
	let base_color = lowering.addressable(base_color);
	let color = Node::call("vec3f", (0..3).map(|index| base_color.component(index)).collect());
	assign(&mut statements, &mut lowering, "albedo", "vec3f16", color);
	for (name, narrow, input, default) in [
		("metalness", "f16", surface.metalness, bemx::Surface::DEFAULT_METALNESS),
		("emission", "vec3f16", surface.emission, bemx::Surface::DEFAULT_EMISSION),
	] {
		let value = value_or_default(&mut lowering, input, default)?;
		assign(&mut statements, &mut lowering, name, narrow, value.syntax);
	}

	let (remaining, stage) = lowering.finish();
	statements.extend(remaining);

	Ok((statements, stage.features))
}

/// Returns the value of a surface input, or the constant `default` when the graph leaves it out.
fn value_or_default<'a>(
	lowering: &mut Lowering<'a, MaterialStage<'a>>,
	input: Option<bemx::NodeId>,
	default: bemx::Value,
) -> Result<Expression<'a>, LowerError> {
	match input {
		Some(input) => lowering.value(input),
		None => Ok(Expression::new(bemx::besl::constant(&default), default.r#type())),
	}
}

/// Appends the statements written so far, then assigns `value`, narrowed to the compact evaluation type `narrow`, to
/// the output `name`.
fn assign<'a>(
	statements: &mut Vec<Node<'a>>,
	lowering: &mut Lowering<'a, MaterialStage<'a>>,
	name: &'static str,
	narrow: &'static str,
	value: Node<'a>,
) {
	lowering.drain_statements_into(statements);
	statements.push(Node::member_assignment(name, Node::call(narrow, vec![value])));
}

/// Returns a read of the scene layer's member `name` through the `scene` parameter every generated `main` takes.
fn scene_member<'a>(name: &'a str) -> Node<'a> {
	Node::accessor(Node::member_expression("scene"), Node::member_expression(name))
}
