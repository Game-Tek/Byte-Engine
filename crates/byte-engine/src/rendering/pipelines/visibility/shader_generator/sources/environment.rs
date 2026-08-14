pub(crate) const ENVIRONMENT_LAT_LONG_IRRADIANCE_SOURCE: &str = r#"
sample_environment_irradiance: fn (normalized_direction: vec3f) -> vec3f {
	// Material evaluation normalizes the shading normal before environment lighting.
	let environment_uv: vec2f = vec2f(
		atan2(normalized_direction.z, normalized_direction.x) * 0.15915494309189535 + 0.5,
		0.5 - asin(clamp(normalized_direction.y, 0.0 - 1.0, 1.0)) * 0.3183098861837907
	);
	let environment_extent: vec2u = texture_size(environment_irradiance);
	let environment_half_texel: f32 = 0.5 / f32(environment_extent.y);
	environment_uv.y = clamp(environment_uv.y, environment_half_texel, 1.0 - environment_half_texel);
	let environment_sample: vec4f = texture_lod(environment_irradiance, environment_uv);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

#[allow(dead_code)]
pub(crate) const ENVIRONMENT_LAT_LONG_SPECULAR_SOURCE: &str = r#"
sample_environment_specular: fn (normalized_direction: vec3f, roughness: f32) -> vec3f {
	// Reflecting a normalized view vector around a normalized shading normal preserves length.
	let environment_uv: vec2f = vec2f(
		atan2(normalized_direction.z, normalized_direction.x) * 0.15915494309189535 + 0.5,
		0.5 - asin(clamp(normalized_direction.y, 0.0 - 1.0, 1.0)) * 0.3183098861837907
	);
	let specular_level: f32 = clamp(roughness, 0.0, 1.0) * 7.0;
	let upper_level: u32 = u32(floor(specular_level)) + 1;
	if (upper_level > 7) {
		upper_level = 7;
	}
	let base_extent: vec2u = texture_size(environment_specular);
	let upper_level_scale: f32 = pow(2.0, f32(upper_level));
	let upper_half_texel: f32 = 0.5 * upper_level_scale / f32(base_extent.y);
	environment_uv.y = clamp(environment_uv.y, upper_half_texel, 1.0 - upper_half_texel);
	let environment_sample: vec4f = texture_lod(environment_specular, environment_uv, specular_level);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

pub(crate) const ENVIRONMENT_IRRADIANCE_SOURCE: &str = r#"
sample_environment_irradiance: fn (normalized_direction: vec3f) -> vec3f {
	let environment_sample: vec4f = texture_lod(environment_irradiance, normalized_direction, 0.0);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

pub(crate) const ENVIRONMENT_SPECULAR_SOURCE: &str = r#"
sample_environment_specular: fn (normalized_direction: vec3f, roughness: f32) -> vec3f {
	let specular_level: f32 = clamp(roughness, 0.0, 1.0) * 7.0;
	let environment_sample: vec4f = texture_lod(environment_specular, normalized_direction, specular_level);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;
