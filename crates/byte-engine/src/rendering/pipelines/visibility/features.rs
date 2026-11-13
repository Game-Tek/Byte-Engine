//! The optional visibility features a project builds into its baked shaders.
//!
//! The project's `config.json` selects them once, and both BELD and the running application read the same values: BELD
//! generates shaders without the code of a left-out feature and skips its pipelines, and the application never creates
//! its pass. Build the set with [`VisibilityFeatures::from_parameters`], then pass it to
//! [`super::VisibilityShaderGenerator::new`], [`crate::application::graphics::configure_default_baking`], and
//! [`super::VisibilityPipelineSettings::with_features`].

use besl::parser::Node;

use crate::application::parameters::Parameters;

/// The subtree of the parameters the features, and the rest of rendering, are read from.
pub const RENDER_PARAMETERS: &str = "render";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds GTAO into the project. Its runtime value also
/// turns the pass on and off.
pub const GTAO_ENABLED_PARAMETER: &str = "gtao.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds SSGI into the project, so `gi.technique` can
/// select it.
pub const SSGI_ENABLED_PARAMETER: &str = "ssgi.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds surfel GI into the project, so `gi.technique` can
/// select it.
pub const SURFEL_GI_ENABLED_PARAMETER: &str = "surfel-gi.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds probe GI into the project, so `gi.technique` can
/// select it.
pub const PROBE_GI_ENABLED_PARAMETER: &str = "probe-gi.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds lightmaps into the project: lightmap UVs in mesh
/// bakes, closest-hit shaders for the lightmap bake, and lightmap reads in material evaluation, so `gi.technique` can
/// select `baked`.
pub const BAKED_GI_ENABLED_PARAMETER: &str = "baked-gi.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds contact shadows into the project. Its runtime
/// value also turns them on and off.
pub const CONTACT_SHADOWS_ENABLED_PARAMETER: &str = "contact-shadows.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that caps how many directional lights the project uses. When
/// it is `1` or less, the project builds the `single_directional_light` feature in. No shader reads that feature yet.
pub const DIRECTIONAL_LIGHTS_MAX_PARAMETER: &str = "lights.directional.max";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds debug views into the project. Absent, debug
/// builds have them and release builds do not. See [`crate::rendering::DebugViews`].
pub const DEBUG_VIEWS_ENABLED_PARAMETER: &str = "debug-views.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that chooses how material evaluation runs: `pipelines` or
/// `uber`. See [`MaterialEvaluationMode`].
pub const MATERIAL_EVALUATION_PARAMETER: &str = "material-evaluation";

/// The `MaterialEvaluationMode` enum chooses how the visibility pipeline shades the materials the visibility buffer
/// recorded. The bake and the runtime must agree on it, so it is part of [`VisibilityFeatures`].
///
/// Set it with [`MATERIAL_EVALUATION_PARAMETER`] in `config.json`, then rebake every material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaterialEvaluationMode {
	/// Each material program bakes its own evaluation shader. Three prepasses list each pipeline's pixels, and each
	/// pipeline shades its own list.
	#[default]
	Pipelines,
	/// The bake joins every material variant into one shader per phase, which one pass of screen tiles runs without
	/// prepasses. See [`super::VisibilityShaderGenerator::uber_program`].
	Uber,
}

impl std::str::FromStr for MaterialEvaluationMode {
	type Err = String;

	fn from_str(name: &str) -> Result<Self, Self::Err> {
		match name {
			"pipelines" => Ok(Self::Pipelines),
			"uber" => Ok(Self::Uber),
			_ => Err(format!(
				"Parameter `{RENDER_PARAMETERS}.{MATERIAL_EVALUATION_PARAMETER}` is invalid. The most likely cause is that `{name}` is neither `pipelines` nor `uber`. Set it to one of the two in `config.json`."
			)),
		}
	}
}

/// The IDs of the uber material evaluation shaders, in the order of [`UberVariant::ALL`], which only
/// [`MaterialEvaluationMode::Uber`] uses. They are resources with no asset behind them: the bake assembles each from
/// every material variant of its kind the store holds.
pub(crate) const UBER_MATERIAL_SHADERS: [&str; 4] = [
	ASSEMBLED_SHADERS[2],
	ASSEMBLED_SHADERS[3],
	ASSEMBLED_SHADERS[4],
	ASSEMBLED_SHADERS[5],
];

/// The `UberVariant` struct names one uber material evaluation shader: the phase it shades, and whether it holds the
/// translucent materials of that phase or the others.
///
/// Translucent materials light their surface from its far side too, which costs every pixel a shader shades, so they
/// get shaders of their own and no other material runs that code. Use it to index the per-shader state of the bake,
/// the loader, and the renderer with [`Self::index`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct UberVariant {
	/// The transparent phase's shader when set, the opaque phase's otherwise.
	pub transparent: bool,
	/// The shader of the phase's translucent materials when set, see
	/// [`resource_management::bemx::Graph::is_translucent`].
	pub translucent: bool,
}

impl UberVariant {
	/// Every variant, in the order of [`UBER_MATERIAL_SHADERS`] and of [`Self::index`].
	pub(crate) const ALL: [Self; 4] = [
		Self::new(false, false),
		Self::new(true, false),
		Self::new(false, true),
		Self::new(true, true),
	];

	pub(crate) const fn new(transparent: bool, translucent: bool) -> Self {
		Self {
			transparent,
			translucent,
		}
	}

	/// Returns the variant whose shader holds the arm `key` of a material of the transparent phase when `transparent`
	/// is set, of the opaque one otherwise. The key's lowest bit marks a translucent material; see
	/// [`super::shader_generator::uber_arm_key`].
	pub(crate) const fn of_arm(key: u32, transparent: bool) -> Self {
		Self::new(transparent, key & 1 != 0)
	}

	/// Returns the variant's position in [`Self::ALL`].
	pub(crate) const fn index(self) -> usize {
		self.transparent as usize + 2 * self.translucent as usize
	}

	/// The shader that writes the background: the opaque phase's shader of the materials that are not translucent.
	/// Material evaluation dispatches it every frame.
	pub(crate) const BACKGROUND: Self = Self::new(false, false);

	/// Returns the variant's evaluation slot in its phase's material list. A phase lists at most its two shaders, and
	/// an instance activates the slot of its material's, so a frame without translucent instances skips their shader.
	pub(crate) const fn evaluation_index(self) -> u32 {
		self.translucent as u32
	}

	/// Returns the variant's shader ID.
	pub(crate) const fn shader(self) -> &'static str {
		UBER_MATERIAL_SHADERS[self.index()]
	}
}

/// The IDs of the coverage shaders, the masked visibility pass's fragment then the masked shadow passes', which every
/// mode uses. The bake assembles each from every masked material variant the store holds, with one arm per distinct
/// `opacity` program.
pub(crate) const COVERAGE_SHADERS: [&str; 2] = [ASSEMBLED_SHADERS[0], ASSEMBLED_SHADERS[1]];

/// Every assembled shader: the [`COVERAGE_SHADERS`], which every mode bakes, then the [`UBER_MATERIAL_SHADERS`].
const ASSEMBLED_SHADERS: [&str; 6] = [
	"byte_engine/rendering/visibility/masked-visibility-coverage.uber",
	"byte_engine/rendering/visibility/masked-depth-coverage.uber",
	"byte_engine/rendering/visibility/material-evaluation-opaque.uber",
	"byte_engine/rendering/visibility/material-evaluation-transparent.uber",
	"byte_engine/rendering/visibility/material-evaluation-opaque-translucent.uber",
	"byte_engine/rendering/visibility/material-evaluation-transparent-translucent.uber",
];

/// The `VisibilityFeatures` struct records which optional visibility features a project builds into its shaders.
///
/// Use it to keep one decision in step across baking and rendering. A left-out feature adds no code to shaders and no
/// pipelines to the resource store, and the runtime cannot turn it on.
///
/// Shaders mark a feature's code with a statement that reads a member named after it, such as
/// `if (push_constant.ssgi != 0) { ... }`, or for debug views any member starting with `debug_view_`, such as
/// `if (parameters.debug_view_normals) { ... }`. The shader generators drop those statements for a left-out feature,
/// so the bindings only they use are left out too.
/// Code that serves every source of indirect diffuse light, SSGI, surfel GI, probe GI, or lightmaps, reads
/// `indirect_diffuse` and stays while any of them is built in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibilityFeatures {
	/// Whether GTAO occlusion reaches material evaluation.
	pub gtao: bool,
	/// Whether screen-space global illumination reaches material evaluation.
	pub ssgi: bool,
	/// Whether ray-traced surfel global illumination reaches material evaluation, in place of SSGI on a device that
	/// traces rays.
	pub surfel_gi: bool,
	/// Whether ray-traced probe global illumination reaches material evaluation, in place of SSGI on a device that
	/// traces rays.
	pub probe_gi: bool,
	/// Whether lightmaps BELD bakes from levels reach material evaluation, and meshes bake the lightmap UVs they need.
	pub baked_gi: bool,
	/// Whether the sun visibility resolve multiplies in the contact-shadow trace.
	pub contact_shadows: bool,
	/// Whether the project uses at most one directional light. No shader reads it yet; it exists so the baked shaders
	/// can skip their multi-light loops.
	pub single_directional_light: bool,
	/// Whether passes can show their debug views; see [`crate::rendering::DebugViews`].
	pub debug_views: bool,
	/// How material evaluation runs, which decides what material bakes store.
	pub material_evaluation: MaterialEvaluationMode,
}

impl Default for VisibilityFeatures {
	/// Every rendering feature but the ray-traced GI techniques, and debug views in debug builds only.
	fn default() -> Self {
		Self {
			gtao: true,
			ssgi: true,
			surfel_gi: false,
			probe_gi: false,
			baked_gi: false,
			contact_shadows: true,
			single_directional_light: false,
			debug_views: cfg!(debug_assertions),
			material_evaluation: MaterialEvaluationMode::default(),
		}
	}
}

impl VisibilityFeatures {
	/// Reads the feature set from the `render` subtree of the startup parameters, such as
	/// `app.parameters().scope("render")`, keeping every feature whose parameter is absent.
	///
	/// # Panics
	///
	/// Panics when a feature parameter is neither `true` nor `false`, or the material evaluation mode is neither
	/// `pipelines` nor `uber`.
	pub fn from_parameters(parameters: impl Parameters) -> Self {
		let defaults = Self::default();
		let built_in = |name: &str, default: bool| {
			parameters.get_parameter(name).map_or(default, |parameter| {
				parameter.as_bool().unwrap_or_else(|| {
					panic!(
						"Parameter `{RENDER_PARAMETERS}.{name}` is invalid. The most likely cause is that `{}` is neither `true` nor `false`.",
						parameter.value()
					)
				})
			})
		};
		Self {
			gtao: built_in(GTAO_ENABLED_PARAMETER, defaults.gtao),
			ssgi: built_in(SSGI_ENABLED_PARAMETER, defaults.ssgi),
			surfel_gi: built_in(SURFEL_GI_ENABLED_PARAMETER, defaults.surfel_gi),
			probe_gi: built_in(PROBE_GI_ENABLED_PARAMETER, defaults.probe_gi),
			baked_gi: built_in(BAKED_GI_ENABLED_PARAMETER, defaults.baked_gi),
			contact_shadows: built_in(CONTACT_SHADOWS_ENABLED_PARAMETER, defaults.contact_shadows),
			single_directional_light: parameters.get_parameter(DIRECTIONAL_LIGHTS_MAX_PARAMETER).map_or(
				defaults.single_directional_light,
				|parameter| {
					parameter
						.parse::<usize>()
						.map_or(defaults.single_directional_light, |max| max <= 1)
				},
			),
			debug_views: built_in(DEBUG_VIEWS_ENABLED_PARAMETER, defaults.debug_views),
			material_evaluation: parameters
				.get_parameter(MATERIAL_EVALUATION_PARAMETER)
				.map_or(defaults.material_evaluation, |parameter| {
					parameter.value().parse().unwrap_or_else(|error: String| panic!("{error}"))
				}),
		}
	}

	/// Returns whether a built-in technique traces rays, so materials need closest-hit shaders and the device needs ray
	/// tracing.
	pub fn traces_rays(&self) -> bool {
		self.surfel_gi || self.probe_gi
	}

	/// Returns whether materials need closest-hit shaders: a built-in technique traces rays at runtime, or the lightmap
	/// bake traces them in BELD.
	pub fn generates_hit_shaders(&self) -> bool {
		self.traces_rays() || self.baked_gi
	}

	/// Returns each feature's shader member name, or a prefix of member names when it ends in `_`, the directory under
	/// `byte_engine/rendering/` and the stems of the engine assets only it uses, and whether the project builds it in.
	/// Each stem names a pipeline and its shader, or a shader the engine assembles a pipeline from, which takes a row of
	/// its own.
	fn table(&self) -> [(&'static str, &'static str, &'static [&'static str], bool); 14] {
		use super::ray_tracing::{DEFAULT_HIT_SHADERS, PROBE_SHADING_SHADERS, SURFEL_SHADING_SHADERS};
		use super::render_pass::{
			CONTACT_SHADOW_PIPELINES, GTAO_PIPELINES, MATERIAL_PREPASS_PIPELINES, PROBE_GI_PIPELINES, RAY_HIT_PIPELINES,
			SSGI_PIPELINES, SURFEL_GI_PIPELINES,
		};
		[
			("gtao", "visibility/", &GTAO_PIPELINES, self.gtao),
			("ssgi", "visibility/", &SSGI_PIPELINES, self.ssgi),
			("surfel_gi", "visibility/", &SURFEL_GI_PIPELINES, self.surfel_gi),
			("surfel_gi", "visibility/", &SURFEL_SHADING_SHADERS, self.surfel_gi),
			("probe_gi", "visibility/", &PROBE_GI_PIPELINES, self.probe_gi),
			("probe_gi", "visibility/", &PROBE_SHADING_SHADERS, self.probe_gi),
			// Every ray tracing pipeline, the lightmap bake's included, runs the default closest hit for the materials whose
			// own hit group it lacks, and both runtime techniques sort their hits with the same pipelines.
			(
				"traces_rays",
				"visibility/",
				&DEFAULT_HIT_SHADERS,
				self.generates_hit_shaders(),
			),
			("traces_rays", "visibility/", &RAY_HIT_PIPELINES, self.traces_rays()),
			// Material evaluation reads indirect diffuse light from the same images whichever source wrote them.
			(
				"indirect_diffuse",
				"visibility/",
				&[],
				self.ssgi || self.surfel_gi || self.probe_gi || self.baked_gi,
			),
			// Material evaluation samples `lightmaps` only in its lightmap branch, and BELD bakes them with the lightmap
			// shaders.
			(
				"lightmaps",
				"lightmaps/",
				&crate::rendering::lightmaps::LIGHTMAP_BAKE_SHADERS,
				self.baked_gi,
			),
			(
				"contact_shadows",
				"visibility/",
				&CONTACT_SHADOW_PIPELINES,
				self.contact_shadows,
			),
			("single_directional_light", "visibility/", &[], self.single_directional_light),
			("debug_view_", "debug-view/", &DEBUG_VIEW_PIPELINES, self.debug_views),
			// Only per-material evaluation shaders read the pixel lists the prepasses write.
			(
				"pixel_mapping",
				"visibility/",
				&MATERIAL_PREPASS_PIPELINES,
				self.material_evaluation == MaterialEvaluationMode::Pipelines,
			),
		]
	}

	/// Returns whether the engine asset `id` is only used by a left-out feature, so a bake can skip it.
	pub fn excludes(&self, id: &str) -> bool {
		// Each pipeline and its shader share a stem, such as `gtao-upscale.pipeline` and `gtao-upscale.besl`.
		let Some((path, extension)) = id
			.strip_prefix("byte_engine/rendering/")
			.and_then(|file| file.rsplit_once('.'))
		else {
			return false;
		};
		matches!(extension, "pipeline" | "besl")
			&& self.table().iter().any(|(_, directory, pipelines, built_in)| {
				!built_in && path.strip_prefix(directory).is_some_and(|stem| pipelines.contains(&stem))
			})
	}

	/// Returns the IDs of the resources a bake assembles from every material variant the store holds, which it must
	/// bake after every material, and every time, because a freshness check cannot see the set of materials change.
	pub fn assembled_resources(&self) -> &'static [&'static str] {
		match self.material_evaluation {
			MaterialEvaluationMode::Pipelines => &ASSEMBLED_SHADERS[..COVERAGE_SHADERS.len()],
			MaterialEvaluationMode::Uber => &ASSEMBLED_SHADERS,
		}
	}

	/// Removes the code of every left-out feature from `statements`, so a shader carries none of it.
	pub(crate) fn remove_left_out_code(&self, statements: &mut Vec<Node<'_>>) {
		for (member, _, _, built_in) in self.table() {
			if !built_in {
				super::shader_generator::remove_statements_reading(statements, member);
			}
		}
	}
}

/// The engine assets only debug views use: the pass that tiles the shown views into the output.
const DEBUG_VIEW_PIPELINES: [&str; 1] = ["present"];

#[cfg(test)]
mod tests {
	use super::*;
	use crate::application::Parameter;

	#[test]
	fn each_parameter_leaves_out_its_feature_and_its_assets() {
		let cases = [
			(GTAO_ENABLED_PARAMETER, "gtao-upscale"),
			(SSGI_ENABLED_PARAMETER, "ssgi-temporal"),
			(SURFEL_GI_ENABLED_PARAMETER, "surfel-resolve"),
			(CONTACT_SHADOWS_ENABLED_PARAMETER, "contact-shadows"),
		];
		for (parameter, pipeline) in cases {
			// The first value of a name wins, so the case's own parameter turns its feature off, and surfel GI, which
			// is off by default, is on for the others.
			let features = VisibilityFeatures::from_parameters(
				&[
					Parameter::new(parameter, "false"),
					Parameter::new(SURFEL_GI_ENABLED_PARAMETER, "true"),
				][..],
			);

			for extension in ["pipeline", "besl"] {
				assert!(
					features.excludes(&format!("byte_engine/rendering/visibility/{pipeline}.{extension}")),
					"parameter: {parameter}"
				);
			}
			// Each feature leaves the others' assets alone.
			let excluded = cases
				.iter()
				.filter(|(_, pipeline)| features.excludes(&format!("byte_engine/rendering/visibility/{pipeline}.pipeline")));
			assert_eq!(excluded.count(), 1, "parameter: {parameter}");
		}
	}

	#[test]
	fn surfel_gi_parameter_leaves_out_every_surfel_asset() {
		let left_out = VisibilityFeatures::from_parameters(&[Parameter::new(SURFEL_GI_ENABLED_PARAMETER, "false")][..]);
		let built_in = VisibilityFeatures::from_parameters(&[Parameter::new(SURFEL_GI_ENABLED_PARAMETER, "true")][..]);
		let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/visibility");
		let assets: Vec<String> = std::fs::read_dir(directory)
			.expect("the visibility assets")
			.map(|entry| {
				entry
					.expect("a visibility asset")
					.file_name()
					.into_string()
					.expect("a UTF-8 file name")
			})
			.filter(|name| name.starts_with("surfel-") && (name.ends_with(".besl") || name.ends_with(".pipeline")))
			.collect();

		// The compute passes have pipeline assets, and the shade pass's ray tracing shaders have only their shaders.
		assert!(assets.iter().any(|asset| asset == "surfel-count.pipeline"), "{assets:?}");
		assert!(assets.iter().any(|asset| asset == "surfel-shade.besl"), "{assets:?}");
		for asset in &assets {
			let id = format!("byte_engine/rendering/visibility/{asset}");
			assert!(left_out.excludes(&id), "a bake without surfel GI should leave out {asset}");
			assert!(!built_in.excludes(&id), "a bake with surfel GI should keep {asset}");
		}
	}

	#[test]
	fn probe_gi_parameter_leaves_out_every_probe_asset() {
		let left_out = VisibilityFeatures::from_parameters(&[Parameter::new(PROBE_GI_ENABLED_PARAMETER, "false")][..]);
		let built_in = VisibilityFeatures::from_parameters(&[Parameter::new(PROBE_GI_ENABLED_PARAMETER, "true")][..]);
		let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/visibility");
		let assets: Vec<String> = std::fs::read_dir(directory)
			.expect("the visibility assets")
			.map(|entry| {
				entry
					.expect("a visibility asset")
					.file_name()
					.into_string()
					.expect("a UTF-8 file name")
			})
			.filter(|name| name.starts_with("probe-") && (name.ends_with(".besl") || name.ends_with(".pipeline")))
			.collect();

		// The compute passes have pipeline assets, and the shade pass's ray tracing shader has only its shader.
		assert!(assets.iter().any(|asset| asset == "probe-update.pipeline"), "{assets:?}");
		assert!(assets.iter().any(|asset| asset == "probe-shade.besl"), "{assets:?}");
		for asset in &assets {
			let id = format!("byte_engine/rendering/visibility/{asset}");
			assert!(left_out.excludes(&id), "a bake without probe GI should leave out {asset}");
			assert!(!built_in.excludes(&id), "a bake with probe GI should keep {asset}");
		}
	}

	#[test]
	fn the_default_hit_stays_while_either_technique_traces_rays() {
		let default_hit = "byte_engine/rendering/visibility/default-hit.besl";
		let features = |surfel_gi, probe_gi| VisibilityFeatures {
			surfel_gi,
			probe_gi,
			..VisibilityFeatures::default()
		};

		assert!(features(false, false).excludes(default_hit));
		assert!(!features(true, false).excludes(default_hit));
		assert!(!features(false, true).excludes(default_hit));
		// Each technique keeps only its own shade pass.
		assert!(features(false, true).excludes("byte_engine/rendering/visibility/surfel-shade.besl"));
		assert!(features(true, false).excludes("byte_engine/rendering/visibility/probe-shade.besl"));
	}

	#[test]
	fn debug_views_parameter_leaves_out_the_present_pass() {
		let left_out = VisibilityFeatures::from_parameters(&[Parameter::new(DEBUG_VIEWS_ENABLED_PARAMETER, "false")][..]);
		let built_in = VisibilityFeatures::from_parameters(&[Parameter::new(DEBUG_VIEWS_ENABLED_PARAMETER, "true")][..]);

		assert!(!left_out.debug_views);
		assert!(built_in.debug_views);
		for extension in ["pipeline", "besl"] {
			let id = format!("byte_engine/rendering/debug-view/present.{extension}");
			assert!(left_out.excludes(&id));
			assert!(!built_in.excludes(&id));
		}
		assert!(!left_out.excludes("byte_engine/rendering/visibility/gtao.pipeline"));
	}

	#[test]
	fn surfel_gi_is_built_in_only_when_the_project_enables_it() {
		let enabled = VisibilityFeatures::from_parameters(&[Parameter::new(SURFEL_GI_ENABLED_PARAMETER, "true")][..]);

		assert!(!VisibilityFeatures::from_parameters(&[][..] as &[Parameter]).surfel_gi);
		assert!(enabled.surfel_gi);
		// Surfel GI takes the place of SSGI only at runtime, so building it in leaves SSGI and its assets in.
		assert!(enabled.ssgi);
		assert!(!enabled.excludes("byte_engine/rendering/visibility/ssgi-trace.pipeline"));
	}

	#[test]
	fn shared_assets_stay_when_every_feature_is_left_out() {
		let features = VisibilityFeatures {
			gtao: false,
			ssgi: false,
			surfel_gi: false,
			probe_gi: false,
			baked_gi: false,
			contact_shadows: false,
			single_directional_light: false,
			debug_views: false,
			material_evaluation: MaterialEvaluationMode::Pipelines,
		};

		// The depth pyramid and the sun visibility resolve also serve passes that always run.
		for asset in ["depth-pyramid.besl", "sun-visibility.pipeline", "sun-visibility.besl"] {
			assert!(
				!features.excludes(&format!("byte_engine/rendering/visibility/{asset}")),
				"asset: {asset}"
			);
		}
	}

	#[test]
	fn material_evaluation_mode_keeps_its_own_assets() {
		let mode = |value| VisibilityFeatures::from_parameters(&[Parameter::new(MATERIAL_EVALUATION_PARAMETER, value)][..]);
		let (pipelines, uber) = (mode("pipelines"), mode("uber"));

		assert_eq!(pipelines.material_evaluation, MaterialEvaluationMode::Pipelines);
		assert_eq!(uber.material_evaluation, MaterialEvaluationMode::Uber);
		for asset in ["pixel-mapping.pipeline", "material-count.besl", "material-offset.pipeline"] {
			let id = format!("byte_engine/rendering/visibility/{asset}");
			assert!(!pipelines.excludes(&id), "asset: {asset}");
			assert!(uber.excludes(&id), "asset: {asset}");
		}
		assert_eq!(pipelines.assembled_resources(), COVERAGE_SHADERS);
		assert_eq!(
			uber.assembled_resources(),
			[&COVERAGE_SHADERS[..], &UBER_MATERIAL_SHADERS[..]].concat()
		);
	}

	#[test]
	#[should_panic(expected = "neither `pipelines` nor `uber`")]
	fn material_evaluation_mode_rejects_other_names() {
		VisibilityFeatures::from_parameters(&[Parameter::new(MATERIAL_EVALUATION_PARAMETER, "ubershader")][..]);
	}

	#[test]
	fn missing_parameters_keep_the_default_features() {
		let features = VisibilityFeatures::from_parameters(&[][..] as &[Parameter]);

		assert_eq!(features, VisibilityFeatures::default());
		assert!(!features.excludes("byte_engine/rendering/visibility/gtao.pipeline"));
	}
}
