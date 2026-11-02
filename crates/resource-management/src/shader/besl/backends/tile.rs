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
pub(crate) fn reflect(tile: u32, name: &str, program: &besl::NodeReference) -> Result<TileInterface, String> {
	let docs = || crate::online_docs_url(TILE_ENTRY_DOCS_PATH);
	let mut outputs = Vec::new();
	for output in collect_outputs(program) {
		let output = output.borrow();
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
		let format = format.borrow().get_name().unwrap_or("unnamed").to_string();
		if format != "vec4f" || count.is_some() {
			return Err(format!(
				"Tile shader `{name}` declares output `{output_name}` as `{format}`{array}. The most likely cause is an output copied from a fragment or mesh shader. Declare it as `{output_name}: output<vec4f, {location}>;`. See {}.",
				docs(),
				array = if count.is_some() { " array" } else { "" },
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
				docs()
			));
		}
		outputs.push(TileOutput {
			name: output_name.clone(),
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
	program: &besl::NodeReference,
) -> Result<Stages, String> {
	let Stages::Tile { local_size, .. } = stage else {
		return Ok(stage);
	};
	if language.is_msl() {
		return Ok(stage);
	}
	super::validate_stage(language, stage, name, program)?;
	lower_as_compute(program)?;
	Ok(Stages::Compute { local_size })
}

/// Rewrites tile shader `program` into the compute shader that platforms without tile functions run instead.
///
/// Each output becomes a write-only, formatless storage image in the [`TILE_OUTPUTS_LAYOUT`] set, at the flat slots
/// after every binding the program declares, in location order. Each `write` to an output becomes the storage-image
/// `write`, and `output_extent()` reads the size of the first output, so the program is an ordinary compute shader.
/// Call it after [`reflect`] and after checking the tile stage's rules, and before evaluating the program's sets, so
/// the outputs set is reflected like any other.
fn lower_as_compute(program: &besl::NodeReference) -> Result<(), String> {
	let mut outputs = collect_outputs(program);
	outputs.sort_by_key(|output| match output.borrow().node() {
		besl::Nodes::Output { location, .. } => *location,
		_ => u8::MAX,
	});
	// The outputs go after every flat slot the program's own bindings take.
	let first_slot = collect_nodes(program, &|node| matches!(node, besl::Nodes::Binding { .. }))
		.iter()
		.filter_map(|binding| match binding.borrow().node() {
			besl::Nodes::Binding { slot, count, .. } => Some(slot + count.map_or(1, std::num::NonZeroU32::get)),
			_ => None,
		})
		.max()
		.unwrap_or(0);

	let Some(first_output) = outputs.first().cloned() else {
		return Ok(());
	};
	let first_output_name = first_output.borrow().get_name().unwrap_or_default().to_string();
	let missing_intrinsic = |name: &str| {
		format!(
			"Missing the `{name}` intrinsic while lowering a tile shader. The most likely cause is a program linked without the BESL root scope. Link the program with `besl::lex`."
		)
	};
	let image_size = program
		.get_descendant("image_size")
		.ok_or_else(|| missing_intrinsic("image_size"))?;
	let write_image = collect_nodes(program, &|node| {
		matches!(node, besl::Nodes::Intrinsic { name, elements, .. } if name == "write" && elements.first().is_some_and(|image| image.borrow().get_name() == Some("image")))
	})
	.pop()
	.ok_or_else(|| missing_intrinsic("write"))?;

	// Each `write` to an output becomes the storage-image `write`, found while the outputs are still outputs.
	for call in collect_nodes(
		program,
		&|node| matches!(node, besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, arguments, .. }) if intrinsic.borrow().get_name() == Some("write") && arguments.first().and_then(besl::referenced_output).is_some()),
	) {
		if let besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) = call.borrow_mut().node_mut() {
			*intrinsic = write_image.clone();
		}
	}

	// `output_extent()` becomes `image_size(first_output)`; the call nodes are shared, so every use sees the change.
	for call in collect_nodes(
		program,
		&|node| matches!(node, besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) if intrinsic.borrow().get_name() == Some("output_extent")),
	) {
		let argument: besl::NodeReference = besl::Node::expression(besl::Expressions::Member {
			name: first_output_name.clone(),
			source: first_output.clone(),
		})
		.into();
		*call.borrow_mut().node_mut() = besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic: image_size.clone(),
			arguments: vec![argument.clone()],
			elements: vec![argument],
		});
	}

	for (index, output) in outputs.iter().enumerate() {
		let name = output.borrow().get_name().unwrap_or_default().to_string();
		let binding = besl::Node::binding(
			&name,
			besl::BindingTypes::Image { format: String::new() },
			first_slot + index as u32,
			false,
			true,
		)
		.in_layout(TILE_OUTPUTS_LAYOUT);
		*output.borrow_mut() = binding;
	}
	Ok(())
}

/// Returns every `output` declaration of `program`.
fn collect_outputs(program: &besl::NodeReference) -> Vec<besl::NodeReference> {
	collect_nodes(program, &|node| matches!(node, besl::Nodes::Output { .. }))
}

/// Returns every node under `root`, once each, that `matches` accepts: the declarations of every scope, the shader's
/// own and its layers', and everything under them.
fn collect_nodes(root: &besl::NodeReference, matches: &dyn Fn(&besl::Nodes) -> bool) -> Vec<besl::NodeReference> {
	fn visit(
		node: &besl::NodeReference,
		matches: &dyn Fn(&besl::Nodes) -> bool,
		seen: &mut std::collections::HashSet<*const std::cell::RefCell<besl::Node>>,
		found: &mut Vec<besl::NodeReference>,
	) {
		let identity: *const std::cell::RefCell<besl::Node> = &**node;
		if !seen.insert(identity) {
			return;
		}
		let borrowed = node.borrow();
		if matches(borrowed.node()) {
			found.push(node.clone());
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
				visit(child, matches, seen, found);
			}
		}
		for child in borrowed.node().children() {
			visit(child, matches, seen, found);
		}
	}

	let mut found = Vec::new();
	visit(root, matches, &mut std::collections::HashSet::new(), &mut found);
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
		let stage = Stages::Tile {
			tile: 16,
			local_size: Extent::square(16),
		};
		for language in [PlatformShaderLanguage::Glsl, PlatformShaderLanguage::Hlsl] {
			let program = besl::compile_to_besl(TILE_SOURCE, None).expect("the tile shader should link");
			reflect(16, "tile.besl", &program).expect("the tile shader's outputs should reflect");

			let lowered = lower_for(language, stage, "tile.besl", &program).expect("the tile shader should lower");

			assert!(matches!(lowered, Stages::Compute { local_size } if local_size == Extent::square(16)));
			super::super::validate_stage(language, lowered, "tile.besl", &program)
				.expect("the lowered shader should follow the compute stage's rules");
			let sets = ProgramEvaluation::from_program(&program)
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
				("result", BindingKind::StorageImage, 1, false, true)
			);

			let settings = ShaderGenerationSettings::compute(Extent::square(16));
			let main = program.get_main().expect("the lowered shader keeps its main");
			match language {
				PlatformShaderLanguage::Glsl => {
					let shader = super::super::glsl::GLSLTranspiler::new()
						.minified(true)
						.generate(&settings, &main)
						.expect("the lowered shader should generate GLSL");
					assert_string_contains!(shader, "#pragma shader_stage(compute)");
					assert_string_contains!(shader, "imageStore(result,");
					assert_string_contains!(shader, "imageSize(result)");
				}
				_ => {
					let shader = super::super::hlsl::Generator::new()
						.minified(true)
						.generate(&settings, &main)
						.expect("the lowered shader should generate HLSL");
					assert_string_contains!(shader, "result[");
				}
			}
		}
	}
}
