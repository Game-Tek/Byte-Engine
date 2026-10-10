//! The optional visibility features a project builds into its baked shaders.
//!
//! The project's `config.json` selects them once, and both BELD and the running application read the same values: BELD
//! generates shaders without the code of a left-out feature and skips its pipelines, and the application never creates
//! its pass. Build the set with [`VisibilityFeatures::from_parameters`], then pass it to
//! [`super::VisibilityShaderGenerator::new`], [`crate::application::graphics::register_default_asset_handlers`], and
//! [`super::VisibilityPipelineSettings::with_features`].

use besl::parser::Node;

use crate::application::parameters::Parameters;

/// The startup parameter that builds GTAO into the project. Its runtime value also turns the pass on and off.
pub const GTAO_ENABLED_PARAMETER: &str = "render.gtao.enabled";
/// The startup parameter that builds SSGI into the project. Its runtime value also turns the pass on and off.
pub const SSGI_ENABLED_PARAMETER: &str = "render.ssgi.enabled";
/// The startup parameter that builds contact shadows into the project. Its runtime value also turns them on and off.
pub const CONTACT_SHADOWS_ENABLED_PARAMETER: &str = "render.contact-shadows.enabled";

/// The `VisibilityFeatures` struct records which optional visibility features a project builds into its shaders.
///
/// Use it to keep one decision in step across baking and rendering. A left-out feature adds no code to shaders and no
/// pipelines to the resource store, and the runtime cannot turn it on.
///
/// Shaders mark a feature's code with a conditional that reads a member named after it, such as
/// `if (push_constant.ssgi != 0) { ... }` or `if (parameters.contact_shadows != 0) { ... }`. The shader generators drop
/// those conditionals for a left-out feature, so the bindings only they use are left out too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibilityFeatures {
	/// Whether GTAO occlusion reaches material evaluation.
	pub gtao: bool,
	/// Whether screen-space global illumination reaches material evaluation.
	pub ssgi: bool,
	/// Whether the sun visibility resolve multiplies in the contact-shadow trace.
	pub contact_shadows: bool,
}

impl Default for VisibilityFeatures {
	fn default() -> Self {
		Self {
			gtao: true,
			ssgi: true,
			contact_shadows: true,
		}
	}
}

impl VisibilityFeatures {
	/// Reads the feature set from startup parameters, keeping every feature whose parameter is absent.
	///
	/// # Panics
	///
	/// Panics when a feature parameter is neither `true` nor `false`.
	pub fn from_parameters(parameters: &(impl Parameters + ?Sized)) -> Self {
		let built_in = |name: &str| {
			parameters.get_parameter(name).is_none_or(|parameter| {
				parameter.as_bool().unwrap_or_else(|| {
					panic!(
						"Parameter `{name}` is invalid. The most likely cause is that `{}` is neither `true` nor `false`.",
						parameter.value()
					)
				})
			})
		};
		Self {
			gtao: built_in(GTAO_ENABLED_PARAMETER),
			ssgi: built_in(SSGI_ENABLED_PARAMETER),
			contact_shadows: built_in(CONTACT_SHADOWS_ENABLED_PARAMETER),
		}
	}

	/// Returns each feature's shader member name, pipeline names, and whether the project builds it in.
	fn table(&self) -> [(&'static str, &'static [&'static str], bool); 3] {
		use super::render_pass::{CONTACT_SHADOW_PIPELINES, GTAO_PIPELINES, SSGI_PIPELINES};
		[
			("gtao", &GTAO_PIPELINES, self.gtao),
			("ssgi", &SSGI_PIPELINES, self.ssgi),
			("contact_shadows", &CONTACT_SHADOW_PIPELINES, self.contact_shadows),
		]
	}

	/// Returns whether the engine asset `id` is only used by a left-out feature, so a bake can skip it.
	pub fn excludes(&self, id: &str) -> bool {
		// Each pipeline and its shader share a stem, such as `gtao-upscale.pipeline` and `gtao-upscale.besl`.
		let Some((stem, extension)) = id
			.strip_prefix("byte-engine/rendering/visibility/")
			.and_then(|file| file.rsplit_once('.'))
		else {
			return false;
		};
		matches!(extension, "pipeline" | "besl")
			&& self
				.table()
				.iter()
				.any(|(_, pipelines, built_in)| !built_in && pipelines.contains(&stem))
	}

	/// Removes the code of every left-out feature from `statements`, so a shader carries none of it.
	pub(crate) fn remove_left_out_code(&self, statements: &mut Vec<Node<'_>>) {
		for (member, _, built_in) in self.table() {
			if !built_in {
				super::shader_generator::remove_conditionals_reading(statements, member);
			}
		}
	}
}

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
	fn shared_assets_stay_when_every_feature_is_left_out() {
		let features = VisibilityFeatures {
			gtao: false,
			ssgi: false,
			contact_shadows: false,
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
		let features = VisibilityFeatures::from_parameters(&[][..]);

		assert_eq!(features, VisibilityFeatures::default());
		assert!(!features.excludes("byte-engine/rendering/visibility/gtao.pipeline"));
	}
}
