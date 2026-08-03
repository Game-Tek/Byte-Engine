/// The `LightColor` enum provides a chromaticity for physically based light authoring.
///
/// Pair a color with [`PhotometricIntensity`], then pass both values to a light constructor such as
/// [`super::DirectionalLight::new`]. RGB magnitude does not control brightness because colors are normalized to unit luminance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightColor {
	/// Uses a blackbody-style color temperature from 1,000 K through 40,000 K.
	Kelvin(f32),
	/// Uses a nonnegative linear-sRGB chromaticity with nonzero luminance.
	LinearSrgb(maths_rs::Vec3f),
}

impl LightColor {
	/// Creates a `Kelvin` color from a temperature in Kelvin. See [`LightColor::Kelvin`].
	pub fn kelvin(kelvin: f32) -> Self {
		Self::Kelvin(kelvin)
	}

	/// Creates a `LinearSrgb` color from RGB components. See [`LightColor::LinearSrgb`].
	pub fn linear_srgb(r: f32, g: f32, b: f32) -> Self {
		Self::LinearSrgb(maths_rs::Vec3f::new(r, g, b))
	}
}

impl From<f32> for LightColor {
	fn from(value: f32) -> Self {
		Self::kelvin(value)
	}
}

impl From<(f32, f32, f32)> for LightColor {
	fn from(value: (f32, f32, f32)) -> Self {
		Self::linear_srgb(value.0, value.1, value.2)
	}
}

/// The `PhotometricIntensity` enum provides physical light quantities with enough context for analytic lights.
///
/// You can use every variant with every light shape. See the
/// [physically based lighting guide](https://byte-engine.0x44491229.dev/docs/use/lighting)
/// to choose the quantity and its reference values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PhotometricIntensity {
	/// Sets illuminance at a measurement distance.
	Illuminance {
		/// The illuminance in lumens per square meter.
		lux: f32,
		/// The distance in meters used to convert illuminance for a local light.
		measurement_distance_m: f32,
	},
	/// Sets luminous intensity with a reference distance for directional conversion.
	LuminousIntensity {
		/// The luminous intensity in lumens per steradian.
		candela: f32,
		/// The distance in meters used to convert intensity for a directional light.
		reference_distance_m: f32,
	},
	/// Sets total luminous flux with a directional beam area.
	LuminousFlux {
		/// The total emitted luminous flux.
		lumens: f32,
		/// The beam area in square meters used to convert flux for a directional light.
		directional_beam_area_m2: f32,
	},
	/// Sets emitter luminance with the geometry needed for an analytic approximation.
	Luminance {
		/// The luminance in candela per square meter.
		nits: f32,
		/// The visible projected emitter area in square meters.
		projected_area_m2: f32,
		/// The distance in meters used to convert luminance for a directional light.
		reference_distance_m: f32,
	},
}

impl PhotometricIntensity {
	/// Creates an illuminance intensity from a lux value, with a default measurement distance of 1 meter.
	pub fn illuminance(lux: f32) -> Self {
		Self::Illuminance {
			lux,
			measurement_distance_m: 1.0,
		}
	}

	/// Creates a luminous intensity from a candela value, with a default reference distance of 1 meter.
	pub fn luminous_intensity(candela: f32) -> Self {
		Self::LuminousIntensity {
			candela,
			reference_distance_m: 1.0,
		}
	}

	/// Creates a luminous flux from a lumens value, with a default directional beam area of 1 square meter.
	pub fn luminous_flux(lumens: f32) -> Self {
		Self::LuminousFlux {
			lumens,
			directional_beam_area_m2: 1.0,
		}
	}

	/// Creates a luminance from a nits value, with a default projected area and reference distance of 1 square meter and 1 meter.
	pub fn luminance(nits: f32) -> Self {
		Self::Luminance {
			nits,
			projected_area_m2: 1.0,
			reference_distance_m: 1.0,
		}
	}
}

/// The `PhotometricError` struct explains why a physical light description cannot be resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhotometricError(&'static str);

impl fmt::Display for PhotometricError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "Invalid photometric light. The most likely cause is {}.", self.0)
	}
}

impl std::error::Error for PhotometricError {}

impl LightColor {
	/// Resolves the authored color to a nonnegative linear-sRGB chromaticity with unit luminance.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color has a negative or non-finite component, has zero luminance,
	/// or contains a temperature outside the supported range.
	pub fn resolve(self) -> Result<maths_rs::Vec3f, PhotometricError> {
		let rgb = match self {
			Self::LinearSrgb(rgb) => rgb,
			Self::Kelvin(temperature) => linear_srgb_from_temperature(temperature)?,
		};
		normalize_luminance(rgb)
	}
}

impl PhotometricIntensity {
	/// Resolves this quantity to directional illuminance in lux.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when a required magnitude, distance, or area is not positive and finite.
	pub fn directional_lux(self) -> Result<f32, PhotometricError> {
		match self {
			Self::Illuminance {
				lux,
				measurement_distance_m,
			} => {
				positive(measurement_distance_m, "the measurement distance is not positive and finite")?;
				positive(lux, "the illuminance is not positive and finite")
			}
			Self::LuminousIntensity {
				candela,
				reference_distance_m,
			} => Ok(positive(candela, "the luminous intensity is not positive and finite")?
				/ positive(reference_distance_m, "the reference distance is not positive and finite")?.powi(2)),
			Self::LuminousFlux {
				lumens,
				directional_beam_area_m2,
			} => Ok(positive(lumens, "the luminous flux is not positive and finite")?
				/ positive(
					directional_beam_area_m2,
					"the directional beam area is not positive and finite",
				)?),
			Self::Luminance {
				nits,
				projected_area_m2,
				reference_distance_m,
			} => Ok(positive(nits, "the luminance is not positive and finite")?
				* positive(projected_area_m2, "the projected area is not positive and finite")?
				/ positive(reference_distance_m, "the reference distance is not positive and finite")?.powi(2)),
		}
	}

	/// Resolves this quantity to point-light luminous intensity in candela.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when a required magnitude, distance, or area is not positive and finite.
	pub fn point_candela(self) -> Result<f32, PhotometricError> {
		self.local_candela(4.0 * std::f32::consts::PI)
	}

	/// Resolves this quantity to cone-light luminous intensity in candela.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when a required magnitude, distance, or area is not positive and finite.
	pub fn cone_candela(self, inner_angle: f32, outer_angle: f32) -> Result<f32, PhotometricError> {
		let effective_solid_angle = std::f32::consts::PI * (2.0 - inner_angle.cos() - outer_angle.cos());
		self.local_candela(effective_solid_angle)
	}

	fn local_candela(self, flux_solid_angle: f32) -> Result<f32, PhotometricError> {
		match self {
			Self::Illuminance {
				lux,
				measurement_distance_m,
			} => Ok(positive(lux, "the illuminance is not positive and finite")?
				* positive(measurement_distance_m, "the measurement distance is not positive and finite")?.powi(2)),
			Self::LuminousIntensity {
				candela,
				reference_distance_m,
			} => {
				positive(reference_distance_m, "the reference distance is not positive and finite")?;
				positive(candela, "the luminous intensity is not positive and finite")
			}
			Self::LuminousFlux {
				lumens,
				directional_beam_area_m2,
			} => {
				positive(
					directional_beam_area_m2,
					"the directional beam area is not positive and finite",
				)?;
				Ok(positive(lumens, "the luminous flux is not positive and finite")? / flux_solid_angle)
			}
			Self::Luminance {
				nits,
				projected_area_m2,
				reference_distance_m,
			} => {
				positive(reference_distance_m, "the reference distance is not positive and finite")?;
				Ok(positive(nits, "the luminance is not positive and finite")?
					* positive(projected_area_m2, "the projected area is not positive and finite")?)
			}
		}
	}
}

/// Converts a blackbody color temperature through CIE xyY and XYZ into linear sRGB.
fn linear_srgb_from_temperature(temperature: f32) -> Result<maths_rs::Vec3f, PhotometricError> {
	if !temperature.is_finite() || !(1_000.0..=40_000.0).contains(&temperature) {
		return Err(PhotometricError("the color temperature is outside 1000 K to 40000 K"));
	}
	let t = f64::from(temperature);
	let x = if t <= 4_000.0 {
		-0.266_123_9e9 / t.powi(3) - 0.234_358_0e6 / t.powi(2) + 0.877_695_6e3 / t + 0.179_910
	} else {
		-3.025_846_9e9 / t.powi(3) + 2.107_037_9e6 / t.powi(2) + 0.222_634_7e3 / t + 0.240_390
	};
	let y = if t <= 2_222.0 {
		-1.106_381_4 * x.powi(3) - 1.348_110_20 * x.powi(2) + 2.185_558_32 * x - 0.202_196_83
	} else if t <= 4_000.0 {
		-0.954_947_6 * x.powi(3) - 1.374_185_93 * x.powi(2) + 2.091_370_15 * x - 0.167_488_67
	} else {
		3.081_758_0 * x.powi(3) - 5.873_386_70 * x.powi(2) + 3.751_129_97 * x - 0.370_014_83
	};
	let xyz_x = x / y;
	let xyz_z = (1.0 - x - y) / y;
	Ok(maths_rs::Vec3f::new(
		(3.240_454_2 * xyz_x - 1.537_138_5 - 0.498_531_4 * xyz_z).max(0.0) as f32,
		(-0.969_266 * xyz_x + 1.876_010_8 + 0.041_556 * xyz_z).max(0.0) as f32,
		(0.055_643_4 * xyz_x - 0.204_025_9 + 1.057_225_2 * xyz_z).max(0.0) as f32,
	))
}

fn normalize_luminance(rgb: maths_rs::Vec3f) -> Result<maths_rs::Vec3f, PhotometricError> {
	if !rgb.x.is_finite() || !rgb.y.is_finite() || !rgb.z.is_finite() || rgb.x < 0.0 || rgb.y < 0.0 || rgb.z < 0.0 {
		return Err(PhotometricError("the light color has a negative or non-finite component"));
	}
	let luminance = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
	let luminance = positive(luminance, "the light color has zero luminance")?;
	Ok(maths_rs::Vec3f::new(rgb.x / luminance, rgb.y / luminance, rgb.z / luminance))
}

fn positive(value: f32, cause: &'static str) -> Result<f32, PhotometricError> {
	if value.is_finite() && value > 0.0 {
		Ok(value)
	} else {
		Err(PhotometricError(cause))
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::Vec3f;

	use super::{LightColor, PhotometricIntensity};

	fn assert_near(actual: f32, expected: f32) {
		assert!(
			(actual - expected).abs() <= expected.abs().max(1.0) * 0.000_01,
			"{actual} != {expected}"
		);
	}

	#[test]
	fn authored_colors_resolve_to_unit_photopic_luminance() {
		for color in [
			LightColor::Kelvin(1_500.0),
			LightColor::Kelvin(6_500.0),
			LightColor::Kelvin(15_000.0),
			LightColor::LinearSrgb(Vec3f::new(0.2, 0.5, 1.0)),
		] {
			let rgb = color.resolve().expect("valid light chromaticity");
			assert_near(0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z, 1.0);
			assert!(rgb.x >= 0.0 && rgb.y >= 0.0 && rgb.z >= 0.0);
		}
	}

	#[test]
	fn point_units_resolve_to_equivalent_candela() {
		let expected = 100.0;
		let quantities = [
			PhotometricIntensity::Illuminance {
				lux: 25.0,
				measurement_distance_m: 2.0,
			},
			PhotometricIntensity::LuminousIntensity {
				candela: expected,
				reference_distance_m: 1.0,
			},
			PhotometricIntensity::LuminousFlux {
				lumens: expected * 4.0 * std::f32::consts::PI,
				directional_beam_area_m2: 1.0,
			},
			PhotometricIntensity::Luminance {
				nits: 50.0,
				projected_area_m2: 2.0,
				reference_distance_m: 1.0,
			},
		];

		for quantity in quantities {
			assert_near(quantity.point_candela().expect("valid point quantity"), expected);
		}
	}

	#[test]
	fn directional_units_resolve_to_equivalent_lux() {
		let expected = 100.0;
		let quantities = [
			PhotometricIntensity::Illuminance {
				lux: expected,
				measurement_distance_m: 1.0,
			},
			PhotometricIntensity::LuminousIntensity {
				candela: 400.0,
				reference_distance_m: 2.0,
			},
			PhotometricIntensity::LuminousFlux {
				lumens: 500.0,
				directional_beam_area_m2: 5.0,
			},
			PhotometricIntensity::Luminance {
				nits: 200.0,
				projected_area_m2: 2.0,
				reference_distance_m: 2.0,
			},
		];

		for quantity in quantities {
			assert_near(quantity.directional_lux().expect("valid directional quantity"), expected);
		}
	}

	#[test]
	fn cone_flux_uses_the_soft_cone_effective_solid_angle() {
		let inner = 20.0_f32.to_radians();
		let outer = 40.0_f32.to_radians();
		let solid_angle = std::f32::consts::PI * (2.0 - inner.cos() - outer.cos());
		let intensity = PhotometricIntensity::LuminousFlux {
			lumens: 250.0 * solid_angle,
			directional_beam_area_m2: 1.0,
		};

		assert_near(intensity.cone_candela(inner, outer).expect("valid cone flux"), 250.0);
	}

	#[test]
	fn invalid_photometric_inputs_return_actionable_errors() {
		let error = LightColor::LinearSrgb(Vec3f::new(0.0, 0.0, 0.0))
			.resolve()
			.expect_err("black has no chromaticity");
		assert!(error.to_string().contains("most likely cause"));

		let error = PhotometricIntensity::Illuminance {
			lux: 100.0,
			measurement_distance_m: 0.0,
		}
		.point_candela()
		.expect_err("zero reference distance");
		assert!(error.to_string().contains("measurement distance"));
	}
}

use std::fmt;
