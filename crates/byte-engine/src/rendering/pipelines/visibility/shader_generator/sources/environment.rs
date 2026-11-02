pub(crate) const ENVIRONMENT_IRRADIANCE_SOURCE: &str = r#"
sample_environment_irradiance: fn (material_evaluation: MaterialEvaluation, normalized_direction: vec3f) -> vec3f {
	let environment_sample: vec4f = texture_lod(material_evaluation.environment_irradiance, normalized_direction, 0.0);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;

pub(crate) const ENVIRONMENT_SPECULAR_SOURCE: &str = r#"
sample_environment_specular: fn (material_evaluation: MaterialEvaluation, normalized_direction: vec3f, roughness: f32) -> vec3f {
	let specular_level: f32 = clamp(roughness, 0.0, 1.0) * 7.0;
	let environment_sample: vec4f = texture_lod(material_evaluation.environment_specular, normalized_direction, specular_level);
	return vec3f(environment_sample.x, environment_sample.y, environment_sample.z);
}
"#;
