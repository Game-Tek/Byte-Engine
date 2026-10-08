//! The optional visibility features a project builds into its baked shaders.
//!
//! The project's `config.json` selects them once, and both BELD and the running application read the same values: BELD
//! generates material shaders without the code of a left-out feature and skips its pipelines, and the application never
//! creates its pass. Build the set with [`VisibilityFeatures::from_parameters`], then pass it to
//! [`super::VisibilityShaderGenerator::new`] and [`super::VisibilityPipelineSettings::with_features`].

use crate::application::parameters::Parameters;

/// The startup parameter that builds GTAO into the project. Its runtime value also turns the pass on and off.
pub const GTAO_ENABLED_PARAMETER: &str = "render.gtao.enabled";

/// The `VisibilityFeatures` struct records which optional visibility features a project builds into its shaders.
///
/// Use it to keep one decision in step across baking and rendering. A left-out feature adds no code to material
/// shaders and no pipelines to the resource store, and the runtime cannot turn it on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibilityFeatures {
	/// Whether GTAO occlusion reaches material evaluation.
	pub gtao: bool,
}

impl Default for VisibilityFeatures {
	fn default() -> Self {
		Self { gtao: true }
	}
}

impl VisibilityFeatures {
	/// Reads the feature set from startup parameters, keeping every feature whose parameter is absent.
	///
	/// # Panics
	///
	/// Panics when a feature parameter is neither `true` nor `false`.
	pub fn from_parameters(parameters: &(impl Parameters + ?Sized)) -> Self {
		let gtao = parameters.get_parameter(GTAO_ENABLED_PARAMETER).is_none_or(|parameter| {
			parameter.as_bool().unwrap_or_else(|| {
				panic!(
					"Parameter `{GTAO_ENABLED_PARAMETER}` is invalid. The most likely cause is that `{}` is neither `true` nor `false`.",
					parameter.value()
				)
			})
		});
		Self { gtao }
	}

	/// Returns whether the engine asset `id` is only used by a left-out feature, so a bake can skip it.
	pub fn excludes(&self, id: &str) -> bool {
		// Each GTAO pipeline and its shader share a stem, such as `gtao-upscale.pipeline` and `gtao-upscale.besl`.
		let gtao_asset = id
			.strip_prefix("byte-engine/rendering/visibility/")
			.and_then(|file| file.rsplit_once('.'))
			.is_some_and(|(stem, extension)| {
				matches!(extension, "pipeline" | "besl") && super::render_pass::GTAO_PIPELINES.contains(&stem)
			});
		!self.gtao && gtao_asset
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::application::Parameter;

	#[test]
	fn gtao_parameter_selects_the_feature_and_its_assets() {
		let parameters = [Parameter::new(GTAO_ENABLED_PARAMETER, "false")];

		let features = VisibilityFeatures::from_parameters(&parameters[..]);

		assert_eq!(features, VisibilityFeatures { gtao: false });
		assert!(features.excludes("byte-engine/rendering/visibility/gtao.pipeline"));
		assert!(features.excludes("byte-engine/rendering/visibility/gtao-upscale.besl"));
		// The depth pyramid also serves contact shadows, SSGI, and the cascade fit.
		assert!(!features.excludes("byte-engine/rendering/visibility/depth-pyramid.besl"));
	}

	#[test]
	fn missing_parameters_keep_every_feature() {
		let features = VisibilityFeatures::from_parameters(&[][..]);

		assert_eq!(features, VisibilityFeatures::default());
		assert!(!features.excludes("byte-engine/rendering/visibility/gtao.pipeline"));
	}
}
