/// A resource id is composed of up to three parts.
/// The base, the extension and the fragment.
///
/// "meshes/Box.gltf#texture"
///
/// "mehses/Box.gltf" is the base
/// "gltf" is the extension
/// "texture" is the fragment
///
/// Fragments like in HTTP urls, allow referencing subresources, they are useful to address elements in container formats.

use std::fmt::Debug;

pub(crate) fn get_base<'a>(url: &'a str) -> Option<&'a str> {
	let mut split = url.split('#');
	let url = split.next()?;
	if url.is_empty() {
		return None;
	}
	let path = std::path::Path::new(url);
	Some(path.to_str()?)
}

pub(crate) fn get_extension<'a>(url: &'a str) -> Option<&'a str> {
    let mut split = url.split('#');
	let url = split.next()?;
	if url.is_empty() {
		return None;
	}
	let path = std::path::Path::new(url);
	Some(path.extension()?.to_str()?)
}

pub(crate) fn get_fragment(url: &str) -> Option<&str> {
	let mut split = url.split('#');
	let _ = split.next().and_then(|x| if x.is_empty() { None } else { Some(x) })?;
	let fragment = split.next().and_then(|x| if x.is_empty() { None } else { Some(x) })?;
	if split.count() == 0 {
		Some(fragment)
	} else {
		None
	}
}

/// A `ResourceId` encapsulates and provides methods for interacting with a full resource id.
/// A resource id is composed of up to three parts.
/// The base, the extension and the fragment.
///
/// "meshes/Box.gltf#texture"
///
/// "mehses/Box.gltf" is the base
/// "gltf" is the extension
/// "texture" is the fragment
///
/// Fragments like in HTTP urls, allow referencing subresources, they are useful to address elements in container formats.
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
        ResourceIdBase { base: get_base(self.full).unwrap() }
    }

    pub fn get_extension(&self) -> &'a str {
        let mut split = self.full.split('#');
    	let url = split.next().unwrap();
    	let path = std::path::Path::new(url);
    	path.extension().unwrap().to_str().unwrap()
    }

    pub fn get_fragment(&self) -> Option<ResourceIdFragment<'a>> {
        get_fragment(self.full).map(|fragment| ResourceIdFragment { fragment })
    }
}

impl Debug for ResourceId<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.full)
    }
}

impl ToString for ResourceId<'_> {
    fn to_string(&self) -> String {
        self.full.to_string()
    }
}

impl AsRef<str> for ResourceId<'_> {
    fn as_ref(&self) -> &str {
        self.full
    }
}

impl Debug for ResourceIdBase<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.base)
    }
}

impl ToString for ResourceIdBase<'_> {
    fn to_string(&self) -> String {
        self.base.to_string()
    }
}

impl AsRef<str> for ResourceIdBase<'_> {
    fn as_ref(&self) -> &str {
        self.base
    }
}

impl Debug for ResourceIdFragment<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.fragment)
    }
}

impl ToString for ResourceIdFragment<'_> {
    fn to_string(&self) -> String {
        self.fragment.to_string()
    }
}

impl AsRef<str> for ResourceIdFragment<'_> {
    fn as_ref(&self) -> &str {
        self.fragment
    }
}

#[cfg(test)]
pub mod tests {
    use super::{get_base, get_fragment};

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
}
