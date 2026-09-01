use std::sync::Arc;

/// The `Name` struct gives an entity a human-readable identity for inspection and tooling.
///
/// Attach a name while spawning an entity with
/// [`Creation::with`](crate::core::factory::Creation::with). The inspector can
/// then return or filter the entity by this exact value. Names are not unique,
/// so one query can return multiple entities.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Name(Arc<str>);

impl Name {
	/// Creates a name from borrowed or owned text.
	pub fn new(name: impl Into<Arc<str>>) -> Self {
		let name = name.into();
		assert!(
			!name.is_empty(),
			"Name cannot be empty. The most likely cause is that an entity name was created from an empty string."
		);
		Self(name)
	}

	/// Returns the text used by inspection and tooling.
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl AsRef<str> for Name {
	fn as_ref(&self) -> &str {
		self.as_str()
	}
}

impl From<&str> for Name {
	fn from(name: &str) -> Self {
		Self::new(name)
	}
}

impl From<String> for Name {
	fn from(name: String) -> Self {
		Self::new(name)
	}
}

#[cfg(test)]
mod tests {
	use super::Name;

	#[test]
	#[should_panic(expected = "Name cannot be empty")]
	fn names_require_visible_text() {
		let _ = Name::new("");
	}
}
