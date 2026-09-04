//! Lower a resolved MaterialX graph into the BESL program that shades it.
//!
//! A `.mtlx` document describes a shading network; this module turns one of its materials into the
//! BESL syntax tree the shader pipeline compiles. Pass the [`Dag`] that
//! [`materialx::parse`](crate::materialx::parse) returned to [`lower`], hand [`Program::root`] to
//! the renderer's program generator, and bind [`Program::textures`] to the material's texture slots.
//!
//! # What the lowered program looks like
//!
//! The program is a `main` function that writes the material properties the renderer's BRDF reads:
//! `albedo`, `metalness`, `roughness`, `emission`, `occlusion` and `normal`. Every MaterialX node
//! the material reaches becomes one local, in the order the values have to be computed, so a node
//! that nothing reads is never emitted. Images become `sample_material` calls against the slots
//! listed in [`Program::textures`].
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
//! let allocator = &&arena;
//!
//! let dag = materialx::parse(source, allocator)?;
//! let program = materialx::besl::lower(&dag)?;
//!
//! // The program links against the shader pipeline's own declarations.
//! assert!(program.textures.is_empty());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # What this module covers
//!
//! Six shading models reach the renderer's BRDF: `standard_surface`, `open_pbr_surface`,
//! `UsdPreviewSurface`, `gltf_pbr`, `disney_principled` and `surface_unlit`. A model a document
//! defines itself works too, as long as its node graph produces one of those. Behind the model, the
//! shading network may use the standard library's value, channel, geometric, texture, math,
//! adjustment and compositing nodes, and any node graph the document defines, including one that
//! implements a node definition of its own.
//!
//! Anything else is reported rather than approximated, because a material that renders wrong is
//! harder to notice than one that fails to bake. A node graph the renderer cannot evaluate, such as
//! one built from individual scattering closures or one that samples a texture with transformed
//! coordinates, is best baked into a texture with the MaterialX tools first.
//!
//! Three properties of the surface differ from what MaterialX describes, because the material stage
//! carries less than a full renderer does:
//!
//! - Object-space and model-space geometric properties read their world-space value, since the stage
//!   reconstructs the shaded point in world space alone.
//! - Images sample with the mesh's own texture coordinates. An `<image>` whose `texcoord` is driven
//!   by anything else is reported instead of being sampled at the wrong place.
//! - Shading models take a world-space normal, which is projected onto the stage's tangent frame.
//!
//! Coat, sheen, transmission and subsurface inputs are read but not shaded, because the renderer's
//! BRDF has no parameter for them.
//!
//! # What is left to do
//!
//! Measured against the 198 documents of the upstream MaterialX resource library, 108 of their 355
//! materials lower today, and every one of those links. The three gaps below account for the rest,
//! in the order they are worth closing. They are also listed under "MaterialX lowering" in
//! `todo.md`.
//!
//! ## Scattering closures
//!
//! 215 of the remaining materials are a `<surface>` node fed by individual closures:
//! `oren_nayar_diffuse_bsdf`, `dielectric_bsdf`, `conductor_bsdf`, `generalized_schlick_bsdf`,
//! `uniform_edf`, and the `layer`, `mix` and `add` nodes that combine them. This is how the physically
//! based shading library writes a surface, and what `standard_surface` itself expands into, so it is
//! the single change that would most widen what bakes.
//!
//! Closing it means reducing a closure tree onto the renderer's metallic-roughness parameters: a
//! diffuse closure gives the base colour, a conductor closure sets metalness, the outermost specular
//! closure gives roughness, `layer` takes its base's colour under its top's roughness, and `mix`
//! blends both sides' parameters. Write it as its own pass over the closure subtree rather than as
//! more arms of `nodes.rs`, because the rules are approximations of a renderer's behaviour and belong
//! next to a comment saying which behaviour, unlike the value nodes, which are exact.
//!
//! ## Transformed texture coordinates
//!
//! 18 materials place, tile or rotate the coordinates an `<image>` samples with, which
//! [`LowerError::UnsupportedTextureCoordinates`] reports. It cannot be fixed in this module alone:
//! `sample_material(texture)` is expanded by the visibility shader generator, which appends
//! `vertex_uv` together with `uv_derivative_x` and `uv_derivative_y`. Passing a transformed
//! coordinate without transforming the derivatives to match picks the wrong mip level, which is a
//! quality bug that renders rather than fails.
//!
//! So this needs three things together: a `sample_material(texture, uv)` form the generator accepts,
//! derivatives carried alongside every lowered coordinate value, and a rule for what to do when a
//! coordinate comes from something with no analytic derivative. Until then, `place2d`, `tiledimage`
//! and `hextiledimage` are honestly reported.
//!
//! ## Standard library nodes written in source code
//!
//! The standard library defines most of its nodes as node graphs, which `nodes.rs` expands for free.
//! The rest are written as backend source code, so this module has to implement each one: the
//! procedural noises `noise2d`, `noise3d`, `fractal3d`, `cellnoise2d`, `worleynoise2d`, `flake2d`
//! and `flake3d`, the colour conversions `hsvtorgb`, `rgbtohsv` and `blackbody`, and the
//! coordinate transform `rotate2d`. These are 11 materials between them and each is independent, so
//! they can be added one at a time. Watch out for BESL's narrow intrinsic overloads while doing it:
//! most of them are registered for `f32` alone, which is why `nodes.rs` carries a `componentwise` helper.
//!
//! Next, give [`Program::root`] to the shader pipeline's
//! [`ProgramGenerator`](crate::asset::handler::implementations::bema::ProgramGenerator).

mod error;
mod lowering;
mod nodes;
mod surface;
mod syntax;

#[cfg(test)]
mod tests;

pub use error::LowerError;

use crate::materialx::{Dag, NodeId};
use lowering::Lowering;

/// The input a material reads its surface shader from.
const SURFACE_SHADER_INPUT: &str = "surfaceshader";

/// The `Texture` struct names one image a lowered program samples.
///
/// The renderer binds these in order, so the entry at index `n` is the texture the program reads
/// through its `material_texture_n` variable. The file is kept exactly as the document wrote it,
/// including any `<UDIM>` or `[token]` substitution and without its prefix joined on, because
/// resolving a reference against the document's location is the asset pipeline's decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Texture<'a> {
	/// The file the `<image>` node named.
	pub file: &'a str,
	/// The path the document asked to be prepended to the file, when it asked for one.
	pub file_prefix: Option<&'a str>,
	/// The colour space the document declared the file to be written in, when it declared one.
	pub colorspace: Option<&'a str>,
}

/// The `Program` struct holds one lowered material: its BESL syntax tree and the images it samples.
///
/// Pass [`Program::root`] to the shader pipeline's program generator, which adds the stage
/// interface and the bindings around it, and declare one `Texture2D` material variable per entry of
/// [`Program::textures`] so the sampling calls resolve to slots.
#[derive(Clone, Debug)]
pub struct Program<'a> {
	/// The BESL syntax tree, a root scope holding one `main` function.
	pub root: besl::parser::Node<'a>,
	/// The images the program samples, in the slot order the renderer binds them.
	pub textures: Vec<Texture<'a>>,
}

/// Lowers a document's first material into a BESL program.
///
/// Use this for a document that describes one material, which is what a baked asset holds. Call
/// [`lower_material`] instead to pick one material out of a document that carries several.
pub fn lower<'a>(dag: &Dag<'a>) -> Result<Program<'a>, LowerError> {
	let material = *dag.materials().first().ok_or(LowerError::NoMaterial)?;

	lower_material(dag, material)
}

/// Lowers one material of a document into a BESL program.
///
/// `material` names either a material node, whose surface shader is followed, or a surface shader
/// node directly. Take one from [`Dag::materials`].
pub fn lower_material<'a>(dag: &Dag<'a>, material: NodeId) -> Result<Program<'a>, LowerError> {
	let mut lowering = Lowering::new(dag);
	// A material written inside a node graph reads that graph's interface, so open the scopes around it.
	let scope = lowering.enter(dag.node(material).graph)?;
	// The material may itself be a definition, in which case the surface shader is inside it.
	let (scope, material) = lowering.resolve(scope, material)?;
	let instance = dag.node(material);

	let (frame, shader) = if surface::is_shading_model(instance.category) {
		(scope, material)
	} else {
		let port = instance
			.input(SURFACE_SHADER_INPUT)
			.ok_or_else(|| LowerError::MissingSurfaceShader {
				material: instance.name.to_string(),
			})?;

		lowering
			.shader(scope, &port.source)?
			.ok_or_else(|| LowerError::MissingSurfaceShader {
				material: instance.name.to_string(),
			})?
	};

	let assignments = surface::lower(&mut lowering, frame, shader, instance.name)?;

	// The values a property reads have to be computed before the property is written.
	let mut body = lowering.statements;
	body.extend(assignments);

	Ok(Program {
		root: besl::parser::Node::root_with_children(vec![besl::parser::Node::main_function(body)]),
		textures: lowering.textures,
	})
}
