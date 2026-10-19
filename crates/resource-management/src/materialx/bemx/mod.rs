//! Convert a resolved MaterialX graph into the BEMX graph that shades it.
//!
//! A `.mtlx` document describes a shading network; this module turns one of its materials into a
//! [BEMX](crate::bemx) graph, the material expression every renderer reads. Pass the [`Dag`] that
//! [`materialx::parse`](crate::materialx::parse) returned to [`convert`], hand [`Conversion::graph`]
//! to the renderer, and bind [`Conversion::textures`] to the texture parameters the graph samples.
//!
//! # What the converted graph looks like
//!
//! The graph ends in one metallic-roughness [`Surface`](crate::bemx::Surface) that carries the
//! shading model's base colour, opacity, metalness, roughness, emission, occlusion and, when a
//! shading network drives it, normal. Every MaterialX node the material reaches becomes one BEMX node
//! per instantiation of the scope it lives in, so a node that nothing reads is left out. Images
//! become [`Image`](crate::bemx::Node::Image) nodes that sample the texture parameter
//! `material_texture_n`, where `n` is the image's position in [`Conversion::textures`].
//!
//! ```
//! use resource_management::{bemx, materialx};
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
//! let allocator = &&arena;
//!
//! let dag = materialx::parse(source, allocator)?;
//! let conversion = materialx::bemx::convert(&dag)?;
//! let graph = &conversion.graph;
//!
//! // The shading model's inputs reach the surface the graph ends in.
//! let metalness = graph.surface().metalness.unwrap();
//! assert_eq!(graph.node(metalness), &bemx::Node::Constant(bemx::Value::Float(1.0)));
//! assert!(conversion.textures.is_empty());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # What this module covers
//!
//! Six shading models reduce onto a BEMX surface: `standard_surface`, `open_pbr_surface`,
//! `UsdPreviewSurface`, `gltf_pbr`, `disney_principled` and `surface_unlit`. A model a document
//! defines itself works too, as long as its node graph produces one of those. Behind the model, the
//! shading network may use the standard library's value, channel, geometric, texture, math,
//! adjustment and compositing nodes, and any node graph the document defines, including one that
//! implements a node definition of its own. Each of those maps onto the BEMX node of the same
//! meaning; integers and booleans become floats, because BEMX computes in floats alone.
//!
//! Anything else is reported rather than approximated, because a material that renders wrong is
//! harder to notice than one that fails to bake. A node graph BEMX cannot describe, such as one
//! built from individual scattering closures or one that samples a texture with transformed
//! coordinates, is best baked into a texture with the MaterialX tools first. A network whose node
//! types disagree is reported as [`ConvertError::Graph`], naming the MaterialX node.
//!
//! Three properties of the graph differ from what MaterialX describes:
//!
//! - Object-space and model-space geometric properties read their world-space value, since BEMX
//!   describes geometry in world space alone.
//! - Images sample with the mesh's own texture coordinates. An `<image>` whose `texcoord` is driven
//!   by anything else is reported as [`ConvertError::UnsupportedTextureCoordinates`] instead of being
//!   sampled at the wrong place.
//! - A `texcoord` node that reads a set other than zero converts to a
//!   [`TexCoord`](crate::bemx::Node::TexCoord) node of that set, which a renderer that carries set
//!   zero alone reports when it lowers the graph.
//!
//! Coat, sheen, transmission and subsurface inputs are not converted, because a BEMX surface has no
//! input for them.
//!
//! # What is left to do
//!
//! Measured against the 198 documents of the upstream MaterialX resource library, 108 of their 355
//! materials converted when this module still lowered to BESL directly; conversion accepts the same
//! nodes. The three gaps below account for the rest, in the order they are worth closing. They are
//! also listed under "MaterialX conversion" in `todo.md`.
//!
//! ## Scattering closures
//!
//! 215 of the remaining materials are a `<surface>` node fed by individual closures:
//! `oren_nayar_diffuse_bsdf`, `dielectric_bsdf`, `conductor_bsdf`, `generalized_schlick_bsdf`,
//! `uniform_edf`, and the `layer`, `mix` and `add` nodes that combine them. This is how the physically
//! based shading library writes a surface, and what `standard_surface` itself expands into, so it is
//! the single change that would most widen what bakes.
//!
//! Closing it means reducing a closure tree onto the inputs of a BEMX
//! [`Surface`](crate::bemx::Surface): a diffuse closure gives the base colour, a conductor closure
//! sets metalness, the outermost specular closure gives roughness, `layer` takes its base's colour
//! under its top's roughness, and `mix` blends both sides' inputs with BEMX mix nodes. Write it as
//! its own pass over the closure subtree rather than as more arms of `nodes.rs`, because the rules
//! are approximations of a renderer's behaviour and belong next to a comment saying which behaviour,
//! unlike the value nodes, which are exact.
//!
//! ## Transformed texture coordinates
//!
//! 18 materials place, tile or rotate the coordinates an `<image>` samples with, which
//! [`ConvertError::UnsupportedTextureCoordinates`] reports. BEMX can already describe them: an
//! [`Image`](crate::bemx::Node::Image) node takes a `texcoord` input. What is missing is a renderer
//! that honours it. The visibility shader generator samples every material texture with `vertex_uv`
//! together with `uv_derivative_x` and `uv_derivative_y`, and passing a transformed coordinate
//! without transforming the derivatives to match picks the wrong mip level, which is a quality bug
//! that renders rather than fails.
//!
//! So this needs the generator to accept a coordinate per sample, derivatives carried alongside
//! every coordinate value, and a rule for what to do when a coordinate comes from something with no
//! analytic derivative. Once it does, pass the converted coordinate as the image node's `texcoord`.
//! Until then, `place2d`, `tiledimage` and `hextiledimage` are honestly reported.
//!
//! ## Standard library nodes written in source code
//!
//! The standard library defines most of its nodes as node graphs, which `nodes.rs` expands for free.
//! The rest are written as backend source code, so BEMX needs a node for each one: the procedural
//! noises `noise2d`, `noise3d`, `fractal3d`, `cellnoise2d`, `worleynoise2d`, `flake2d` and
//! `flake3d`, the colour conversions `hsvtorgb`, `rgbtohsv` and `blackbody`, and the coordinate
//! transform `rotate2d`. These are 11 materials between them and each is independent, so they can be
//! added one at a time: add the node to [`bemx::Node`], lower it in
//! [`bemx::besl`], and map the category onto it here.
//!
//! Next, give [`Conversion::graph`] to a renderer's
//! [`ProgramGenerator`](crate::asset::handler::implementations::bema::ProgramGenerator), or lower it
//! yourself with [`bemx::besl::Lowering`].

mod conversion;
mod error;
mod nodes;
mod surface;

#[cfg(test)]
mod tests;

use conversion::Converter;
pub use error::ConvertError;

use crate::bemx::{self, Node};
use crate::materialx::{Dag, NodeId};

/// The input a material reads its surface shader from.
const SURFACE_SHADER_INPUT: &str = "surfaceshader";

/// The `Texture` struct names one image a converted graph samples.
///
/// The renderer binds these in order, so the entry at index `n` is the texture the graph's image
/// nodes read through the texture parameter `material_texture_n`. The file is kept exactly as the
/// document wrote it, including any `<UDIM>` or `[token]` substitution and without its prefix joined
/// on, because resolving a reference against the document's location is the asset pipeline's
/// decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Texture<'a> {
	/// The file the `<image>` node named.
	pub file: &'a str,
	/// The path the document asked to be prepended to the file, when it asked for one.
	pub file_prefix: Option<&'a str>,
	/// The colour space the document declared the file to be written in, when it declared one.
	pub colorspace: Option<&'a str>,
}

/// The `Conversion` struct holds one converted material: its BEMX graph and the images it samples.
///
/// Give [`Conversion::graph`] to a renderer's program generator, and declare one texture parameter
/// per entry of [`Conversion::textures`] so the graph's image nodes resolve to slots.
#[derive(Clone, Debug)]
pub struct Conversion<'a> {
	/// The material's BEMX graph.
	pub graph: bemx::Graph,
	/// The images the graph samples, in slot order: entry `n` is the texture parameter `material_texture_n`.
	pub textures: Vec<Texture<'a>>,
}

/// Converts a document's first material into a BEMX graph.
///
/// Use this for a document that describes one material, which is what a baked asset holds. Call
/// [`convert_material`] instead to pick one material out of a document that carries several.
pub fn convert<'a>(dag: &Dag<'a>) -> Result<Conversion<'a>, ConvertError> {
	let material = *dag.materials().first().ok_or(ConvertError::NoMaterial)?;

	convert_material(dag, material)
}

/// Converts one material of a document into a BEMX graph.
///
/// `material` names either a material node, whose surface shader is followed, or a surface shader
/// node directly. Take one from [`Dag::materials`].
pub fn convert_material<'a>(dag: &Dag<'a>, material: NodeId) -> Result<Conversion<'a>, ConvertError> {
	let mut converter = Converter::new(dag);
	// A material written inside a node graph reads that graph's interface, so open the scopes around it.
	let scope = converter.enter(dag.node(material).graph)?;
	// The material may itself be a definition, in which case the surface shader is inside it.
	let (scope, material) = converter.resolve(scope, material)?;
	let instance = dag.node(material);

	let (frame, shader) = if surface::is_shading_model(instance.category) {
		(scope, material)
	} else {
		let port = instance
			.input(SURFACE_SHADER_INPUT)
			.ok_or_else(|| ConvertError::MissingSurfaceShader {
				material: instance.name.to_string(),
			})?;

		converter
			.shader(scope, &port.source)?
			.ok_or_else(|| ConvertError::MissingSurfaceShader {
				material: instance.name.to_string(),
			})?
	};

	let surface = surface::convert(&mut converter, frame, shader, instance.name)?;
	let surface = converter.add(instance.name, Node::Surface(surface))?;
	let (builder, textures) = converter.finish();
	let graph = builder.finish(surface).map_err(|error| ConvertError::Graph {
		node: instance.name.to_string(),
		error,
	})?;

	Ok(Conversion { graph, textures })
}
