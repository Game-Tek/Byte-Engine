//! Light entities consumed by scene rendering pipelines.
//!
//! Create [`ConeLight`], [`DirectionalLight`], or [`PointLight`] values through
//! [`crate::core::factory::Creator::create`] on the world scope ([`crate::core::message_bus::MessageScope`]). The associated
//! [`crate::gameplay::Transform`] controls each light's position and orientation. For measured IES
//! profiles, the transform also controls the emission axis and C0 plane. Use [`ConeLight::new_ies`]
//! or [`PointLight::new_ies`] to attach the profile. The renderer erases concrete
//! lights only at its internal scene-storage boundary.
//!
//! Transform positions and photometric reference distances use meters. The renderer resolves authored
//! units on the CPU and sends scene-referred RGB lux or candela to the GPU.
//!
//! Follow [Light a scene](/docs/use/lighting)
//! to choose units and submit a light to the active world.

use maths_rs::Vec3f;

pub mod cone;
pub mod directional;
mod ies_profile;
mod photometry;
pub mod point;

pub use cone::ConeLight;
pub use cone::ConeLight as Cone;
pub use directional::DirectionalLight;
pub use directional::DirectionalLight as Directional;
pub use ies_profile::IesProfile;
pub use photometry::{LightColor, PhotometricError, PhotometricIntensity};
pub use point::PointLight;
pub use point::PointLight as Point;

use crate::core::factory::CreateMessage;

/// The `PointLightUpdate`, `ConeLightUpdate`, and `DirectionalLightUpdate` types replace a light's settings: a light
/// created again under the handle of the light it replaces. The inspector posts them under the same names.
pub type PointLightUpdate = CreateMessage<PointLight>;
pub type ConeLightUpdate = CreateMessage<ConeLight>;
pub type DirectionalLightUpdate = CreateMessage<DirectionalLight>;

/// The `Light` trait identifies the shader and storage class of a scene light.
pub trait Light {
	fn class(&self) -> LightClasses;
}

/// The [`LightClasses`] enum identifies the shader and storage layout required by
/// a light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightClasses {
	Cone,
	Directional,
	Point,
}

/// The `LocalEmission` struct holds the emitter state every local light shares, so cone and point lights define it
/// once and the renderer reads it the same way for both.
///
/// Reach it through the `emission` field of [`ConeLight`] or [`PointLight`]. `color` is RGB luminous intensity in candela,
/// or unit-luminance chromaticity when an [`IesProfile`] supplies the calibrated intensity.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEmission {
	pub color: Vec3f,
	ies_profile: Option<IesProfile>,
	shadow_near_override: Option<f32>,
	shadow_far_override: Option<f32>,
}

impl LocalEmission {
	/// Resolves a local light's authored `color` and `intensity`: through its IES profile when it names one, and
	/// otherwise to the candela `candela` resolves the intensity to for the light's shape.
	fn new(
		color: LightColor,
		intensity: &PhotometricIntensity,
		candela: impl FnOnce(&PhotometricIntensity) -> Result<f32, PhotometricError>,
	) -> Result<Self, PhotometricError> {
		match intensity {
			PhotometricIntensity::Ies { profile, dimmer } => Self::ies(color, *dimmer, profile.clone()),
			_ => Ok(Self::uniform(color.resolve()?, candela(intensity)?)),
		}
	}

	/// Creates an analytic emitter whose `chromaticity` carries `candela`.
	fn uniform(chromaticity: Vec3f, candela: f32) -> Self {
		Self::with_profile(
			Vec3f::new(chromaticity.x * candela, chromaticity.y * candela, chromaticity.z * candela),
			None,
		)
	}

	/// Resolves an emitter whose calibrated intensity and angular distribution come from a baked IES profile.
	fn ies(color: LightColor, dimmer: f32, ies_profile_resource_id: impl Into<String>) -> Result<Self, PhotometricError> {
		Ok(Self::with_profile(
			color.resolve()?,
			Some(IesProfile::new(ies_profile_resource_id, dimmer)?),
		))
	}

	fn with_profile(color: Vec3f, ies_profile: Option<IesProfile>) -> Self {
		Self {
			color,
			ies_profile,
			shadow_near_override: None,
			shadow_far_override: None,
		}
	}

	/// Returns the optional IES profile that supplies this emitter's intensity distribution.
	pub fn ies_profile(&self) -> Option<&IesProfile> {
		self.ies_profile.as_ref()
	}

	/// Returns the optional near clipping-distance override for the renderer.
	pub(crate) fn shadow_near_override(&self) -> Option<f32> {
		self.shadow_near_override
	}

	/// Returns the optional far clipping-distance override for the renderer.
	pub(crate) fn shadow_far_override(&self) -> Option<f32> {
		self.shadow_far_override
	}
}

#[derive(Clone)]
/// The `Lights` enum gives renderer storage one internal representation for supported light types.
pub(crate) enum Lights {
	Cone(ConeLight),
	Direction(DirectionalLight),
	Point(PointLight),
}

impl Lights {
	/// Returns the emitter state of a local light, or `None` for a directional light.
	pub(crate) fn local(&self) -> Option<&LocalEmission> {
		match self {
			Lights::Cone(light) => Some(&light.emission),
			Lights::Point(light) => Some(&light.emission),
			Lights::Direction(_) => None,
		}
	}
}

/// Checks a local light's authored shadow range: each end above zero and finite, and the near one below the far one
/// when both are set.
fn checked_shadow_range(near: Option<f32>, far: Option<f32>) -> Result<(), PhotometricError> {
	let positive = |value: Option<f32>| value.is_none_or(|value| value.is_finite() && value > 0.0);
	if !(positive(near) && positive(far)) {
		return Err(PhotometricError::new(
			"that a shadow distance isn't above zero and finite. Give `shadow_near` and `shadow_far` in meters above zero, or leave them out",
		));
	}
	if let (Some(near), Some(far)) = (near, far)
		&& near >= far
	{
		return Err(PhotometricError::new(
			"that `shadow_near` isn't below `shadow_far`. Give a near distance below the far one",
		));
	}
	Ok(())
}

/// Implements serde for a light through its checked payload proxy `$proxy`, the shape its level kind and its inspector
/// update message take, as facet's `proxy` attribute does for reflection.
macro_rules! serde_through_proxy {
	($light:ty, $proxy:ty) => {
		impl serde::Serialize for $light {
			fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
				<$proxy>::from(self).serialize(serializer)
			}
		}

		impl<'de> serde::Deserialize<'de> for $light {
			fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
				<$light>::try_from(<$proxy as serde::Deserialize>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
			}
		}
	};
}
pub(crate) use serde_through_proxy;

/// Implements the shadow-range builders and IES accessor of a light that embeds a [`LocalEmission`] in `emission`.
///
/// Both local lights expose the same builder surface, so it is written once here.
macro_rules! local_emission_builders {
	($light:ty, $view:literal) => {
		impl $light {
			/// Returns the optional IES profile that supplies this light's intensity distribution.
			pub fn ies_profile(&self) -> Option<&IesProfile> {
				self.emission.ies_profile()
			}

			#[doc = concat!("Overrides the renderer-derived near clipping distance for this light's ", $view, ".")]
			pub fn with_shadow_near(mut self, shadow_near: f32) -> Self {
				self.emission.shadow_near_override = Some(shadow_near);
				self
			}

			#[doc = concat!("Overrides the renderer-derived far clipping distance for this light's ", $view, ".")]
			pub fn with_shadow_far(mut self, shadow_far: f32) -> Self {
				self.emission.shadow_far_override = Some(shadow_far);
				self
			}

			#[doc = concat!("Overrides both renderer-derived clipping distances for this light's ", $view, ".")]
			pub fn with_shadow_range(self, shadow_near: f32, shadow_far: f32) -> Self {
				self.with_shadow_near(shadow_near).with_shadow_far(shadow_far)
			}
		}
	};
}

local_emission_builders!(ConeLight, "shadow view");
local_emission_builders!(PointLight, "cube shadow map");

impl From<ConeLight> for Lights {
	fn from(val: ConeLight) -> Self {
		Lights::Cone(val)
	}
}

impl From<PointLight> for Lights {
	fn from(val: PointLight) -> Self {
		Lights::Point(val)
	}
}

impl From<DirectionalLight> for Lights {
	fn from(val: DirectionalLight) -> Self {
		Lights::Direction(val)
	}
}
