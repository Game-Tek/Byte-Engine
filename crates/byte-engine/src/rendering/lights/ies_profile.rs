use super::PhotometricError;

/// The `IesProfile` struct identifies a baked IES intensity map used by a local light.
///
/// Use it through [`super::ConeLight::new_ies`] or [`super::PointLight::new_ies`]. The visibility
/// pipeline loads the named image asynchronously and applies its dimmed calibrated luminous-intensity
/// map after the upload completes. The light entity's associated [`crate::gameplay::Transform`] controls
/// the profile orientation: local `+X` defines the C0 tangent, local `+Y` defines C90, and local `+Z`
/// defines the emission axis.
#[derive(Debug, Clone, PartialEq)]
pub struct IesProfile {
	resource_id: String,
	dimmer: f32,
}

impl IesProfile {
	/// Creates an IES profile reference with a calibrated-output dimmer.
	///
	/// `dimmer` is a linear fraction from `0.0` for off through `1.0` for the profile's measured output.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when `resource_id` is empty or `dimmer` is outside `0.0..=1.0`.
	pub fn new(resource_id: impl Into<String>, dimmer: f32) -> Result<Self, PhotometricError> {
		let resource_id = resource_id.into();

		if resource_id.trim().is_empty() {
			return Err(PhotometricError::new(
				"that the IES profile's resource ID is empty. Name the baked .ies resource",
			));
		}
		if !(dimmer.is_finite() && (0.0..=1.0).contains(&dimmer)) {
			return Err(PhotometricError::new(
				"that the IES dimmer isn't a fraction from 0 through 1. Give 0 for off through 1 for the measured output",
			));
		}
		Ok(Self { resource_id, dimmer })
	}

	/// Returns the baked `Image` resource ID requested by the visibility pipeline.
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}

	/// Returns the linear fraction of the profile's calibrated luminous intensity.
	pub fn dimmer(&self) -> f32 {
		self.dimmer
	}
}
