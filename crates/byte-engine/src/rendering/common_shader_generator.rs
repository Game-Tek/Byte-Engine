use resource_management::asset::handler::implementations::besl::ProgramTransform;

use crate::application::parameters::Parameters;
use crate::rendering::pipelines::visibility::VisibilityFeatures;

// Keeping the shared helpers in portable BESL makes their VM tests exercise the
// same implementation that every graphics backend lowers for production use.
const COMMON_SHADER_SOURCE: &str = r#"
	source_over: fn (source: vec4f, destination: vec4f) -> vec4f {
		let inverse_alpha: f32 = 1.0 - source.w;
		let source_rgb: vec3f = vec3f(source.x, source.y, source.z);
		let destination_rgb: vec3f = vec3f(destination.x, destination.y, destination.z);
		let color: vec3f = source_rgb + destination_rgb * inverse_alpha;
		let alpha: f32 = source.w + destination.w * inverse_alpha;
		return vec4f(color.x, color.y, color.z, alpha);
	}

	sign_not_zero: fn (value: f32) -> f32 {
		if (value < 0.0) {
			return 0.0 - 1.0;
		}
		return 1.0;
	}

	octahedral_encode: fn (direction: vec3f) -> vec2f {
		let denominator: f32 = abs(direction.x) + abs(direction.y) + abs(direction.z);
		let unit: vec3f = direction / max(denominator, 0.000001);
		let encoded: vec2f = vec2f(unit.x, unit.y);
		if (unit.z < 0.0) {
			encoded = vec2f(
				(1.0 - abs(unit.y)) * sign_not_zero(unit.x),
				(1.0 - abs(unit.x)) * sign_not_zero(unit.y)
			);
		}
		return encoded * 0.5 + vec2f(0.5, 0.5);
	}

	octahedral_decode: fn (uv: vec2f) -> vec3f {
		let encoded: vec2f = uv * 2.0 - vec2f(1.0, 1.0);
		let direction: vec3f = vec3f(encoded.x, encoded.y, 1.0 - abs(encoded.x) - abs(encoded.y));
		let fold: f32 = clamp(0.0 - direction.z, 0.0, 1.0);
		let x_adjustment: f32 = 0.0 - fold;
		if (direction.x < 0.0) {
			x_adjustment = fold;
		}
		let y_adjustment: f32 = 0.0 - fold;
		if (direction.y < 0.0) {
			y_adjustment = fold;
		}
		return normalize(vec3f(direction.x + x_adjustment, direction.y + y_adjustment, direction.z));
	}

	atmosphere_ray_sphere_roots: fn (
		origin: vec3f,
		direction: vec3f,
		center: vec3f,
		radius: f32
	) -> vec2f {
		let offset: vec3f = origin - center;
		let b: f32 = dot(offset, direction);
		let c: f32 = dot(offset, offset) - radius * radius;
		let discriminant: f32 = b * b - c;
		if (discriminant < 0.0) {
			return vec2f(1.0, 0.0);
		}
		let root: f32 = sqrt(discriminant);
		return vec2f((0.0 - b) - root, (0.0 - b) + root);
	}

	atmosphere_density_profile: fn (
		sample_position: vec3f,
		planet_center: vec3f,
		ground_radius: f32,
		rayleigh_scale_height: f32,
		mie_scale_height: f32,
		ozone_strength: f32
	) -> vec3f {
		let altitude: f32 = length(sample_position - planet_center) - ground_radius;
		if (altitude < 0.0) {
			return vec3f(0.0, 0.0, 0.0);
		}
		let rayleigh: f32 = exp((0.0 - altitude) / rayleigh_scale_height);
		let mie: f32 = exp((0.0 - altitude) / mie_scale_height);
		let ozone: f32 = max(0.0, 1.0 - abs(altitude - 25000.0) / 15000.0) * ozone_strength;
		return vec3f(rayleigh, mie, ozone);
	}

	atmosphere_extinction_from_density: fn (density: vec3f, haze_scattering: f32) -> vec3f {
		let beta_rayleigh: vec3f = vec3f(0.000005802, 0.000013558, 0.000033100);
		let beta_mie: vec3f = vec3f(haze_scattering, haze_scattering, haze_scattering);
		let beta_ozone: vec3f = vec3f(0.000000650, 0.000001881, 0.000000085);
		return density.x * beta_rayleigh + density.y * beta_mie + density.z * beta_ozone;
	}

	atmosphere_transmittance_uv: fn (
		position: vec3f,
		direction: vec3f,
		planet_center: vec3f,
		ground_radius: f32,
		atmosphere_radius: f32
	) -> vec2f {
		let radial: vec3f = normalize(position - planet_center);
		let altitude: f32 = length(position - planet_center) - ground_radius;
		let atmosphere_height: f32 = atmosphere_radius - ground_radius;
		let height: f32 = clamp(altitude / atmosphere_height, 0.0, 1.0);
		let zenith_cosine: f32 = clamp(dot(radial, direction), 0.0 - 1.0, 1.0);
		return vec2f(zenith_cosine * 0.5 + 0.5, height);
	}

	// Jorge Jimenez's interleaved gradient noise, with the temporal offset described by Alan Wolfe:
	// https://blog.demofox.org/2022/01/01/interleaved-gradient-noise-a-different-kind-of-low-discrepancy-sequence/
	interleaved_gradient_noise: fn (pixel_x: u32, pixel_y: u32, frame: u32) -> f32 {
		let wrapped_frame: u32 = frame % 64;
		let frame_offset: f32 = 5.588238 * f32(wrapped_frame);
		let x: f32 = f32(pixel_x) + frame_offset;
		let y: f32 = f32(pixel_y) + frame_offset;
		return fract(52.9829189 * fract(0.06711056 * x + 0.00583715 * y));
	}

	make_raster_ndc_from_pixel_coordinates: fn (
		pixel_coordinates: vec2u,
		image_extent: vec2u
	) -> vec2f {
		let normalized: vec2f = vec2f(
			(f32(pixel_coordinates.x) + 0.5) / f32(image_extent.x),
			(f32(pixel_coordinates.y) + 0.5) / f32(image_extent.y)
		);
		return vec2f(normalized.x * 2.0 - 1.0, 1.0 - normalized.y * 2.0);
	}
"#;

/// Builds the `Common` module that production shaders, VM tests, and graphics backends link against, followed by the
/// `use Common::*;` that lets the declarations after it call the helpers by name.
///
/// This parses the source on every call. Shader generators call it once when they are created and keep the result.
pub fn common_shader_module() -> [besl::parser::Node<'static>; 2] {
	let mut root = besl::parse(COMMON_SHADER_SOURCE)
		.expect("Failed to parse the common BESL shader module. The most likely cause is invalid portable BESL syntax.");

	let children = match root.node_mut() {
		besl::parser::Nodes::Scope { children, .. } => std::mem::take(children),
		_ => unreachable!("Invalid common BESL shader root. The most likely cause is a parser contract regression."),
	};

	[
		besl::parser::Node::scope("Common", children),
		besl::parser::Node::r#use("Common", true),
	]
}

/// The `CommonShaderGenerator` struct prepares standalone shaders, such as the engine's render passes, for baking.
///
/// Install it on the standalone BESL handler. It gives every shader the common helper module and removes the code of
/// the visibility features the project leaves out; see [`VisibilityFeatures`]. It parses the common module once when it
/// is created, so repeated shader builds only clone the owned syntax tree.
#[derive(Clone)]
pub struct CommonShaderGenerator {
	module: [besl::parser::Node<'static>; 2],
	features: VisibilityFeatures,
}

impl CommonShaderGenerator {
	/// Creates the generator for a project whose `render` parameters, such as `app.parameters().scope("render")`,
	/// select the features it builds in; see [`VisibilityFeatures::from_parameters`].
	///
	/// Next, install it with [`AssetManager::set_program_transform`](resource_management::asset::manager::AssetManager::set_program_transform).
	pub fn new(parameters: impl Parameters) -> Self {
		Self::with_features(VisibilityFeatures::from_parameters(parameters))
	}

	/// Creates the generator for a project that builds in `features`.
	pub fn with_features(features: VisibilityFeatures) -> Self {
		Self {
			module: common_shader_module(),
			features,
		}
	}
}

impl ProgramTransform for CommonShaderGenerator {
	fn identity(&self) -> u64 {
		self.features.generator_identity()
	}

	fn transform<'a>(&self, mut root: besl::parser::Node<'a>) -> besl::parser::Node<'a> {
		// A feature's code sits in `main`; helpers it alone calls drop out because backends emit only what `main` reaches.
		if let Some(besl::parser::Nodes::Function { statements, .. }) = root.get_mut("main").map(|main| main.node_mut()) {
			self.features.remove_left_out_code(statements);
		}
		root.add(self.module.to_vec());

		root
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, Value};

	use super::common_shader_module;
	use crate::rendering::shader_vm_test::{buffer, compile, run_at, slot};

	/// Every standalone visibility shader still links when any one feature is left out, so taking out a feature's
	/// statements never leaves a statement reading something they declared.
	#[test]
	fn every_visibility_shader_links_with_each_feature_left_out() {
		let builtins = besl::Builtins::new();
		use resource_management::asset::handler::implementations::besl::ProgramTransform;

		use crate::rendering::pipelines::visibility::{MaterialEvaluationMode, VisibilityFeatures};

		let all = VisibilityFeatures {
			gtao: true,
			ssgi: true,
			surfel_gi: true,
			probe_gi: true,
			baked_gi: true,
			contact_shadows: true,
			subsurface: true,
			shadowed_suns: 4,
			debug_views: true,
			material_evaluation: MaterialEvaluationMode::Pipelines,
		};
		let left_out: [fn(&mut VisibilityFeatures); 8] = [
			|features| features.gtao = false,
			|features| features.ssgi = false,
			|features| features.surfel_gi = false,
			|features| features.probe_gi = false,
			|features| features.contact_shadows = false,
			|features| features.subsurface = false,
			|features| features.shadowed_suns = 1,
			|features| features.debug_views = false,
		];

		let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/rendering/visibility");
		let mut shaders = std::fs::read_dir(directory)
			.expect("the visibility assets should be readable")
			.map(|entry| entry.expect("a visibility asset entry should be readable").file_name())
			.filter_map(|name| name.to_str().filter(|name| name.ends_with(".besl")).map(str::to_string))
			.collect::<Vec<_>>();
		shaders.sort();

		for leave_out in left_out {
			let mut features = all;
			leave_out(&mut features);
			let generator = super::CommonShaderGenerator::with_features(features);

			for shader in &shaders {
				let source = crate::rendering::shader_vm_test::asset_source(&format!("visibility/{shader}"));
				let root = crate::rendering::shader_vm_test::with_engine_modules(&source);
				if let Err(error) = besl::lex(&builtins, generator.transform(root)) {
					panic!("'{shader}' doesn't link with {features:?}: {error:?}");
				}
			}
		}
	}

	/// Verifies premultiplied source-over composition, the interleaved gradient noise and its 64-frame period, the
	/// octahedral mapping, and the raster mapping of pixel centers through the VM.
	#[test]
	fn common_helpers_execute_with_expected_vm_results() {
		let source = r#"
			main: fn () -> void {
				results.composited = source_over(vec4f(0.2, 0.05, 0.025, 0.25), vec4f(0.2, 0.4, 0.6, 0.5));
				results.noise = interleaved_gradient_noise(10, 20, 1);
				results.periodic_noise = interleaved_gradient_noise(10, 20, 65);
				results.oct_encoded = octahedral_encode(vec3f(0.0, 0.0, 1.0));
				results.oct_decoded = octahedral_decode(vec2f(0.5, 0.5));
				results.oct_round_trip = octahedral_decode(octahedral_encode(vec3f(0.0 - 0.6, 0.0, 0.0 - 0.8)));
				results.raster_ndc = make_raster_ndc_from_pixel_coordinates(vec2u(0, 0), vec2u(4, 2));
				results.raster_ndc_last = make_raster_ndc_from_pixel_coordinates(vec2u(3, 1), vec2u(4, 2));
			}
		"#;
		let members = [
			("composited", "vec4f"),
			("noise", "f32"),
			("periodic_noise", "f32"),
			("oct_encoded", "vec2f"),
			("oct_decoded", "vec3f"),
			("oct_round_trip", "vec3f"),
			("raster_ndc", "vec2f"),
			("raster_ndc_last", "vec2f"),
		]
		.map(|(name, kind)| besl::ParserNode::member(name, kind))
		.to_vec();

		let builtins = besl::Builtins::new();
		let mut root = crate::rendering::shader_vm_test::with_engine_modules(source);
		root.add(common_shader_module().to_vec());
		root.add(vec![besl::ParserNode::binding(
			"results",
			besl::ParserNode::buffer(members),
			0,
			false,
			true,
		)]);
		let program = compile(&besl::lex(&builtins, root).expect(
			"Failed to lex a common shader VM test. The most likely cause is an unresolved portable helper or test binding.",
		));
		let mut results = buffer(&program, slot(&program, "results"));
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot(&program, "results"), &mut results);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);

		let read = |name: &str| -> Vec<f32> {
			match results.read(name).expect("Missing common shader result.") {
				Value::F32(value) => vec![value],
				Value::Vec2F(value) => value.to_vec(),
				Value::Vec3F(value) => value.to_vec(),
				Value::Vec4F(value) => value.to_vec(),
				value => panic!("Unexpected common shader result type for `{name}`: {value:?}"),
			}
		};
		let frame_offset = 5.588238;
		let (x, y) = (10.0 + frame_offset, 20.0 + frame_offset);
		let expected_noise = (52.982_918_f32 * (0.06711056_f32 * x + 0.00583715 * y).fract()).fract();
		for (name, expected) in [
			("composited", vec![0.35, 0.35, 0.475, 0.625]),
			("noise", vec![expected_noise]),
			("periodic_noise", vec![expected_noise]),
			("oct_encoded", vec![0.5, 0.5]),
			("oct_decoded", vec![0.0, 0.0, 1.0]),
			("oct_round_trip", vec![-0.6, 0.0, -0.8]),
			("raster_ndc", vec![-0.75, 0.5]),
			("raster_ndc_last", vec![0.75, -0.5]),
		] {
			let actual = read(name);
			assert!(
				actual
					.iter()
					.zip(&expected)
					.all(|(actual, expected)| (actual - expected).abs() <= 1e-5),
				"Common shader result `{name}` changed: expected {expected:?}, found {actual:?}. The most likely cause is a common helper or VM arithmetic regression."
			);
		}
	}
}
