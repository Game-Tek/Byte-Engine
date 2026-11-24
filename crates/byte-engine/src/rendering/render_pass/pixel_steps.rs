//! Pixel steps, and the runs that record consecutive steps as one pass.
//!
//! A pixel step is a post-scene pass that only changes each pixel of `main` from that pixel's own color, such as a
//! tone mapper. Implement [`PixelStep`] for one, with the [`RenderFeature`] that builds it, and register the feature
//! with [`crate::rendering::renderer::Renderer::add_pixel_step`]. When the render graph closes, the renderer groups
//! registered steps into runs: each run takes the longest fusion in the resource store that matches its steps, and
//! records as one tile pass that reads and writes `main` once. See
//! [pixel steps and fusion](/docs/reference/rendering#pixel-steps-and-fusion).

use ghi::context::ContextCreate as _;
use smallvec::SmallVec;

use super::{Installed, RenderFeature, RenderPassBuilder, RenderPassReturn, allocate_render_command, simple_tile};
use crate::rendering::{Sink, render_passes::blit::ImageBypassPass};

/// The `PixelStep` trait is one sink's step that only transforms each pixel of `main`, which runs alone or fused with
/// the steps registered next to it, without recording commands itself.
///
/// The step's pipeline is a `tile` pipeline whose shader is a
/// [pixel shader](/docs/reference/besl/interface#pixel-shaders). Build the step's buffers, images, and samplers in the
/// constructor [`RenderFeature::create_pass`] calls, then hand the renderer what its shader's layer binds with
/// [`Self::resources`]. `F` is the feature that builds the step, which every call borrows; its name is the one
/// `render.pass.<name>` enables or bypasses.
pub trait PixelStep<F> {
	/// Returns the render-target name of the replacement `main` when this step ends its run, which screenshots capture.
	fn output(&self) -> &'static str;

	/// Returns the format the step writes for `input`, the format of the `main` it reads. Most steps keep it.
	fn output_format(&self, input: ghi::Formats) -> ghi::Formats {
		input
	}

	/// Returns the resources the shader's layer binds, by BESL name; empty for a shader that takes no layer.
	///
	/// The renderer reads them once, when it binds the step's run.
	fn resources(&self) -> &[simple_tile::Resource];

	/// Updates the step's own GPU data for the frame, such as its parameter buffer, and reports whether it applies
	/// anything this frame.
	///
	/// The renderer calls it every frame, enabled or bypassed, so a step can keep draining its inputs. A step that
	/// returns `false`, such as fog with no fog to draw, leaves `main` unchanged.
	fn prepare(&mut self, _feature: &mut F, _frame: &mut ghi::implementation::Frame, _sink: &Sink) -> bool {
		true
	}

	/// Reports whether this step has output that the last presented frame does not show yet; see
	/// [`super::RenderPass::needs_frame`].
	fn needs_frame(&mut self, _feature: &mut F) -> bool {
		false
	}
}

/// The `InstalledSteps` trait lets the renderer keep step features of different types in one list, each with the steps
/// it built, and reach each sink's step by the sink's index.
pub(crate) trait InstalledSteps {
	/// Returns the feature's name; see [`RenderFeature::name`].
	fn name(&self) -> &'static str;
	/// Builds the step of `sink` through `builder`.
	fn create_step(&mut self, sink: usize, builder: &mut RenderPassBuilder);
	/// Drops the step of `sink`, which no frame renders again.
	fn remove_sink(&mut self, sink: usize);
	/// Brings the feature up to date for the frame; see [`RenderFeature::update`].
	fn update(&mut self);
	/// Asks the feature and then every step whether they need a frame, so each adopts its inputs.
	fn needs_frame(&mut self) -> bool;
	/// Returns the output name of the step of `sink`; see [`PixelStep::output`].
	fn output(&self, sink: usize) -> &'static str;
	/// Returns the format the step of `sink` writes for `input`; see [`PixelStep::output_format`].
	fn output_format(&self, sink: usize, input: ghi::Formats) -> ghi::Formats;
	/// Returns the resources the step of `sink` binds; see [`PixelStep::resources`].
	fn resources(&self, sink: usize) -> &[simple_tile::Resource];
	/// Prepares the step of `sink` and reports whether it applies anything this frame; see [`PixelStep::prepare`].
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sink: &Sink) -> bool;
	/// Returns the feature for [`crate::rendering::Renderer::render_feature_mut`] to downcast.
	fn feature_mut(&mut self) -> &mut dyn std::any::Any;
}

impl<F: RenderFeature> InstalledSteps for Installed<F>
where
	F::Pass: PixelStep<F>,
{
	fn name(&self) -> &'static str {
		self.feature.name()
	}

	fn create_step(&mut self, sink: usize, builder: &mut RenderPassBuilder) {
		let step = self.feature.create_pass(builder);
		self.passes.push((sink, step));
	}

	fn remove_sink(&mut self, sink: usize) {
		self.passes.retain(|(of, _)| *of != sink);
	}

	fn update(&mut self) {
		self.feature.update();
	}

	fn needs_frame(&mut self) -> bool {
		// Every step answers, even after one already did, so each adopts its inputs this tick.
		let feature_needs = self.feature.needs_frame();
		self.passes
			.iter_mut()
			.fold(feature_needs, |needs, (_, step)| step.needs_frame(&mut self.feature) | needs)
	}

	fn output(&self, sink: usize) -> &'static str {
		self.step(sink).output()
	}

	fn output_format(&self, sink: usize, input: ghi::Formats) -> ghi::Formats {
		self.step(sink).output_format(input)
	}

	fn resources(&self, sink: usize) -> &[simple_tile::Resource] {
		self.step(sink).resources()
	}

	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sink: &Sink) -> bool {
		self.pass_mut(sink.index())
			.is_some_and(|(feature, step)| step.prepare(feature, frame, sink))
	}

	fn feature_mut(&mut self) -> &mut dyn std::any::Any {
		&mut self.feature
	}
}

impl<F: RenderFeature> Installed<F>
where
	F::Pass: PixelStep<F>,
{
	/// Returns the step of `sink`, which the renderer built before it builds the run that reads it.
	fn step(&self, sink: usize) -> &F::Pass {
		self.pass(sink).expect(
			"Pixel step is missing for the sink. The most likely cause is that the renderer built a run before building its steps.",
		)
	}
}

/// The `Fusion` struct names one fused pipeline of the resource store and the step pipelines it runs, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fusion {
	pub(crate) id: String,
	pub(crate) steps: Vec<String>,
}

/// Reads every fused pipeline of the store behind `resources`, after baking the stale fusions in development builds.
///
/// A fused pipeline that can't be read is logged and left out.
pub(crate) async fn load_fusions(resources: &resource_management::ResourceManager) -> Vec<Fusion> {
	use resource_management::resources::pipeline::{Pipeline, PipelineKind};

	#[cfg(debug_assertions)]
	resources.bake_discovered("fusion").await;

	let query = resource_management::resource::storage_backend::Query::new("Pipeline").eq("kind", "fused");
	let ids = match resources.query_ids(query).await {
		Ok(page) => page.items,
		Err(error) => {
			log::error!(
				"Failed to find the fused pipelines in the resource store: {error} The most likely cause is an unreadable resource store. Every pixel step records on its own."
			);
			return Vec::new();
		}
	};
	let mut fusions = Vec::with_capacity(ids.len());
	for id in ids {
		match resources.request::<Pipeline>(&id).await {
			Ok(pipeline) => {
				if let PipelineKind::Fused { steps, .. } = &pipeline.resource().kind {
					fusions.push(Fusion {
						steps: steps.clone(),
						id,
					});
				}
			}
			Err(error) => log::error!(
				"Failed to read fused pipeline '{id}': {error} The most likely cause is a fusion that failed to bake. Fix the fusion; until then its steps record on their own."
			),
		}
	}
	fusions
}

/// The `Planned` enum is one entry of a sink's post-scene passes, as [`plan`] groups the registrations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Planned {
	/// The registration at this index, a pass that records its own commands.
	Pass(usize),
	/// The step registrations in `steps`, recorded as one pass with the pipeline `pipeline`.
	Run {
		steps: std::ops::Range<usize>,
		pipeline: String,
	},
}

/// Groups `registrations`, each the pipeline of a pixel step or `None` for a pass that records its own commands, into
/// the passes a sink records, in order.
///
/// Each run starts at the first step not in a run yet and takes the longest of `fusions` whose steps are the pipelines
/// of that step and the ones after it. A step that no fusion takes runs alone on its own pipeline.
pub(crate) fn plan(registrations: &[Option<&str>], fusions: &[Fusion]) -> Vec<Planned> {
	let mut planned = Vec::with_capacity(registrations.len());
	let mut index = 0;
	while index < registrations.len() {
		let Some(pipeline) = registrations[index] else {
			planned.push(Planned::Pass(index));
			index += 1;
			continue;
		};
		// The steps that follow without a pass between them, which a fusion may take from the front.
		let following = registrations[index..]
			.iter()
			.map_while(|registration| *registration)
			.collect::<SmallVec<[&str; 8]>>();
		let fusion = fusions
			.iter()
			.filter(|fusion| fusion.steps.len() <= following.len() && fusion.steps.iter().zip(&following).all(|(a, b)| a == b))
			.max_by_key(|fusion| fusion.steps.len());
		let (length, pipeline) = fusion.map_or((1, pipeline), |fusion| (fusion.steps.len(), fusion.id.as_str()));
		planned.push(Planned::Run {
			steps: index..index + length,
			pipeline: pipeline.to_string(),
		});
		index += length;
	}
	planned
}

/// The `PixelRun` struct records one sink's run of pixel steps as one tile pass over `main`.
///
/// The renderer builds it for each [`Planned::Run`] with [`Self::new`] and records it as a node of the sink's graph.
/// It holds no step: each frame the renderer prepares the run's step registrations, with [`step_mask`], and hands the
/// run the mask of the steps that apply.
pub(crate) struct PixelRun {
	name: &'static str,
	/// The registrations of the run's steps, in order.
	steps: SmallVec<[usize; 4]>,
	pass: simple_tile::Pass,
	/// Forwards `main` unchanged while the run has nothing to apply or its pipeline is not ready.
	main_copy: Option<ImageBypassPass>,
}

impl PixelRun {
	/// Builds the steps of `steps`, each a registration index and its installed feature, in order, for `sink`, as one
	/// pass that runs `pipeline` and replaces `main`.
	///
	/// `pipeline` is the steps' fused pipeline, or the one step's own pipeline.
	pub(crate) fn new(
		builder: &mut RenderPassBuilder<'_>,
		sink: usize,
		steps: &mut [(usize, &mut dyn InstalledSteps)],
		pipeline: &str,
	) -> Self {
		let source = builder.read_from("main");
		let input_format = builder.format_of("main");
		for (_, step) in steps.iter_mut() {
			step.create_step(sink, builder);
		}
		let name: &'static str = match &*steps {
			[(_, step)] => step.name(),
			// A fused run reports one metric for its steps, built once per sink when the graph closes.
			steps => steps.iter().map(|(_, step)| step.name()).collect::<Vec<_>>().join("+").leak(),
		};
		let (_, last) = steps.last().expect("A run has at least one step");
		let output_format = steps
			.iter()
			.fold(input_format, |format, (_, step)| step.output_format(sink, format));
		let output = builder
			.create_main_render_target(ghi::image::Builder::new(output_format, ghi::Uses::Image).name(last.output(sink)));
		let main_copy = builder.take_main_copy();

		let sampler = builder.context().build_sampler(ghi::sampler::Builder::new());
		// Every step's layer names its descriptors apart, so the run binds them all by name with its own.
		let resources = steps
			.iter()
			.flat_map(|(_, step)| step.resources(sink).iter().copied())
			.chain([simple_tile::Resource::combined_image_sampler(
				"source",
				source,
				sampler,
				ghi::Layouts::Read,
			)])
			.collect::<SmallVec<[_; 8]>>();
		let pass = simple_tile::Pipeline::compile(builder, simple_tile::Descriptor::new(name, pipeline, &[output.format()]))
			.bind(name, &resources, &[simple_tile::Output::new("result", output)]);

		Self {
			name,
			steps: steps.iter().map(|(index, _)| *index).collect(),
			pass,
			main_copy,
		}
	}

	/// Returns the name the run reports under: its step's, or its steps' joined with `+` when fused.
	pub(crate) fn name(&self) -> &'static str {
		self.name
	}

	/// Returns the registrations of the run's steps, in order.
	pub(crate) fn steps(&self) -> &[usize] {
		&self.steps
	}

	/// Prepares the run's commands, which apply the steps in `mask`, or forward `main` unchanged when `mask` is empty or
	/// the pipeline is not ready.
	pub(crate) fn prepare<'a>(
		&mut self,
		mask: u32,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		// With no step to apply, or no pipeline yet, `main` passes through unchanged.
		let Some(pass) = (mask != 0).then(|| self.pass.ready(frame)).flatten() else {
			return self.main_copy.as_mut()?.prepare(frame, sink, frame_allocator);
		};
		// Only a fused shader reads a step mask.
		let constant = (self.steps.len() > 1).then_some(mask);
		let extent = sink.extent();
		let label = self.name;
		Some(allocate_render_command(frame_allocator, move |command_buffer| {
			use ghi::command_buffer::CommonCommandBufferMode as _;
			command_buffer.region(
				|region_label| region_label.write_str(label),
				|command_buffer| pass.record_with_constant(command_buffer, extent, constant),
			);
		}))
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, Texture, Value, output_slot};

	use super::{Fusion, Planned, plan};
	use crate::rendering::shader_vm_test::{
		PIXEL_TILE, buffer, compile, empty_image, link_pixel_chain, push_constant_buffer, rgba, run_pixel, slot, texture_2d,
	};

	const CORRECTION: &str = "color-correction/correct.besl";
	const LUT: &str = "lut/apply.besl";
	const ACES: &str = "aces/tone-mapping.besl";

	/// The `Step` struct holds one step's resources for a VM run, with the slots of one linked program.
	struct Step {
		/// The step's parameter buffer and the layer that declares it.
		parameters: Buffer,
		layer: &'static str,
		/// The step's LUT, for the LUT step.
		lut: Option<Texture>,
	}

	/// Returns the name of the parameter buffer the layer `layer` declares.
	fn parameters_name(layer: &str) -> &'static str {
		match layer {
			"ColorCorrection" => "correction_parameters",
			_ => "lut_parameters",
		}
	}

	/// Creates `layer`'s resources for `program`: color correction's parameters, or a LUT that inverts each channel at
	/// 70% strength. Any values do, as long as the fused and the separate runs get the same ones.
	fn step(program: &ExecutableProgram, layer: &'static str) -> Step {
		let mut parameters = buffer(program, slot(program, parameters_name(layer)));
		let fields: &[(&str, [f32; 4])] = match layer {
			"ColorCorrection" => &[
				("to_working_x", [0.9, 0.1, 0.0, 0.0]),
				("to_working_y", [0.05, 0.9, 0.05, 0.0]),
				("to_working_z", [0.0, 0.1, 0.9, 0.0]),
				("from_working_x", [1.1, -0.1, 0.0, 0.0]),
				("from_working_y", [-0.05, 1.1, -0.05, 0.0]),
				("from_working_z", [0.0, -0.1, 1.1, 0.0]),
				("look", [1.3, 1.2, 0.5, 0.0]),
				("tone", [0.2, -0.1, 0.3, 0.2]),
				("split", [0.01, 0.0, -0.01, 0.0]),
			],
			_ => &[
				("domain_min", [0.0, 0.0, 0.0, 0.0]),
				("domain_scale", [1.0, 1.0, 1.0, 0.0]),
				("sampling", [0.5, 0.25, 0.7, 0.0]),
			],
		};
		for (name, value) in fields {
			parameters
				.write(name, Value::Vec4F(*value))
				.expect("Expected the step's parameters to match its shader");
		}
		let lut = (layer == "Lut").then(|| {
			let mut lut = Texture::new_3d(2, 2, 2).expect("Expected a valid test LUT extent");
			for z in 0..2 {
				for y in 0..2 {
					for x in 0..2 {
						lut.write_3d([x, y, z], [1.0 - x as f32, 1.0 - y as f32, 1.0 - z as f32, 1.0])
							.expect("Expected a valid test LUT coordinate");
					}
				}
			}
			lut
		});
		Step { parameters, layer, lut }
	}

	/// Runs the chain of `steps`, each a step shader and the layer it takes, on one pixel of `color`, with the step mask
	/// `mask` when the chain is fused, and returns the written color.
	fn run_chain(steps: &[(&str, Option<&'static str>)], color: [f32; 4], mask: u32) -> [f32; 4] {
		let program = compile(&link_pixel_chain(
			&steps.iter().map(|(shader, _)| *shader).collect::<Vec<_>>(),
		));
		let mut resources = steps
			.iter()
			.filter_map(|(_, layer)| layer.map(|layer| step(&program, layer)))
			.collect::<Vec<_>>();
		let mut push_constant = (steps.len() > 1).then(|| {
			let mut push_constant = push_constant_buffer(&program);
			push_constant
				.write("steps", Value::U32(mask))
				.expect("Expected a fused chain's step mask");
			push_constant
		});
		let mut source = texture_2d(1, 1, &[color]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(slot(&program, "source"), &mut source);
		descriptors.bind_image(output_slot(0), &mut result);
		for resource in &mut resources {
			descriptors.bind_buffer(slot(&program, parameters_name(resource.layer)), &mut resource.parameters);
			if let Some(lut) = &mut resource.lut {
				descriptors.bind_texture(slot(&program, "lut_texture"), lut);
			}
		}
		if let Some(push_constant) = &mut push_constant {
			descriptors.bind_push_constant(push_constant);
		}
		run_pixel(&program, &mut descriptors, [0, 0], PIXEL_TILE, [1, 1]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	fn assert_close(actual: [f32; 4], expected: [f32; 4], case: &str) {
		assert!(
			actual.iter().zip(expected).all(|(a, e)| (a - e).abs() <= 1e-5),
			"{case}: the fused chain wrote {actual:?}, and the steps one after another {expected:?}."
		);
	}

	const COLORS: [[f32; 4]; 4] = [
		[0.18, 0.18, 0.18, 0.5],
		[0.8, 0.2, 0.05, 1.0],
		[0.02, 0.3, 0.6, 0.25],
		[4.0, 3.0, 2.5, 1.0],
	];

	#[test]
	fn a_fused_chain_matches_its_steps_run_one_after_another() {
		for color in COLORS {
			let corrected = run_chain(&[(CORRECTION, Some("ColorCorrection"))], color, 0);
			let looked = run_chain(&[(LUT, Some("Lut"))], corrected, 0);
			let mapped = run_chain(&[(ACES, None)], looked, 0);
			let fused = run_chain(
				&[(CORRECTION, Some("ColorCorrection")), (LUT, Some("Lut")), (ACES, None)],
				color,
				0b111,
			);
			assert_close(fused, mapped, "Correction, LUT, and ACES");
		}
	}

	#[test]
	fn a_cleared_step_bit_leaves_that_step_out() {
		for color in COLORS {
			let corrected = run_chain(&[(CORRECTION, Some("ColorCorrection"))], color, 0);
			let mapped = run_chain(&[(ACES, None)], corrected, 0);
			let fused = run_chain(
				&[(CORRECTION, Some("ColorCorrection")), (LUT, Some("Lut")), (ACES, None)],
				color,
				0b101,
			);
			assert_close(fused, mapped, "Correction and ACES with the LUT skipped");
		}
		let unchanged = run_chain(&[(CORRECTION, Some("ColorCorrection")), (ACES, None)], COLORS[1], 0);
		assert_close(unchanged, COLORS[1], "Every step skipped");
	}

	/// Every fusion the engine ships links its steps and lowers to the platform shading language, as its bake does.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn every_shipped_fusion_lowers_to_the_platform_shader_language() {
		use resource_management::shader::{ShaderGenerationSettings, besl::backends::platform::PlatformShaderCompiler};

		let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
		let read_json = |id: &str| -> serde_json::Value {
			let path = assets.join(
				id.strip_prefix("byte_engine/")
					.expect("Engine assets start with `byte_engine/`"),
			);
			serde_json::from_str(&std::fs::read_to_string(&path).expect("Expected a readable engine asset"))
				.expect("Expected an engine asset of valid JSON")
		};
		let fusions = std::fs::read_dir(assets.join("rendering/fusions")).expect("Expected the engine's fusions");
		let mut count = 0;
		for entry in fusions {
			let path = entry.expect("Expected a readable fusion").path();
			let fusion: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("Expected a fusion"))
				.expect("Expected a fusion of valid JSON");
			let shaders = fusion["steps"]
				.as_array()
				.expect("Expected a fusion's steps")
				.iter()
				.map(|step| {
					let pipeline = read_json(step.as_str().expect("Expected a step ID"));
					let shader = pipeline["kind"]["shader"]
						.as_str()
						.expect("Expected a step's shader")
						.to_string();
					shader
						.strip_prefix("byte_engine/rendering/")
						.expect("Expected an engine shader")
						.to_string()
				})
				.collect::<Vec<_>>();
			let shaders = shaders.iter().map(String::as_str).collect::<Vec<_>>();
			let mut root = link_pixel_chain(&shaders);
			let name = path.display().to_string();
			let settings = ShaderGenerationSettings::tile(PIXEL_TILE, utils::Extent::square(PIXEL_TILE)).name(name.clone());
			PlatformShaderCompiler::new()
				.generate(&settings, &mut root)
				.await
				.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error}"));
			count += 1;
		}
		assert!(count >= 6, "Expected the engine's six fusions, found {count}.");
	}

	fn fusion(id: &str, steps: &[&str]) -> Fusion {
		Fusion {
			id: id.to_string(),
			steps: steps.iter().map(|step| step.to_string()).collect(),
		}
	}

	fn run(steps: std::ops::Range<usize>, pipeline: &str) -> Planned {
		Planned::Run {
			steps,
			pipeline: pipeline.to_string(),
		}
	}

	#[test]
	fn the_longest_matching_fusion_takes_the_steps() {
		let fusions = [fusion("ab", &["a", "b"]), fusion("abc", &["a", "b", "c"])];
		assert_eq!(plan(&[Some("a"), Some("b"), Some("c")], &fusions), vec![run(0..3, "abc")]);
	}

	#[test]
	fn a_pass_between_steps_ends_their_run() {
		let fusions = [fusion("ab", &["a", "b"])];
		assert_eq!(
			plan(&[Some("a"), None, Some("b")], &fusions),
			vec![run(0..1, "a"), Planned::Pass(1), run(2..3, "b")]
		);
	}

	#[test]
	fn a_step_no_fusion_takes_runs_alone() {
		let fusions = [fusion("bc", &["b", "c"])];
		assert_eq!(
			plan(&[Some("a"), Some("b"), Some("c"), Some("d")], &fusions),
			vec![run(0..1, "a"), run(1..3, "bc"), run(3..4, "d")]
		);
	}

	#[test]
	fn a_fusion_matches_only_its_steps_in_order() {
		let fusions = [fusion("ba", &["b", "a"]), fusion("ac", &["a", "c"])];
		assert_eq!(plan(&[Some("a"), Some("b")], &fusions), vec![run(0..1, "a"), run(1..2, "b")]);
	}

	#[test]
	fn without_fusions_every_step_runs_alone() {
		assert_eq!(
			plan(&[None, Some("a"), Some("b")], &[]),
			vec![Planned::Pass(0), run(1..2, "a"), run(2..3, "b")]
		);
	}
}
