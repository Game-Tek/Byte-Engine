//! Checks the rules each shader stage sets on the code it runs, before a backend lowers a program.
//!
//! Backends report a failed lowering without saying why, so the rules a shader author can break are checked here
//! first, each with an error that names the rule and how to fix it.

use super::{RAY_TRACING_STAGE_INTRINSICS, SUBGROUP_INTRINSICS, any_code_node, platform::PlatformShaderLanguage};
use crate::shader::generator::Stages;

const RAY_TRACING_ENTRY_DOCS_PATH: &str = "reference/besl/interface#ray-tracing-entry-points";
const RAY_QUERIES_DOCS_PATH: &str = "reference/besl/intrinsics#ray-queries";
const RAY_GENERATION_DOCS_PATH: &str = "reference/besl/intrinsics#ray-generation-stage";
const CLOSEST_HIT_DOCS_PATH: &str = "reference/besl/intrinsics#closest-hit-stage";
const STAGE_SUPPORT_DOCS_PATH: &str = "reference/besl/interface#stage-and-backend-support";

/// The calls that read compute invocation values, which ray tracing stages don't have.
const COMPUTE_INVOCATION_INTRINSICS: [&str; 4] = ["thread_id", "thread_idx", "threadgroup_position", "thread_position"];

/// Checks that `program`, lowered for `stage` in `language`, follows the rules of its stage.
///
/// A program that breaks a rule fails with an error that names the rule and the fix, instead of the generic failure
/// a backend reports. `name` names the shader in the errors. Call it before a backend lowers `program`; next, lower
/// the program with [`super::platform::Generator::lower`].
pub(crate) fn validate_stage(
	language: PlatformShaderLanguage,
	stage: Stages,
	name: &str,
	program: &besl::NodeReference,
) -> Result<(), String> {
	let Some(main) = program.get_main() else {
		return Ok(());
	};

	if stage.is_ray_tracing() && !lowers_ray_tracing(language) {
		return Err(format!(
			"{stage} shader `{name}` can't be generated as {language}. The most likely cause is a ray tracing shader baked for DX12, whose shader generator doesn't lower ray tracing yet. Bake ray tracing shaders only for Metal and Vulkan, and leave them out of bakes for DX12. See {docs}.",
			stage = stage_name(stage),
			language = language_name(language),
			docs = crate::online_docs_url(STAGE_SUPPORT_DOCS_PATH),
		));
	}

	validate_entry_point(stage, name, &main)?;
	if matches!(stage, Stages::ClosestHit) {
		validate_closest_hit_descriptors(name, program)?;
	}

	// The first rule the code breaks, found by one walk over `main` and every function it calls.
	let mut violation = None;
	any_code_node(&main, true, &mut |node| {
		violation = code_violation(language, stage, name, node);
		violation.is_some()
	});
	violation.map_or(Ok(()), Err)
}

/// Checks the signature that normalization leaves on a ray tracing `main`: a closest hit takes its payload and returns
/// it, and a ray generation shader takes and returns nothing.
fn validate_entry_point(stage: Stages, name: &str, main: &besl::NodeReference) -> Result<(), String> {
	let main = main.borrow();
	let besl::Nodes::Function { params, return_type, .. } = main.node() else {
		return Ok(());
	};
	let docs = || crate::online_docs_url(RAY_TRACING_ENTRY_DOCS_PATH);
	match stage {
		Stages::ClosestHit => {
			let takes_payload = matches!(
				params.as_slice(),
				[payload] if matches!(payload.borrow().node(), besl::Nodes::Parameter { r#type, .. } if r#type == return_type)
			);
			if takes_payload {
				Ok(())
			} else {
				Err(format!(
					"Closest-hit shader `{name}` has a `main` that doesn't take and return its payload. The most likely cause is a `main` without a payload parameter, with a parameter after the payload, or with a return type other than the payload's. Declare `main: fn (input: StageInput, payload: P) -> P`, where `P` is the payload struct. See {}.",
					docs()
				))
			}
		}
		Stages::RayGeneration => {
			if params.is_empty() && return_type.borrow().get_name() == Some("void") {
				Ok(())
			} else {
				Err(format!(
					"Ray generation shader `{name}` has a `main` that takes a parameter or returns a value. The most likely cause is a closest-hit `main` baked with the `RayGeneration` stage. Declare `main: fn (input: StageInput) -> void`, or set `stage` to `ClosestHit` in the shader's `.besl.bead` file. See {}.",
					docs()
				))
			}
		}
		_ => Ok(()),
	}
}

/// Rejects a closest hit that declares descriptors itself. Its sets must be the first sets of the ray generation
/// shader, which only shared layout layers can declare.
fn validate_closest_hit_descriptors(name: &str, program: &besl::NodeReference) -> Result<(), String> {
	let sets = crate::shader::besl::evaluation::collect_set_nodes(program)?;
	let Some(own) = sets
		.iter()
		.filter(|set| set.layout.is_none())
		.flat_map(|set| &set.nodes)
		.next()
	else {
		return Ok(());
	};
	let own = own.borrow();
	let binding = own.get_name().unwrap_or("<unnamed>");
	Err(format!(
		"Closest-hit shader `{name}` declares descriptor `{binding}` itself. The most likely cause is a descriptor declared in the shader instead of in a layout layer, which gives the closest hit a set that the ray generation shader doesn't have. Move `{binding}` to a layout layer that the ray generation shader lists, and list that layer in this shader too. See {}.",
		crate::online_docs_url(RAY_TRACING_ENTRY_DOCS_PATH)
	))
}

/// Returns the error for the stage rule that `node` breaks, or `None` when it breaks none.
fn code_violation(language: PlatformShaderLanguage, stage: Stages, name: &str, node: &besl::NodeReference) -> Option<String> {
	let stage_name = stage_name(stage);
	let ray_tracing_docs = || crate::online_docs_url(RAY_TRACING_ENTRY_DOCS_PATH);
	match node.borrow().node() {
		besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) => {
			let intrinsic = intrinsic.borrow();
			let call = intrinsic.get_name()?;
			call_violation(language, stage, name, call)
		}
		besl::Nodes::Expression(besl::Expressions::Discard) if stage.is_ray_tracing() => Some(format!(
			"{stage_name} shader `{name}` uses `discard`. The most likely cause is fragment code moved into a ray tracing shader, which has no fragment to discard. Remove `discard`, and report the case in the payload instead. See {}.",
			ray_tracing_docs()
		)),
		besl::Nodes::Workgroup { name: storage, .. } if stage.is_ray_tracing() => Some(format!(
			"{stage_name} shader `{name}` uses workgroup storage `{storage}`. The most likely cause is compute code moved into a ray tracing shader, whose invocations share no workgroup. Use a local variable or a buffer instead. See {}.",
			ray_tracing_docs()
		)),
		_ => None,
	}
}

/// Returns the error for a call to the intrinsic named `call` that `stage` or `language` doesn't allow, or `None`.
fn call_violation(language: PlatformShaderLanguage, stage: Stages, name: &str, call: &str) -> Option<String> {
	let stage_name = stage_name(stage);
	let traces_rays = matches!(call, "intersect" | "trace_occlusion" | "invoke");
	if traces_rays && !lowers_ray_tracing(language) {
		return Some(format!(
			"Shader `{name}` calls `{call}`, which can't be generated as {language}. The most likely cause is a ray query or `invoke` in a shader baked for DX12, whose shader generator doesn't lower ray tracing yet. Bake shaders that trace rays only for Metal and Vulkan, and leave them out of bakes for DX12. See {docs}.",
			language = language_name(language),
			docs = crate::online_docs_url(STAGE_SUPPORT_DOCS_PATH),
		));
	}
	match call {
		"invoke" if !matches!(stage, Stages::RayGeneration) => Some(format!(
			"{stage_name} shader `{name}` calls `invoke`. The most likely cause is shading code outside the ray generation shader, the only stage with a shader binding table to run closest hits from. Call `invoke` only in `RayGeneration` shaders. See {}.",
			crate::online_docs_url(RAY_GENERATION_DOCS_PATH)
		)),
		"intersect" | "trace_occlusion" if !matches!(stage, Stages::Compute { .. } | Stages::RayGeneration) => Some(format!(
			"{stage_name} shader `{name}` calls `{call}`. The most likely cause is a ray query in a stage that can't trace rays: only compute and ray generation shaders can. Trace the ray in a `Compute` or `RayGeneration` shader, and pass the result to this one. See {}.",
			crate::online_docs_url(RAY_QUERIES_DOCS_PATH)
		)),
		"launch_index" | "launch_size" if !matches!(stage, Stages::RayGeneration) => Some(format!(
			"{stage_name} shader `{name}` reads `{call}`. The most likely cause is a launch value read outside a ray generation shader, the only stage that runs on a launch grid. Read it in the `RayGeneration` shader, and pass it on in the payload or a buffer. See {}.",
			crate::online_docs_url(RAY_GENERATION_DOCS_PATH)
		)),
		call if RAY_TRACING_STAGE_INTRINSICS.contains(&call)
			&& !matches!(call, "invoke" | "launch_index" | "launch_size")
			&& !matches!(stage, Stages::ClosestHit) =>
		{
			Some(format!(
				"{stage_name} shader `{name}` reads `{call}`. The most likely cause is a hit value read outside a closest-hit shader, the only stage that shades a hit. Read the matching member of the `Hit` that `intersect` returns instead, such as `hit.distance` for `hit_distance`. See {}.",
				crate::online_docs_url(CLOSEST_HIT_DOCS_PATH)
			))
		}
		call if stage.is_ray_tracing()
			&& (call == "workgroup_barrier"
				|| SUBGROUP_INTRINSICS.contains(&call)
				|| COMPUTE_INVOCATION_INTRINSICS.contains(&call)) =>
		{
			Some(format!(
				"{stage_name} shader `{name}` calls `{call}`. The most likely cause is compute code moved into a ray tracing shader, whose invocations share no workgroup or subgroup. Remove the call, and run the code that needs it in a `Compute` shader. See {}.",
				crate::online_docs_url(RAY_TRACING_ENTRY_DOCS_PATH)
			))
		}
		_ => None,
	}
}

/// Reports whether the generator of `language` lowers ray tracing stages and ray queries: MSL for Metal and GLSL for
/// Vulkan do, HLSL for DX12 doesn't yet.
fn lowers_ray_tracing(language: PlatformShaderLanguage) -> bool {
	!matches!(language, PlatformShaderLanguage::Hlsl)
}

/// Returns the name a `.besl.bead` file gives `stage`.
fn stage_name(stage: Stages) -> &'static str {
	match stage {
		Stages::Vertex => "Vertex",
		Stages::Fragment => "Fragment",
		Stages::Compute { .. } => "Compute",
		Stages::Task { .. } => "Task",
		Stages::Mesh { .. } => "Mesh",
		Stages::RayGeneration => "RayGeneration",
		Stages::ClosestHit => "ClosestHit",
	}
}

fn language_name(language: PlatformShaderLanguage) -> &'static str {
	match language {
		PlatformShaderLanguage::Glsl => "GLSL",
		PlatformShaderLanguage::Hlsl => "HLSL",
		PlatformShaderLanguage::Msl => "MSL",
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The layers every fixture lists: the payload closest hits take and return, and the scene ray queries test.
	const LAYERS: [(&str, &str); 2] = [
		("surface", "SurfaceHit: struct { albedo: vec3f, distance: f32, }"),
		(
			"scene",
			"scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>;",
		),
	];

	/// Links `source` under [`LAYERS`] and checks it for `stage` in `language`.
	fn validate(language: PlatformShaderLanguage, stage: Stages, source: &str) -> Result<(), String> {
		let module = besl::parse(source).unwrap_or_else(|error| panic!("Expected the fixture to parse ({error:?}): {source}"));
		let program = besl::stack_layers(module, LAYERS)
			.and_then(besl::lex)
			.unwrap_or_else(|error| panic!("Expected the fixture to link ({error:?}): {source}"));
		validate_stage(language, stage, "fixture", &program)
	}

	/// Asserts that checking `source` for `stage` in MSL fails with an error that contains `expected` and links to the
	/// rule it breaks.
	fn assert_rejected(stage: Stages, source: &str, expected: &str) {
		let error = validate(PlatformShaderLanguage::Msl, stage, source).expect_err(&format!(
			"Expected `{expected}` to be rejected in {}: {source}",
			stage_name(stage)
		));
		assert!(error.contains(expected), "Expected an error about `{expected}`, got: {error}");
		assert!(
			error.contains("The most likely cause"),
			"The error should name its cause: {error}"
		);
		assert!(
			error.contains("reference/besl/"),
			"The error should link to the rule it breaks: {error}"
		);
	}

	#[test]
	fn ray_tracing_shaders_that_follow_their_stage_rules_are_accepted() {
		let compute = Stages::Compute {
			local_size: utils::Extent::line(64),
		};
		let shaders = [
			(
				Stages::RayGeneration,
				"main: fn (input: StageInput) -> void { let hit: Hit = intersect(scene, 255, vec3f(f32(input.launch_index.x), 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 100.0); let surface: SurfaceHit = invoke(hit, SurfaceHit(vec3f(0.0, 0.0, 0.0), -1.0)); }",
			),
			(
				Stages::ClosestHit,
				"main: fn (input: StageInput, payload: SurfaceHit) -> SurfaceHit { let surface: SurfaceHit = payload; surface.distance = input.hit_distance; return surface; }",
			),
			(
				compute,
				"main: fn () -> void { let blocked: bool = trace_occlusion(scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 100.0); }",
			),
		];
		for language in [PlatformShaderLanguage::Msl, PlatformShaderLanguage::Glsl] {
			for (stage, source) in shaders {
				if let Err(error) = validate(language, stage, source) {
					panic!(
						"Expected the {} shader to be accepted as {}, got: {error}",
						stage_name(stage),
						language_name(language)
					);
				}
			}
		}
	}

	#[test]
	fn calls_that_a_stage_does_not_allow_are_rejected() {
		let cases = [
			(
				Stages::Fragment,
				"main: fn () -> void { let hit: Hit = intersect(scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 1.0); }",
				"calls `intersect`",
			),
			(
				Stages::ClosestHit,
				"main: fn (payload: SurfaceHit) -> SurfaceHit { let blocked: bool = trace_occlusion(scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 1.0); return payload; }",
				"calls `trace_occlusion`",
			),
			(
				Stages::ClosestHit,
				"main: fn (input: StageInput, payload: SurfaceHit) -> SurfaceHit { let hit: Hit = Hit(0, 1.0, 0, 0, 0, vec2f(0.0, 0.0), input.ray_origin, input.ray_direction); return invoke(hit, payload); }",
				"calls `invoke`",
			),
			(
				Stages::Compute {
					local_size: utils::Extent::line(1),
				},
				"main: fn () -> void { let size: vec3u = launch_size(); }",
				"reads `launch_size`",
			),
			(
				Stages::RayGeneration,
				"main: fn () -> void { let distance: f32 = hit_distance(); }",
				"reads `hit_distance`",
			),
			(
				Stages::RayGeneration,
				"main: fn () -> void { workgroup_barrier(); }",
				"calls `workgroup_barrier`",
			),
			(
				Stages::ClosestHit,
				"main: fn (payload: SurfaceHit) -> SurfaceHit { let lane: u32 = subgroup_lane_index(); return payload; }",
				"calls `subgroup_lane_index`",
			),
			(
				Stages::RayGeneration,
				"main: fn () -> void { let id: vec2u = thread_id(); }",
				"calls `thread_id`",
			),
			(Stages::RayGeneration, "main: fn () -> void { discard; }", "uses `discard`"),
			(
				Stages::RayGeneration,
				"scratch: workgroup<f32, 4>; main: fn () -> void { scratch[0] = 1.0; }",
				"uses workgroup storage `scratch`",
			),
		];
		for (stage, source, expected) in cases {
			assert_rejected(stage, source, expected);
		}
	}

	#[test]
	fn ray_tracing_entry_points_must_match_their_stage() {
		assert_rejected(
			Stages::RayGeneration,
			"main: fn (payload: SurfaceHit) -> SurfaceHit { return payload; }",
			"takes a parameter or returns a value",
		);
		assert_rejected(
			Stages::ClosestHit,
			"main: fn () -> void {}",
			"doesn't take and return its payload",
		);
	}

	#[test]
	fn closest_hits_that_declare_descriptors_are_rejected() {
		assert_rejected(
			Stages::ClosestHit,
			"albedos: descriptor<{ type: vec4f[], binding: 1, access: read }>; main: fn (input: StageInput, payload: SurfaceHit) -> SurfaceHit { let surface: SurfaceHit = payload; surface.distance = albedos[input.hit_geometry_index].x; return surface; }",
			"declares descriptor `",
		);
	}

	#[test]
	fn ray_tracing_is_rejected_for_hlsl() {
		let language = PlatformShaderLanguage::Hlsl;
		let error = validate(language, Stages::RayGeneration, "main: fn () -> void {}")
			.expect_err("Expected a ray generation shader to be rejected");
		assert!(error.contains("can't be generated as HLSL"), "{error}");

		let compute = Stages::Compute {
			local_size: utils::Extent::line(1),
		};
		let error = validate(
			language,
			compute,
			"main: fn () -> void { let hit: Hit = intersect(scene, 255, vec3f(0.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), 0.0, 1.0); }",
		)
		.expect_err("Expected a ray query to be rejected");
		assert!(
			error.contains("calls `intersect`, which can't be generated as HLSL"),
			"{error}"
		);
	}
}
