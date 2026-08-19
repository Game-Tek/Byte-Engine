//! Parse resource IDs into a base path, file extension, and optional fragment.
//!
//! A fragment identifies a subresource in a container asset. For example,
//! `meshes/Box.gltf#texture` has the base `meshes/Box.gltf`, the extension
//! `gltf`, and the fragment `texture`.
use std::fmt::Debug;

pub(crate) fn get_base(url: &str) -> Option<&str> {
	let mut split = url.split('#');
	let url = split.next()?;
	if url.is_empty() {
		return None;
	}
	let path = std::path::Path::new(url);
	path.to_str()
}

pub(crate) fn get_fragment(url: &str) -> Option<&str> {
	let mut split = url.split('#');
	let _ = split.next().filter(|&x| !x.is_empty())?;
	let fragment = split.next().filter(|&x| !x.is_empty())?;
	if split.count() == 0 {
		Some(fragment)
	} else {
		None
	}
}

/// The `ResourceId` struct provides borrowed access to a full resource ID and its components.
///
/// For `meshes/Box.gltf#texture`, the base is `meshes/Box.gltf`, the extension
/// is `gltf`, and the fragment is `texture`. Use fragments to identify
/// subresources in container formats.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResourceId<'a> {
	full: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResourceIdBase<'a> {
	base: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResourceIdFragment<'a> {
	fragment: &'a str,
}

impl<'a> ResourceId<'a> {
	pub fn new(full: &'a str) -> Self {
		Self { full }
	}

	pub fn get_base(&self) -> ResourceIdBase<'a> {
		ResourceIdBase {
			base: get_base(self.full).unwrap(),
		}
	}

	pub fn get_extension(&self) -> &'a str {
		let mut split = self.full.split('#');
		let url = split.next().unwrap();
		let path = std::path::Path::new(url);
		path.extension().and_then(|extension| extension.to_str()).unwrap_or_default()
	}

	pub fn get_fragment(&self) -> Option<ResourceIdFragment<'a>> {
		get_fragment(self.full).map(|fragment| ResourceIdFragment { fragment })
	}
}

// All resource-ID views expose their borrowed component through the same formatting and conversion contract.
macro_rules! impl_resource_id_view {
	($view:ident, $field:ident) => {
		impl Debug for $view<'_> {
			fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				formatter.write_str(self.$field)
			}
		}

		impl ToString for $view<'_> {
			fn to_string(&self) -> String {
				self.$field.to_string()
			}
		}

		impl AsRef<str> for $view<'_> {
			fn as_ref(&self) -> &str {
				self.$field
			}
		}
	};
}

impl_resource_id_view!(ResourceId, full);
impl_resource_id_view!(ResourceIdBase, base);
impl_resource_id_view!(ResourceIdFragment, fragment);

#[cfg(test)]
pub mod tests {
	use super::{get_base, get_fragment, ResourceId};

	fn assert_text_view(view: &(impl AsRef<str> + std::fmt::Debug + ToString), expected: &str) {
		assert_eq!(view.as_ref(), expected);
		assert_eq!(view.to_string(), expected);
		assert_eq!(format!("{view:?}"), expected);
	}

	#[test]
	fn test_base_url_parse() {
		assert_eq!(get_base("name.extension").unwrap(), "name.extension");
		assert_eq!(get_base("name.extension#").unwrap(), "name.extension");
		assert_eq!(get_base("#fragment"), None);
		assert_eq!(get_base("name.extension#fragment").unwrap(), "name.extension");
		assert_eq!(get_base("dir/name.extension").unwrap(), "dir/name.extension");
		assert_eq!(get_base("dir/name.extension#").unwrap(), "dir/name.extension");
		assert_eq!(get_base("dir/#fragment").unwrap(), "dir/");
		assert_eq!(get_base("dir/name.extension#fragment").unwrap(), "dir/name.extension");
	}

	#[test]
	fn test_fragment_parse() {
		assert_eq!(get_fragment("name.extension"), None);
		assert_eq!(get_fragment("name.extension#"), None);
		assert_eq!(get_fragment("#fragment"), None);
		assert_eq!(get_fragment("name.extension#fragment").unwrap(), "fragment");
	}

	#[test]
	fn extensionless_resource_ids_report_an_empty_format_without_panicking() {
		assert_eq!(super::ResourceId::new("buffers/skeleton").get_extension(), "");
	}

	#[test]
	fn resource_id_views_preserve_their_exact_text_across_public_conversions() {
		let id = ResourceId::new("meshes/Box.gltf#texture");
		assert_text_view(&id, "meshes/Box.gltf#texture");
		assert_text_view(&id.get_base(), "meshes/Box.gltf");
		assert_text_view(&id.get_fragment().unwrap(), "texture");
	}
}
