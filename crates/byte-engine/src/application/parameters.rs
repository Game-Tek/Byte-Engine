use crate::application::Parameter;

/// The `Parameters` trait provides methods for any entity that wnats to expose configurations.
pub trait Parameters {
	/// Returns any paramater that matches the provided full name.
	fn get_parameter(&self, name: &str) -> Option<&Parameter>;
}
