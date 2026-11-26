use super::{LightColor, LocalEmission, PhotometricError, PhotometricIntensity, serde_through_proxy};

/// The `PointLight` struct provides photometric settings for omnidirectional local emitters.
///
/// Use the associated [`crate::gameplay::Transform`] to place the light and orient an optional IES profile. It keeps
/// the color and intensity it was authored with, which facet and serde read and write as a level's
/// [`point_light` kind](/docs/reference/levels#lights) and the inspector's `PointLightUpdate` payload. To change it
/// later, create it again under its own handle.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
#[facet(opaque, proxy = SerializablePointLight)]
pub struct PointLight {
	pub(super) color: LightColor,
	pub(super) intensity: PhotometricIntensity,
	/// What the color and intensity resolve to, with the shadow-range overrides every local light shares.
	pub(crate) emission: LocalEmission,
}

impl PointLight {
	/// Creates a point light of `color` and `intensity`, whose GPU color is luminous intensity in candela.
	///
	/// An intensity of [`PhotometricIntensity::Ies`] takes the light's intensity and angular distribution from a baked
	/// IES profile, tinted by `color`. The renderer derives cube-shadow coverage from the resolved luminous intensity.
	/// Use [`Self::with_shadow_near`], [`Self::with_shadow_far`], or [`Self::with_shadow_range`] to override that
	/// range. Next, pass the returned light to [`crate::core::factory::Creator::create`] on the world scope
	/// ([`crate::core::message_bus::MessageScope`]) to make it available to the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value, or the IES profile
	/// is invalid: see [`super::IesProfile::new`].
	pub fn new(color: LightColor, intensity: PhotometricIntensity) -> Result<Self, PhotometricError> {
		let emission = LocalEmission::new(color, &intensity, PhotometricIntensity::point_candela)?;
		Ok(Self {
			color,
			intensity,
			emission,
		})
	}
}

/// The `SerializablePointLight` struct is the checked reflection boundary of [`PointLight`], in the `point_light`
/// payload's shape.
#[derive(facet::Facet, serde::Serialize, serde::Deserialize)]
#[facet(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub(crate) struct SerializablePointLight {
	color: LightColor,
	intensity: PhotometricIntensity,
	#[facet(default)]
	#[serde(default)]
	shadow_near: Option<f32>,
	#[facet(default)]
	#[serde(default)]
	shadow_far: Option<f32>,
	#[facet(default)]
	#[serde(default)]
	blinds: f32,
}

impl TryFrom<SerializablePointLight> for PointLight {
	type Error = String;

	/// Checks the payload against the light's rules before building it.
	fn try_from(value: SerializablePointLight) -> Result<Self, Self::Error> {
		PointLight::new(value.color, value.intensity)
			.map_err(|error| error.to_string())?
			.with_payload_emission(value.shadow_near, value.shadow_far, value.blinds)
	}
}

impl From<&PointLight> for SerializablePointLight {
	fn from(light: &PointLight) -> Self {
		Self {
			color: light.color,
			intensity: light.intensity.clone(),
			shadow_near: light.emission.shadow_near_override(),
			shadow_far: light.emission.shadow_far_override(),
			blinds: light.blinds(),
		}
	}
}

serde_through_proxy!(PointLight, SerializablePointLight);
