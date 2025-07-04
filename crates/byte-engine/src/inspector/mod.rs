//! Byte-Engine inspector module.
//! Provides interfaces to interact with the engine's internal state.

use std::fmt::Debug;

use crate::core::Entity;

pub mod http;

pub trait Inspectable: Entity + Send + Sync {
	fn as_string(&self) -> String;

	fn class_name(&self) -> &'static str {
		std::any::type_name::<Self>()
	}

	fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
		Err("Not implemented".to_string())
	}
}
