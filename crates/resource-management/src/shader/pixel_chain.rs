//! Pixel chains: the `main` a bake generates to run one or more pixel shaders over each pixel of `main`, in order.
//!
//! A pixel shader is a `Tile` stage shader that declares `pixel` in place of `main`; see the
//! [pixel shader reference](/docs/reference/besl/interface#pixel-shaders). A chain
//! attaches each step's source as the module `step_<i>` of a generated wrapper, so the steps' declarations stay apart,
//! and renames each step's `pixel` to `step_<i>_pixel`, because backends emit every function by its own name.
//!
//! Bakes drive the chain in three parts: read the modules the [`declarations`] ask for, answering each query with the
//! file [`locate`] names; pass the steps and modules to [`wrapper`] for the generated source; then parse it with
//! [`program`]. [`assemble`] does all three for hosts that read files without waiting, such as tests.

use besl::{
	LoadedModule,
	parser::{Node, Nodes, TypeName},
};

/// The most steps one chain runs, which is how many bits of its step mask the renderer sets.
pub const MAX_STEPS: usize = 8;

/// The reference page that defines pixel shaders.
const PIXEL_SHADER_DOCS_PATH: &str = "reference/besl/interface#pixel-shaders";

/// The name each step's `pixel` takes once renamed, by step index. They are static so the renamed tree can borrow them.
const ENTRY_NAMES: [&str; MAX_STEPS] = [
	"step_0_pixel",
	"step_1_pixel",
	"step_2_pixel",
	"step_3_pixel",
	"step_4_pixel",
	"step_5_pixel",
	"step_6_pixel",
	"step_7_pixel",
];

/// The `ChainFile` enum names where a file a chain's module loader asks for lives.
#[derive(Debug, PartialEq, Eq)]
pub enum ChainFile<'q> {
	/// The source of the step at this index.
	Step(usize),
	/// A file next to a step's shader, at this path relative to the shader's directory.
	BesideStep(usize, &'q str),
	/// A file at this path from the asset root.
	Root(&'q str),
	/// A file that can't exist, such as a step written as a directory module.
	Missing,
}

/// Returns where `file`, one of the files a [`besl::ModuleQuery`] names, lives. `from_root` is the query's.
pub fn locate(file: &str, from_root: bool) -> ChainFile<'_> {
	if from_root {
		return ChainFile::Root(file);
	}
	// A step's own modules sit in the directory named after the step, as `step_<i>/<path>`.
	let (first, rest) = file.split_once('/').unwrap_or((file, ""));
	let Some(index) = first
		.strip_suffix(".besl")
		.unwrap_or(first)
		.strip_prefix("step_")
		.and_then(|index| index.parse::<usize>().ok())
	else {
		return ChainFile::Missing;
	};
	match rest {
		"" if first.ends_with(".besl") => ChainFile::Step(index),
		"" | "mod.besl" => ChainFile::Missing,
		rest => ChainFile::BesideStep(index, rest),
	}
}

/// Returns the declarations of a chain of `count` steps' modules, the source a [`besl::ModuleLoader`] starts from.
pub fn declarations(count: usize) -> String {
	(0..count).map(|index| format!("step_{index}: mod;\n")).collect()
}

/// Returns whether `program`, a parsed shader, is a pixel shader: one that declares `pixel` and no `main`.
pub fn is_pixel_shader(program: &Node<'_>) -> bool {
	let declares = |name| top_level(program).iter().any(|node| node.name() == Some(name));
	declares("pixel") && !declares("main")
}

/// Returns the source of the shader that runs `steps`, each the source of one pixel shader, in order. `modules` are the
/// modules the [`declarations`] of the chain loaded.
///
/// With more than one step, the shader runs step `i` only while bit `i` of its push constant `steps` is set. Next,
/// parse the result with [`program`].
pub fn wrapper(steps: &[&str], modules: &[LoadedModule]) -> Result<String, String> {
	if steps.is_empty() || steps.len() > MAX_STEPS {
		return Err(format!(
			"A pixel chain has {} steps. The most likely cause is a fusion that lists too few or too many step pipelines. List 1 to {MAX_STEPS} steps.",
			steps.len()
		));
	}
	let layers = steps
		.iter()
		.enumerate()
		.map(|(index, source)| step_layer(index, source, modules))
		.collect::<Result<Vec<_>, _>>()?;
	// Backends emit every descriptor by its own name, so no two steps' layers may share one.
	let descriptors = layers.iter().flatten().flat_map(|(_, names)| names).collect::<Vec<_>>();
	if let Some(name) = descriptors
		.iter()
		.enumerate()
		.find_map(|(index, name)| descriptors[..index].contains(name).then_some(name))
	{
		return Err(format!(
			"Two steps of one pixel chain take layers with the descriptor `{name}`. The most likely cause is a fusion of two steps that take the same layer, or of layers whose descriptors share a name. Rename the descriptor in one step's layer, or fuse steps that take different layers. See {}.",
			utils::online_docs_url("reference/assets#fusions")
		));
	}

	let paths = layers
		.into_iter()
		.map(|layer| layer.map(|(path, _)| path))
		.collect::<Vec<_>>();
	Ok(wrapper_source(&paths))
}

/// Writes the wrapper's source for steps that take `layers`, by index: the path of each step's layer type, if any.
fn wrapper_source(layers: &[Option<String>]) -> String {
	use std::fmt::Write as _;

	let masked = layers.len() > 1;
	let mut source = declarations(layers.len());
	source.push_str("\nsource: descriptor<{ type: Texture2D, binding: 0, access: read }>;\nresult: output<vec4f, 0>;\n");
	if masked {
		source.push_str("push_constant: push_constant {\n\tsteps: u32,\n}\n");
	}

	source.push_str("\nmain: fn (input: StageInput");
	for (index, layer) in layers.iter().enumerate() {
		if let Some(layer) = layer {
			let _ = write!(source, ", layer_{index}: {layer}");
		}
	}
	source.push_str(
		") -> void {\n\tlet coordinate: vec2u = input.thread_id;\n\tlet extent: vec2u = input.extent;\n\tif (coordinate.x >= extent.x || coordinate.y >= extent.y) {\n\t\treturn;\n\t}\n\tlet color: vec4f = fetch(source, coordinate);\n",
	);
	for (index, layer) in layers.iter().enumerate() {
		let layer = layer.as_ref().map_or(String::new(), |_| format!(", layer_{index}"));
		let call = format!(
			"color = step_{index}::{}(color, coordinate, extent{layer});",
			ENTRY_NAMES[index]
		);
		if masked {
			let _ = writeln!(
				source,
				"\tif ((push_constant.steps & {}) != 0) {{\n\t\t{call}\n\t}}",
				1u32 << index
			);
		} else {
			let _ = writeln!(source, "\t{call}");
		}
	}
	source.push_str("\twrite(result, coordinate, color);\n}\n");
	source
}

/// Parses `wrapper`, the source [`wrapper`] returned, attaches `modules`, and renames each step's `pixel`.
///
/// Next, apply the renderer's program transform and link the result.
pub fn program<'a>(wrapper: &'a str, modules: &'a [LoadedModule]) -> Result<Node<'a>, String> {
	let mut program = besl::parse(wrapper).map_err(|error| {
		format!("Failed to parse a generated pixel chain ({error:?}). The most likely cause is a bug in the chain generator.")
	})?;
	besl::attach_modules(&mut program, modules)
		.map_err(|error| format!("Failed to attach the pixel chain's BESL modules ({error:?})."))?;
	for (index, name) in ENTRY_NAMES.iter().enumerate() {
		let Some(step) = program.get_mut(&format!("step_{index}")) else {
			break;
		};
		let entry = step
			.get_mut("pixel")
			.expect("A step's `pixel` was checked when its wrapper was generated");
		if let Nodes::Function { name: entry, .. } = entry.node_mut() {
			*entry = name;
		}
	}
	Ok(program)
}

/// Generates the chain of `steps` and loads its modules through `read`, which returns a file's source by where
/// [`locate`] says it lives, or `None` when it doesn't exist.
///
/// Returns the wrapper's source and the modules; next, pass both to [`program`].
pub fn assemble(
	steps: &[&str],
	mut read: impl FnMut(ChainFile<'_>) -> Option<String>,
) -> Result<(String, Vec<LoadedModule>), String> {
	let failed = |error: besl::CompilationError| format!("Failed to load the pixel chain's BESL modules ({error:?}).");
	let mut loader = besl::ModuleLoader::new(&declarations(steps.len())).map_err(failed)?;
	while let Some(query) = loader.query() {
		let sources = query.files.each_ref().map(|file| match locate(file, query.from_root) {
			ChainFile::Step(index) => steps.get(index).map(|source| source.to_string()),
			ChainFile::Missing => None,
			located => read(located),
		});
		loader.answer(sources).map_err(failed)?;
	}
	let modules = loader.into_modules();
	let wrapper = wrapper(steps, &modules)?;
	Ok((wrapper, modules))
}

/// Returns the declarations at the top of a parsed file.
fn top_level<'n, 'a>(program: &'n Node<'a>) -> &'n [Node<'a>] {
	match program.node() {
		Nodes::Scope { children, .. } => children,
		_ => &[],
	}
}

/// Checks that step `index`, whose shader source is `source`, follows the pixel shader rules, and returns the path its
/// layer type has from the chain's wrapper with the names of the layer's descriptors, or `None` when it takes no layer.
fn step_layer(index: usize, source: &str, modules: &[LoadedModule]) -> Result<Option<(String, Vec<String>)>, String> {
	let invalid = |problem: &str, fix: &str| {
		format!(
			"Step {index} of a pixel chain {problem} The most likely cause is a step shader that doesn't follow the pixel shader rules. {fix} See {}.",
			utils::online_docs_url(PIXEL_SHADER_DOCS_PATH)
		)
	};
	let program = besl::parse(source).map_err(|error| format!("Failed to parse step {index} of a pixel chain ({error:?})."))?;
	let declarations = top_level(&program);

	if !is_pixel_shader(&program) {
		return Err(invalid("isn't a pixel shader.", "Declare `pixel` in place of `main`."));
	}
	if let Some(own) = declarations.iter().find(|node| {
		matches!(
			node.node(),
			Nodes::Binding { .. }
				| Nodes::Descriptor { .. }
				| Nodes::Output { .. }
				| Nodes::PushConstant { .. }
				| Nodes::Workgroup { .. }
		)
	}) {
		return Err(invalid(
			&format!("declares `{}` outside a layer.", own.name().unwrap_or("push_constant")),
			"Move the resource into the layer `pixel` takes.",
		));
	}

	let Some(Nodes::Function { params, return_type, .. }) =
		declarations.iter().find(|node| node.name() == Some("pixel")).map(Node::node)
	else {
		return Err(invalid(
			"declares `pixel` as something other than a function.",
			"Declare `pixel: fn (...)`.",
		));
	};
	let types = params
		.iter()
		.map(|parameter| match parameter.node() {
			Nodes::Parameter {
				r#type: TypeName::Named(name),
				..
			} => Some(*name),
			_ => None,
		})
		.collect::<Vec<_>>();
	let signature =
		"Declare `pixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u, layer: Layer) -> vec4f`, with the layer optional.";
	let shaped = matches!(
		types.as_slice(),
		[Some("vec4f"), Some("vec2u"), Some("vec2u")] | [Some("vec4f"), Some("vec2u"), Some("vec2u"), Some(_)]
	);
	if !shaped || !matches!(return_type, TypeName::Named("vec4f")) {
		return Err(invalid(
			"declares `pixel` with the wrong parameters or return type.",
			signature,
		));
	}
	let Some(Some(layer)) = types.get(3) else {
		return Ok(None);
	};

	let (path, members) = resolve_layer(index, layer, declarations, modules).ok_or_else(|| {
		invalid(
			&format!("takes `{layer}`, which isn't a layer it declares or brings in from the asset root."),
			"Declare the layer in the step's shader, name it with a path from the asset root, or `use` the module from the asset root that declares it.",
		)
	})?;
	match members {
		LayerMembers::Descriptors(names) => Ok(Some((path, names))),
		LayerMembers::Empty => Err(invalid(
			&format!("takes `{layer}`, which has no descriptors."),
			"Add the step's descriptors to the layer, or leave the layer out of `pixel`.",
		)),
		LayerMembers::Other(member) => Err(invalid(
			&format!("takes `{layer}`, whose member `{member}` isn't a descriptor named something other than `source`."),
			"Keep only descriptors in the layer, and rename any named `source`.",
		)),
	}
}

/// The `LayerMembers` enum sorts a step's layer by what it holds, which decides whether a chain can take it.
enum LayerMembers {
	/// One or more descriptors, none named `source`, by name.
	Descriptors(Vec<String>),
	Empty,
	/// A member a step's layer can't hold, by name.
	Other(String),
}

/// Finds the layer named `layer` that step `index` takes, among the step's `declarations` and the modules from the asset
/// root it brings in, and returns its path from the chain's wrapper and what it holds.
fn resolve_layer(
	index: usize,
	layer: &str,
	declarations: &[Node<'_>],
	modules: &[LoadedModule],
) -> Option<(String, LayerMembers)> {
	// A path from the asset root names the module in front of its last segment.
	if let Some(path) = layer.strip_prefix("::") {
		let (module, name) = path.rsplit_once("::")?;
		return members_in_module(module, name, modules).map(|members| (layer.to_string(), members));
	}
	if let Some(members) = layer_members(declarations, layer) {
		return Some((format!("step_{index}::{layer}"), members));
	}
	declarations.iter().find_map(|node| {
		let Nodes::Use { path, glob } = node.node() else {
			return None;
		};
		let path = path.strip_prefix("::")?;
		let (module, name) = if *glob {
			(path, layer)
		} else {
			let (module, name) = path.rsplit_once("::")?;
			(name == layer).then_some((module, name))?
		};
		members_in_module(module, name, modules).map(|members| (format!("::{module}::{name}"), members))
	})
}

/// Returns what the layer `name` that the module from the asset root `module` declares holds, if it declares one.
fn members_in_module(module: &str, name: &str, modules: &[LoadedModule]) -> Option<LayerMembers> {
	let loaded = modules
		.iter()
		.find(|loaded| loaded.path.is_empty() && loaded.root.as_deref() == Some(module))?;
	let parsed = besl::parse(&loaded.source).ok()?;
	layer_members(top_level(&parsed), name)
}

/// Returns what the layer `name` among `declarations` holds, or `None` when none of them is that layer.
fn layer_members(declarations: &[Node<'_>], name: &str) -> Option<LayerMembers> {
	declarations.iter().find_map(|node| match node.node() {
		Nodes::Layer { name: declared, members } if *declared == name => Some(
			match members.iter().find(|(member, node)| {
				*member == "source" || !matches!(node.node(), Nodes::Descriptor { .. } | Nodes::Binding { .. })
			}) {
				Some((member, _)) => LayerMembers::Other(member.to_string()),
				None if members.is_empty() => LayerMembers::Empty,
				None => LayerMembers::Descriptors(members.iter().map(|(member, _)| member.to_string()).collect()),
			},
		),
		_ => None,
	})
}

#[cfg(test)]
mod tests {
	use super::{ChainFile, assemble, locate, program};

	const LUT_LAYOUT: &str = r#"
LutParameters: struct { strength: vec4f, }
Lut: layer {
	lut_texture: descriptor<{ type: Texture3D, binding: 0, access: read }>;
	parameters: descriptor<{ type: LutParameters, binding: 1, access: read, memory: constant }>;
}
"#;

	const LUT_STEP: &str = r#"
use ::engine::layouts::lut::*;
pixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u, lut: Lut) -> vec4f {
	let looked: vec4f = texture_lod(lut.lut_texture, vec3f(color.x, color.y, color.z));
	return color + (looked - color) * lut.parameters.strength.x;
}
"#;

	const GRAIN_STEP: &str = r#"
Grain: layer {
	grain: descriptor<{ type: Texture2D, binding: 0, access: read }>;
}
pixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u, grain: Grain) -> vec4f {
	return color + fetch(grain.grain, coordinate);
}
"#;

	const HALVE_STEP: &str = "pixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u) -> vec4f { return color * 0.5; }";

	/// Reads the chain's files from the fixtures: the LUT layout from the asset root and nothing beside the steps.
	fn read(file: ChainFile<'_>) -> Option<String> {
		match file {
			ChainFile::Root("engine/layouts/lut.besl") => Some(LUT_LAYOUT.to_string()),
			_ => None,
		}
	}

	/// Assembles and links a chain of `steps`, returning the linked program or the first error.
	fn link(steps: &[&str]) -> Result<besl::NodeReference, String> {
		let (wrapper, modules) = assemble(steps, read)?;
		let program = program(&wrapper, &modules)?;
		besl::lex(program).map_err(|error| format!("{error:?}"))
	}

	#[test]
	fn files_beside_a_step_resolve_in_its_directory() {
		assert_eq!(locate("step_2.besl", false), ChainFile::Step(2));
		assert_eq!(locate("step_2/mod.besl", false), ChainFile::Missing);
		assert_eq!(locate("step_2/noise.besl", false), ChainFile::BesideStep(2, "noise.besl"));
		assert_eq!(
			locate("step_2/noise/mod.besl", false),
			ChainFile::BesideStep(2, "noise/mod.besl")
		);
		assert_eq!(locate("engine/layouts.besl", true), ChainFile::Root("engine/layouts.besl"));
	}

	#[test]
	fn steps_with_layers_from_the_root_and_their_own_and_none_link_as_one_shader() {
		let program = link(&[LUT_STEP, GRAIN_STEP, HALVE_STEP, HALVE_STEP]).expect("the chain should link");
		assert!(program.get_main().is_some(), "the chain should have a generated `main`");
	}

	#[test]
	fn one_step_links_without_a_step_mask() {
		let (wrapper, _) = assemble(&[HALVE_STEP], read).expect("one step should assemble");
		assert!(!wrapper.contains("push_constant"), "one step needs no step mask:\n{wrapper}");
		link(&[HALVE_STEP]).expect("one step should link");
	}

	#[test]
	fn a_shader_with_main_is_not_a_step() {
		let error = link(&["main: fn (input: StageInput) -> void {}"]).expect_err("a `main` shader isn't a step");
		assert!(error.contains("isn't a pixel shader"), "{error}");
	}

	#[test]
	fn a_step_with_its_own_descriptor_is_rejected() {
		let step = "lut: descriptor<{ type: Texture3D, binding: 0, access: read }>;\npixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u) -> vec4f { return color; }";
		let error = link(&[step]).expect_err("a descriptor outside a layer can't be fused");
		assert!(error.contains("outside a layer"), "{error}");
	}

	#[test]
	fn a_step_with_the_wrong_signature_is_rejected() {
		let step = "pixel: fn (color: vec4f) -> vec4f { return color; }";
		let error = link(&[step]).expect_err("a step must take its pixel and extent");
		assert!(error.contains("wrong parameters"), "{error}");
	}

	#[test]
	fn two_steps_that_take_one_layer_are_rejected() {
		let error = link(&[LUT_STEP, LUT_STEP]).expect_err("two steps can't bind one layer");
		assert!(error.contains("descriptor `lut_texture`"), "{error}");
	}

	#[test]
	fn layers_whose_descriptors_share_a_name_are_rejected() {
		let step = "Own: layer {\n\tgrain: descriptor<{ type: Texture2D, binding: 0, access: read }>;\n}\npixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u, own: Own) -> vec4f { return color; }";
		let error = link(&[GRAIN_STEP, step]).expect_err("backends emit both descriptors by one name");
		assert!(error.contains("descriptor `grain`"), "{error}");
	}

	#[test]
	fn a_layer_with_a_descriptor_named_source_is_rejected() {
		let step = "Own: layer {\n\tsource: descriptor<{ type: Texture2D, binding: 0, access: read }>;\n}\npixel: fn (color: vec4f, coordinate: vec2u, extent: vec2u, own: Own) -> vec4f { return color; }";
		let error = link(&[step]).expect_err("`source` belongs to the chain");
		assert!(error.contains("`source`"), "{error}");
	}
}
