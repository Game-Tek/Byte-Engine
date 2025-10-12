use crate::{os::WindowLike, Events};

pub struct Window {

}

pub struct Handles {

}

impl WindowLike for Window {
	fn try_new(name: &str, extent: utils::Extent, id_name: &str) -> Result<Self, String> {
		Err("Not implemented".to_string())
	}

	fn poll(&mut self) -> impl Iterator<Item = Events> {
		std::iter::empty()
	}

	fn handles(&self) -> Handles {
		Handles {}
	}
}
