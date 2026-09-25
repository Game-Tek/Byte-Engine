/// The `Environment` struct identifies the baked environment-map resource used for scene lighting and reflections.
///
/// Create an environment through [`crate::gameplay::world::DefaultWorld::factory`]
/// after installing the visibility pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
	resource_id: String,
	illuminance: Option<f32>,
}

impl Environment {
	/// Creates an environment backed by the named `.environment.bead` resource.
	///
	/// See the [environment-map asset guide](/docs/develop/resource-management/assets#environment-maps)
	/// before selecting the resource through the world factory.
	pub fn new(resource_id: impl Into<String>) -> Self {
		Self {
			resource_id: resource_id.into(),
			illuminance: None,
		}
	}

	/// Scales the environment so it delivers `lux` to an upward-facing surface.
	///
	/// Environment maps store light in arbitrary units, so use this to match one to real-world lights such as a
	/// [`crate::rendering::DirectionalLight`] sun. A clear daytime sky delivers about 10,000 to 25,000 lux and an
	/// overcast one about 1,000 to 10,000. Without it, the map's own values are used unchanged.
	pub fn with_illuminance(mut self, lux: f32) -> Self {
		debug_assert!(
			lux.is_finite() && lux >= 0.0,
			"Environment illuminance is invalid. The most likely cause is a negative or non-finite lux value."
		);
		self.illuminance = Some(lux);
		self
	}

	/// Returns the requested illuminance on an upward-facing surface in lux, if any. See [`Self::with_illuminance`].
	pub fn illuminance(&self) -> Option<f32> {
		self.illuminance
	}

	/// Returns the baked environment-map resource used to load this environment.
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}
}

#[cfg(test)]
mod tests {
	use super::Environment;

	#[test]
	fn environment_retains_its_baked_resource_id() {
		let environment = Environment::new("studio.environment.bead");

		assert_eq!(environment.resource_id(), "studio.environment.bead");
		assert_eq!(environment.illuminance(), None);
		assert_eq!(environment.with_illuminance(20_000.0).illuminance(), Some(20_000.0));
	}
}
