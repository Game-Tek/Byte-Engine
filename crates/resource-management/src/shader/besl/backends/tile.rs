//! The parts of the `Tile` stage that every platform shares: the interface a tile shader reflects, the rules its
//! outputs follow, and the rewrite that runs it as a compute shader where the platform has no tile functions.
//!
//! A tile shader declares its results as `output<vec4f, LOCATION>` and writes them with `write(output, pixel, value)`
//! inside the tile its invocation runs in. On Metal those outputs become color attachments of the render pass that
//! dispatches the tile function. GLSL and HLSL have no tile functions, so [`lower_for`] rewrites the shader into a
//! compute shader whose outputs are write-only storage images in the [`TILE_OUTPUTS_LAYOUT`] set, before reflection,
//! and the ordinary compute lowering does the rest.

use super::platform::PlatformShaderLanguage;
use crate::resources::material::{TileInterface, TileOutput};
use crate::shader::generator::Stages;

/// The page section that specifies tile shaders, which errors about them link to.
pub(crate) const TILE_ENTRY_DOCS_PATH: &str = "reference/besl/interface#tile-entry-points";

/// The layout name of the descriptor set that holds a tile shader's outputs on platforms that run it as compute.
pub const TILE_OUTPUTS_LAYOUT: &str = "tile_outputs";

/// Returns how many bytes one value of `besl_type` takes in a Metal tile function's threadgroup memory, where
/// three-component vectors take the space of four, or `None` for a type a tile shader can't share through `workgroup`
/// storage, such as a struct or a matrix, because the render pass that dispatches the tile function reserves the
/// storage by size.
pub(crate) fn workgroup_bytes(besl_type: &str) -> Option<u32> {
	Some(match besl_type {
		"f16" | "u16" => 2,
		"f32" | "u32" | "i32" | "vec2f16" | "vec2u16" | "atomicu32" | "atomici32" => 4,
		"vec2f" | "vec2u" | "vec2i" | "vec3f16" | "vec4f16" | "vec3u16" | "vec4u16" => 8,
		"vec3f" | "vec3u" | "vec4f" | "vec4u" => 16,
		_ => return None,
	})
}

/// Returns the interface of tile shader `program` running on `tile` x `tile` pixel tiles, after checking its outputs.
///
/// A tile shader writes one to eight `vec4f` outputs with distinct locations below eight, none of them arrays. `name`
/// names the shader in errors. Call it before [`lower_for`], which can replace the outputs.
pub(crate) fn reflect(tile: u32, name: &str, program: &besl::Program) -> Result<TileInterface, String> {
	let docs = || crate::online_docs_url(TILE_ENTRY_DOCS_PATH);
	let mut outputs = Vec::new();
	for output in collect_outputs(program) {
		let output = &program[output];
		let besl::Nodes::Output {
			name: output_name,
			format,
			location,
			count,
			..
		} = output.node()
		else {
			continue;
		};
		let format = program.name(*format).unwrap_or("unnamed").to_string();
		if format != "vec4f" || count.is_some() {
			return Err(format!(
				"Tile shader `{name}` declares output `{output_name}` as `{format}`{array}. The most likely cause is an output copied from a fragment or mesh shader. Declare it as `{output_name}: output<vec4f, {location}>;`. See {}.",
				docs(),
				array = if count.is_some() { " array" } else { "" },
				output_name = program.text(*output_name),
			));
		}
		if usize::from(*location) >= TileInterface::MAX_OUTPUTS
			|| outputs
				.iter()
				.any(|output: &TileOutput| output.location == u32::from(*location))
		{
			return Err(format!(
				"Tile shader `{name}` declares output `{output_name}` at location {location}, which is out of range or taken. The most likely cause is a repeated or skipped location. Give each output its own location from 0 to {}. See {}.",
				TileInterface::MAX_OUTPUTS - 1,
				docs(),
				output_name = program.text(*output_name),
			));
		}
		outputs.push(TileOutput {
			name: program.text(*output_name).to_owned(),
			location: u32::from(*location),
		});
	}
	if outputs.is_empty() {
		return Err(format!(
			"Tile shader `{name}` declares no outputs. The most likely cause is a compute shader moved to the `Tile` stage that still writes a storage image. Declare its result as `result: output<vec4f, 0>;` and write it with `write(result, pixel, value)`. See {}.",
			docs()
		));
	}
	outputs.sort_by_key(|output| output.location);
	Ok(TileInterface {
		size: tile,
		outputs,
		threadgroup_memory: Vec::new(),
	})
}

/// Returns the stage that `language` runs a shader of `stage` as, after rewriting `program` for it.
///
/// Metal runs a `Tile` shader as a tile function, unchanged. Other platforms run it as a compute shader: the tile
/// stage's rules are checked while the program still has its outputs, and [`lower_as_compute`] then rewrites it.
/// Every other stage runs as itself. `name` names the shader in errors.
pub(crate) fn lower_for(
	language: PlatformShaderLanguage,
	stage: Stages,
	name: &str,
	program: &mut besl::Program,
) -> Result<Stages, String> {
	let Stages::Tile { local_size, .. } = stage else {
		return Ok(stage);
	};
	if language.is_msl() {
		return Ok(stage);
	}
	super::validate_stage(stage, name, program)?;
	lower_as_compute(program)?;
	Ok(Stages::Compute { local_size })
}

/// Rewrites tile shader `program` into the compute shader that platforms without tile functions run instead.
///
/// Each output becomes a write-only, formatless storage image in the [`TILE_OUTPUTS_LAYOUT`] set, a set of its own that
/// comes before the program's own descriptors, numbered in location order. Each `write` to an output becomes the storage-image
/// `write`, and `output_extent()` reads the size of the first output, so the program is an ordinary compute shader.
/// Call it after [`reflect`] and after checking the tile stage's rules, and before evaluating the program's sets, so
/// the outputs set is reflected like any other.
fn lower_as_compute(program: &mut besl::Program) -> Result<(), String> {
	let mut outputs = collect_outputs(program);
	outputs.sort_by_key(|output| match program[output].node() {
		besl::Nodes::Output { location, .. } => *location,
		_ => u8::MAX,
	});
	let Some(first_output) = outputs.first().cloned() else {
		return Ok(());
	};
	let first_output_name = program[first_output].get_name().expect("Every output has a name.");
	let missing_intrinsic = |name: &str| {
		format!(
			"Missing the `{name}` intrinsic while lowering a tile shader. The most likely cause is a program linked without the BESL root scope. Link the program with `besl::lex`."
		)
	};
	let image_size = program
		.get_descendant(program.root(), "image_size")
		.ok_or_else(|| missing_intrinsic("image_size"))?;
	// The walks below compare every node's name, so they look the names up once.
	let (Some(write), Some(image)) = (program.find_name("write"), program.find_name("image")) else {
		return Err(missing_intrinsic("write"));
	};
	let write_image = collect_nodes(program, &|node| {
		matches!(node, besl::Nodes::Intrinsic { name, elements, .. } if *name == write && elements.first().is_some_and(|element| program[element].get_name() == Some(image)))
	})
	.pop()
	.ok_or_else(|| missing_intrinsic("write"))?;

	// Each `write` to an output becomes the storage-image `write`, found while the outputs are still outputs.
	for call in collect_nodes(
		program,
		&|node| matches!(node, besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, arguments, .. }) if program[intrinsic].get_name() == Some(write) && arguments.first().and_then(|node| besl::referenced_output(program, node)).is_some()),
	) {
		if let besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) = program[call].node_mut() {
			*intrinsic = write_image;
		}
	}

	// `output_extent()` becomes `image_size(first_output)`; the call nodes are shared, so every use sees the change.
	// A program without the `output_extent` name has no call to it.
	let output_extent = program.find_name("output_extent");
	for call in collect_nodes(
		program,
		&|node| matches!(node, besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) if output_extent.is_some() && program[intrinsic].get_name() == output_extent),
	) {
		let argument = program.add(besl::Node::expression(besl::Expressions::Member {
			name: first_output_name,
			source: first_output,
		}));
		*program[call].node_mut() = besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic: image_size,
			arguments: vec![argument],
			elements: vec![argument],
		});
	}

	for (index, output) in outputs.iter().enumerate() {
		let name = program[output].get_name().expect("Every output has a name.");
		let binding = besl::Node::binding(
			name,
			besl::BindingTypes::Image {
				format: None,
				layered: false,
			},
			index as u32,
			false,
			true,
		)
		.in_layout(TILE_OUTPUTS_LAYOUT);
		program[output] = binding;
	}
	// The outputs set is a set of its own, so the program's sets are placed again to give it a position.
	besl::place_descriptors(program).map_err(|error| {
		format!(
			"Failed to place the outputs of a tile shader in a descriptor set ({error:?}). The most likely cause is an output set with more bindings than one set holds. Declare fewer outputs."
		)
	})
}

/// Returns every `output` declaration of `program`.
fn collect_outputs(program: &besl::Program) -> Vec<besl::NodeId> {
	collect_nodes(program, &|node| matches!(node, besl::Nodes::Output { .. }))
}

/// Returns every node of `program`, once each, that `matches` accepts: the declarations of every scope, the shader's
/// own and its layers', and everything under them.
fn collect_nodes(program: &besl::Program, matches: &dyn Fn(&besl::Nodes) -> bool) -> Vec<besl::NodeId> {
	fn visit(
		program: &besl::Program,
		node: &besl::NodeId,
		matches: &dyn Fn(&besl::Nodes) -> bool,
		seen: &mut besl::NodeSet,
		found: &mut Vec<besl::NodeId>,
	) {
		if !seen.insert(*node) {
			return;
		}
		let borrowed = &program[node];
		if matches(borrowed.node()) {
			found.push(*node);
		}
		// Scopes hold declarations, which expressions only name, so they are walked apart from expression children.
		// The targets of a layer belong to the shader only when `main` takes the layer.
		if let besl::Nodes::Scope { children, .. }
		| besl::Nodes::Layer {
			members: children,
			entry: Some(_),
			..
		} = borrowed.node()
		{
			for child in children {
				visit(program, child, matches, seen, found);
			}
		}
		for child in borrowed.node().children() {
			visit(program, child, matches, seen, found);
		}
	}

	let mut found = Vec::new();
	visit(
		program,
		&program.root(),
		matches,
		&mut besl::NodeSet::for_program(program),
		&mut found,
	);
	found
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::*;
	use crate::shader::{
		besl::evaluation::{BindingKind, ProgramEvaluation},
		generator::ShaderGenerationSettings,
	};

	const TILE_SOURCE: &str = r#"
		source: descriptor<{ type: Texture2D, binding: 0, access: read }>;
		result: output<vec4f, 0>;

		main: fn (input: StageInput) -> void {
			let pixel: vec2u = input.thread_id;
			let extent: vec2u = input.extent;
			if (pixel.x >= extent.x || pixel.y >= extent.y) {
				return;
			}
			write(result, pixel, fetch(source, pixel));
		}
	"#;

	/// Verifies that platforms without tile functions run a tile shader as an ordinary compute shader: it passes the
	/// compute stage's rules, writes its output as a storage image in the outputs set, and lowers to GLSL and HLSL.
	#[test]
	fn tile_shader_runs_as_compute_without_tile_functions() {
		let builtins = besl::Builtins::new();
		let stage = Stages::Tile {
			tile: 16,
			local_size: Extent::square(16),
		};
		for language in [PlatformShaderLanguage::Glsl, PlatformShaderLanguage::Hlsl] {
			let mut program =
				besl::compile_to_besl(TILE_SOURCE, besl::Program::new(&builtins)).expect("the tile shader should link");
			reflect(16, "tile.besl", &program).expect("the tile shader's outputs should reflect");

			let lowered = lower_for(language, stage, "tile.besl", &mut program).expect("the tile shader should lower");

			assert!(matches!(lowered, Stages::Compute { local_size } if local_size == Extent::square(16)));
			super::super::validate_stage(lowered, "tile.besl", &program)
				.expect("the lowered shader should follow the compute stage's rules");
			let sets = ProgramEvaluation::from_program(&mut program)
				.expect("the lowered shader should reflect")
				.into_sets();
			let outputs = sets
				.iter()
				.find(|set| set.layout.as_deref() == Some(TILE_OUTPUTS_LAYOUT))
				.expect("the outputs become a set of their own");
			let [binding] = outputs.bindings.as_slice() else {
				panic!("the outputs set holds one binding per output: {:?}", outputs.bindings);
			};
			assert_eq!(
				(binding.name.as_str(), binding.kind, binding.slot, binding.read, binding.write),
				("result", BindingKind::StorageImage { layered: false }, 0, false, true)
			);

			let settings = ShaderGenerationSettings::compute(Extent::square(16));
			let main = program.get_main().expect("the lowered shader keeps its main");
			match language {
				PlatformShaderLanguage::Glsl => {
					let shader = super::super::glsl::GLSLTranspiler::new(&mut program)
						.minified(true)
						.generate(&settings, &main)
						.expect("the lowered shader should generate GLSL");
					assert_string_contains!(shader, "#pragma shader_stage(compute)");
					assert_string_contains!(shader, "imageStore(besl_binding_result[");
					assert_string_contains!(shader, "imageSize(besl_binding_result[");
				}
				_ => {
					let shader = super::super::hlsl::Generator::new(&mut program)
						.minified(true)
						.generate(&settings, &main)
						.expect("the lowered shader should generate HLSL");
					assert_string_contains!(shader, "result[");
				}
			}
		}
	}
}
