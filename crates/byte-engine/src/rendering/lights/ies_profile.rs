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
	/// # Panics
	///
	/// Panics when `resource_id` is empty or `dimmer` is outside `0.0..=1.0`. The most likely cause is
	/// that the light was created without a baked `.ies` resource path or with an invalid dimmer.
	pub fn new(resource_id: impl Into<String>, dimmer: f32) -> Self {
		let resource_id = resource_id.into();

		assert!(
			!resource_id.trim().is_empty(),
			"Invalid IES profile resource ID. The most likely cause is that the light was created without a baked .ies resource path."
		);
		assert!(
			dimmer.is_finite() && (0.0..=1.0).contains(&dimmer),
			"Invalid IES dimmer. The most likely cause is that the dimmer is not a finite fraction from 0 through 1."
		);
		Self { resource_id, dimmer }
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
