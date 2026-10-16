//! The optional visibility features a project builds into its baked shaders.
//!
//! The project's `config.json` selects them once, and both BELD and the running application read the same values: BELD
//! generates shaders without the code of a left-out feature and skips its pipelines, and the application never creates
//! its pass. Build the set with [`VisibilityFeatures::from_parameters`], then pass it to
//! [`super::VisibilityShaderGenerator::new`], [`crate::application::graphics::register_default_asset_handlers`], and
//! [`super::VisibilityPipelineSettings::with_features`].

use besl::parser::Node;

use crate::application::parameters::Parameters;

/// The subtree of the parameters the features, and the rest of rendering, are read from.
pub const RENDER_PARAMETERS: &str = "render";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds GTAO into the project. Its runtime value also
/// turns the pass on and off.
pub const GTAO_ENABLED_PARAMETER: &str = "gtao.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds SSGI into the project. Its runtime value also
/// turns the pass on and off.
pub const SSGI_ENABLED_PARAMETER: &str = "ssgi.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds contact shadows into the project. Its runtime
/// value also turns them on and off.
pub const CONTACT_SHADOWS_ENABLED_PARAMETER: &str = "contact-shadows.enabled";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that builds debug views into the project. Absent, debug
/// builds have them and release builds do not. See [`crate::rendering::render_pass::DebugViews`].
pub const DEBUG_VIEWS_ENABLED_PARAMETER: &str = "debug-views.enabled";

/// The `VisibilityFeatures` struct records which optional visibility features a project builds into its shaders.
///
/// Use it to keep one decision in step across baking and rendering. A left-out feature adds no code to shaders and no
/// pipelines to the resource store, and the runtime cannot turn it on.
///
/// Shaders mark a feature's code with a statement that reads a member named after it, such as
/// `if (push_constant.ssgi != 0) { ... }` or `let slot: u32 = debug_view_slot(parameters.debug_views, 0u);`. The
/// shader generators drop those statements for a left-out feature, so the bindings only they use are left out too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibilityFeatures {
	/// Whether GTAO occlusion reaches material evaluation.
	pub gtao: bool,
	/// Whether screen-space global illumination reaches material evaluation.
	pub ssgi: bool,
	/// Whether the sun visibility resolve multiplies in the contact-shadow trace.
	pub contact_shadows: bool,
	/// Whether passes can show their debug views; see [`crate::rendering::render_pass::DebugViews`].
	pub debug_views: bool,
}

impl Default for VisibilityFeatures {
	/// Every rendering feature, and debug views in debug builds only.
	fn default() -> Self {
		Self {
			gtao: true,
			ssgi: true,
			contact_shadows: true,
			debug_views: cfg!(debug_assertions),
		}
	}
}

impl VisibilityFeatures {
	/// Reads the feature set from the `render` subtree of the startup parameters, such as
	/// `app.parameters().scope("render")`, keeping every feature whose parameter is absent.
	///
	/// # Panics
	///
	/// Panics when a feature parameter is neither `true` nor `false`.
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
			contact_shadows: built_in(CONTACT_SHADOWS_ENABLED_PARAMETER, defaults.contact_shadows),
			debug_views: built_in(DEBUG_VIEWS_ENABLED_PARAMETER, defaults.debug_views),
		}
	}

	/// Returns each feature's shader member name, the directory under `byte-engine/rendering/` and the stems of the
	/// engine assets only it uses, and whether the project builds it in. Each stem names a pipeline and its shader.
	fn table(&self) -> [(&'static str, &'static str, &'static [&'static str], bool); 4] {
		use super::render_pass::{CONTACT_SHADOW_PIPELINES, GTAO_PIPELINES, SSGI_PIPELINES};
		[
			("gtao", "visibility/", &GTAO_PIPELINES, self.gtao),
			("ssgi", "visibility/", &SSGI_PIPELINES, self.ssgi),
			(
				"contact_shadows",
				"visibility/",
				&CONTACT_SHADOW_PIPELINES,
				self.contact_shadows,
			),
			("debug_views", "debug-view/", &DEBUG_VIEW_PIPELINES, self.debug_views),
		]
	}

	/// Returns whether the engine asset `id` is only used by a left-out feature, so a bake can skip it.
	pub fn excludes(&self, id: &str) -> bool {
		// Each pipeline and its shader share a stem, such as `gtao-upscale.pipeline` and `gtao-upscale.besl`.
		let Some((path, extension)) = id
			.strip_prefix("byte-engine/rendering/")
			.and_then(|file| file.rsplit_once('.'))
		else {
			return false;
		};
		matches!(extension, "pipeline" | "besl")
			&& self.table().iter().any(|(_, directory, pipelines, built_in)| {
				!built_in && path.strip_prefix(directory).is_some_and(|stem| pipelines.contains(&stem))
			})
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
			(CONTACT_SHADOWS_ENABLED_PARAMETER, "contact-shadows"),
		];
		for (parameter, pipeline) in cases {
			let features = VisibilityFeatures::from_parameters(&[Parameter::new(parameter, "false")][..]);

			for extension in ["pipeline", "besl"] {
				assert!(
					features.excludes(&format!("byte-engine/rendering/visibility/{pipeline}.{extension}")),
					"parameter: {parameter}"
				);
			}
			// Each feature leaves the others' assets alone.
			let excluded = cases
				.iter()
				.filter(|(_, pipeline)| features.excludes(&format!("byte-engine/rendering/visibility/{pipeline}.pipeline")));
			assert_eq!(excluded.count(), 1, "parameter: {parameter}");
		}
	}

	#[test]
	fn debug_views_parameter_leaves_out_the_present_pass() {
		let left_out = VisibilityFeatures::from_parameters(&[Parameter::new(DEBUG_VIEWS_ENABLED_PARAMETER, "false")][..]);
		let built_in = VisibilityFeatures::from_parameters(&[Parameter::new(DEBUG_VIEWS_ENABLED_PARAMETER, "true")][..]);

		assert!(!left_out.debug_views);
		assert!(built_in.debug_views);
		for extension in ["pipeline", "besl"] {
			let id = format!("byte-engine/rendering/debug-view/present.{extension}");
			assert!(left_out.excludes(&id));
			assert!(!built_in.excludes(&id));
		}
		assert!(!left_out.excludes("byte-engine/rendering/visibility/gtao.pipeline"));
	}

	#[test]
	fn shared_assets_stay_when_every_feature_is_left_out() {
		let features = VisibilityFeatures {
			gtao: false,
			ssgi: false,
			contact_shadows: false,
			debug_views: false,
		};

		// The depth pyramid and the sun visibility resolve also serve passes that always run.
		for asset in ["depth-pyramid.besl", "sun-visibility.pipeline", "sun-visibility.besl"] {
			assert!(
				!features.excludes(&format!("byte-engine/rendering/visibility/{asset}")),
				"asset: {asset}"
			);
		}
	}

	#[test]
	fn missing_parameters_keep_every_feature() {
		let features = VisibilityFeatures::from_parameters(&[][..] as &[Parameter]);

		assert_eq!(features, VisibilityFeatures::default());
		assert!(!features.excludes("byte-engine/rendering/visibility/gtao.pipeline"));
	}
}
