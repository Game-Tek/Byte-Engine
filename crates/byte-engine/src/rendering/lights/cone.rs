/// The `ConeLight` struct provides photometric settings for local lighting constrained to a cone.
///
/// Use it for spotlights, flashlights, and other emitters that need a soft transition between a
/// fully lit inner cone and an unlit outer cone. Cone angles are half angles measured in radians
/// from the associated [`crate::gameplay::Transform`]'s forward direction. It keeps the color and intensity it was
/// authored with, which facet and serde read and write as a level's
/// [`cone_light` kind](/docs/reference/levels#lights) and the inspector's `ConeLightUpdate` payload. To change it
/// later, create it again under its own handle.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
#[facet(opaque, proxy = SerializableConeLight)]
pub struct ConeLight {
	pub(super) color: LightColor,
	pub(super) intensity: PhotometricIntensity,
	/// What the color and intensity resolve to, with the shadow-range overrides every local light shares.
	pub(crate) emission: LocalEmission,
	pub(crate) inner_angle: Radians,
	pub(crate) outer_angle: Radians,
}

impl ConeLight {
	/// Creates a cone light of `color` and `intensity` whose intensity fades smoothly between the inner and outer half
	/// angles.
	///
	/// An intensity of [`PhotometricIntensity::Ies`] takes the light's intensity and angular distribution from a baked
	/// IES profile, tinted by `color`; the cone cutoff still applies after the IES lookup. The renderer derives the
	/// shadow range from the resolved luminous intensity. Use [`Self::with_shadow_near`], [`Self::with_shadow_far`],
	/// or [`Self::with_shadow_range`] to override that range. Next, pass the returned light to
	/// [`crate::core::factory::Creator::create`] on the world scope ([`crate::core::message_bus::MessageScope`]) to make
	/// it available to the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value, the IES profile is
	/// invalid, or the half angles aren't finite or don't satisfy `0 ≤ inner_angle < outer_angle ≤ π` radians.
	pub fn new(
		color: LightColor,
		intensity: PhotometricIntensity,
		inner_angle: Radians,
		outer_angle: Radians,
	) -> Result<Self, PhotometricError> {
		Self::validate_angles(inner_angle, outer_angle)?;
		let emission = LocalEmission::new(color, &intensity, |intensity| {
			intensity.cone_candela(inner_angle, outer_angle)
		})?;
		Ok(Self {
			color,
			intensity,
			emission,
			inner_angle,
			outer_angle,
		})
	}

	/// Returns the half angles of the fully lit inner cone and of the outer cone, in radians from the forward
	/// direction.
	pub fn angles(&self) -> (Radians, Radians) {
		(self.inner_angle, self.outer_angle)
	}

	/// Checks the angular range shared by uniform and IES-backed cone lights: finite half angles, the inner one from
	/// zero up to the outer one, and the outer one at most π.
	fn validate_angles(inner_angle: Radians, outer_angle: Radians) -> Result<(), PhotometricError> {
		if !(inner_angle.is_finite()
			&& outer_angle.is_finite()
			&& inner_angle >= Radians::new(0.0)
			&& inner_angle < outer_angle)
		{
			return Err(PhotometricError::new(
				"that the cone's half angles aren't finite or its inner angle isn't below its outer angle. Give both in radians, the inner one from zero up to the outer one",
			));
		}
		if outer_angle > Radians::new(std::f32::consts::PI) {
			return Err(PhotometricError::new(
				"that the cone's outer half angle exceeds π. Give it in radians, at most π",
			));
		}
		Ok(())
	}

	/// Returns whether this light's cone fits in one perspective shadow view.
	pub fn supports_shadow_mapping(&self) -> bool {
		self.outer_angle < Radians::new(std::f32::consts::FRAC_PI_2)
	}
}

/// The `SerializableConeLight` struct is the checked reflection boundary of [`ConeLight`], in the `cone_light`
/// payload's shape, with half angles in radians.
#[derive(facet::Facet, serde::Serialize, serde::Deserialize)]
#[facet(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub(crate) struct SerializableConeLight {
	color: LightColor,
	intensity: PhotometricIntensity,
	inner_angle: f32,
	outer_angle: f32,
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

impl TryFrom<SerializableConeLight> for ConeLight {
	type Error = String;

	/// Checks the payload against the light's rules before building it.
	fn try_from(value: SerializableConeLight) -> Result<Self, Self::Error> {
		ConeLight::new(
			value.color,
			value.intensity,
			Radians::new(value.inner_angle),
			Radians::new(value.outer_angle),
		)
		.map_err(|error| error.to_string())?
		.with_payload_emission(value.shadow_near, value.shadow_far, value.blinds)
	}
}

impl From<&ConeLight> for SerializableConeLight {
	fn from(light: &ConeLight) -> Self {
		Self {
			color: light.color,
			intensity: light.intensity.clone(),
			inner_angle: light.inner_angle.value(),
			outer_angle: light.outer_angle.value(),
			shadow_near: light.emission.shadow_near_override(),
			shadow_far: light.emission.shadow_far_override(),
			blinds: light.blinds(),
		}
	}
}

serde_through_proxy!(ConeLight, SerializableConeLight);

use math::Radians;

use super::{LightColor, LocalEmission, PhotometricError, PhotometricIntensity, serde_through_proxy};
