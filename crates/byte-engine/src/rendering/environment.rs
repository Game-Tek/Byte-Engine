/// The `Environment` struct identifies the baked image used for scene lighting and reflections.
///
/// Create an environment through
/// [`crate::gameplay::world::DefaultWorld::environment_factory_mut`]
/// after installing the visibility pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
	resource_id: String,
}

impl Environment {
	/// Creates an environment backed by the named baked image resource.
	pub fn new(resource_id: impl Into<String>) -> Self {
		Self {
			resource_id: resource_id.into(),
		}
	}

	/// Returns the baked image resource used to load this environment.
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}
}

#[cfg(test)]
mod tests {
	use super::Environment;

	#[test]
	fn environment_retains_its_baked_resource_id() {
		let environment = Environment::new("studio.exr");

		assert_eq!(environment.resource_id(), "studio.exr");
	}
}
