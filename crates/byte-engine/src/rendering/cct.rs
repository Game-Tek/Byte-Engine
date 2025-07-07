//! Taken from https://github.com/m-lima/tempergb

use math::Vector3;

/// Converts a color temperature to RBG [`Color`](struct.Color.html).
///
/// **Note:** The input temperature should be in the `[1000, 40000]` range. Values outside of this
/// range will be truncated.
pub fn rgb_from_temperature(temperature: impl Into<f64>) -> Vector3 {
	let temperature = {
		let temperature: f64 = temperature.into();
		if temperature < 1000.0 {
			1000.0
		} else if temperature > 40000.0 {
			40000.0
		} else {
			temperature
		}
	} / 100.0;

	let r = if temperature <= 66.0 {
		0xff
	} else {
		into_saturated_u8(329.698_727_446 * (temperature - 60.0).powf(-0.133_204_759_2))
	};

	let g = if temperature <= 66.0 {
		into_saturated_u8(99.470_802_586_1 * temperature.ln() - 161.119_568_166_1)
	} else {
		into_saturated_u8(288.122_169_528_3 * (temperature - 60.0).powf(-0.075_514_849_2))
	};

	let b = if temperature >= 66.0 {
		0xff
	} else if temperature <= 19.0 {
		0
	} else {
		into_saturated_u8(138.517_731_223_1 * (temperature - 10.0).ln() - 305.044_792_730_7)
	};

	Vector3::new(r as f32 / 255f32, g as f32 / 255f32, b as f32 / 255f32)
}

#[inline]
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
// Allow(clippy::cast_sign_loss, clippy::cast_possible_truncation): The bounds have been previously checked
fn into_saturated_u8(float: f64) -> u8 {
	if float < 0.0 {
		0
	} else if float > 255.0 {
		255
	} else {
		float as u8
	}
}


#[cfg(test)]
mod tests {
	use math::Vector3;

	use super::{rgb_from_temperature,};

	fn assert_temperature<F: Into<f64>>(temperature: F, r: u8, g: u8, b: u8) {
		assert_eq!(rgb_from_temperature(temperature), Vector3::new(r as f32 / 255f32, g as f32 / 255f32, b as f32 / 255f32));
	}

	#[test]
	fn temperature_0() {
		assert_temperature(0, 255, 67, 0);
	}

	#[test]
	fn temperature_1500() {
		assert_temperature(1500, 255, 108, 0);
	}

	#[test]
	fn temperature_2500() {
		assert_temperature(2500, 255, 159, 70);
	}

	#[test]
	fn temperature_5000() {
		assert_temperature(5000, 255, 228, 205);
	}

	#[test]
	fn temperature_6600() {
		assert_temperature(6600, 255, 255, 255);
	}

	#[test]
	fn temperature_10000() {
		assert_temperature(10000, 201, 218, 255);
	}

	#[test]
	fn temperature_15000() {
		assert_temperature(15000, 181, 205, 255);
	}

	#[test]
	fn temperature_40000() {
		assert_temperature(40000, 151, 185, 255);
	}

	#[test]
	fn temperature_60000() {
		assert_temperature(60000, 151, 185, 255);
	}
}
