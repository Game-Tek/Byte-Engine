use super::{besl::backends::platform::PlatformShaderLanguage, hlsl_shader_compiler::compile_hlsl_source_to_dxil};
use crate::{resources::material::ShaderArtifact, types::ShaderTypes};

/// Maps a platform compiler output to the artifact kind persisted in resources.
fn platform_shader_artifact(language: PlatformShaderLanguage) -> ShaderArtifact {
	match language {
		PlatformShaderLanguage::Glsl => ShaderArtifact::Spirv,
		PlatformShaderLanguage::Hlsl => ShaderArtifact::Dxil,
		PlatformShaderLanguage::Msl => ShaderArtifact::Mtlb {
			entry_point: language.entry_point().to_string(),
		},
	}
}

/// Converts generated platform output into the binary representation expected by release runtimes.
pub(crate) fn finalize_platform_shader_artifact(
	language: PlatformShaderLanguage,
	stage: ShaderTypes,
	name: &str,
	payload: Box<[u8]>,
) -> Result<(ShaderArtifact, Box<[u8]>), String> {
	let artifact = platform_shader_artifact(language);
	let payload = if language.is_hlsl() {
		let entry_point = language.entry_point();
		let source = std::str::from_utf8(&payload).map_err(|_| {
			format!(
				"Failed to read generated HLSL for shader '{name}'. The most likely cause is that the HLSL backend emitted non-UTF-8 source."
			)
		})?;
		compile_hlsl_source_to_dxil(source, name, entry_point, stage)?
	} else {
		payload
	};

	Ok((artifact, payload))
}
