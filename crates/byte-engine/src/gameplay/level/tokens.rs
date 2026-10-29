//! Reads a baked level's tokens with serde, so each payload type decodes through its own derived code, with no text
//! parsing and no reflection.

use std::fmt;

use resource_management::resources::level::{Token, Tokens};
use serde::de::{
	self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor,
	value::{BorrowedStrDeserializer, MapDeserializer},
};

use super::ENTITY_REFERENCE;

/// The `TokenDeserializer` struct reads a run of one kind's payloads in turn, each through the payload type's
/// `Deserialize`.
///
/// An entity reference reads as the object `{ "$entity": handle }`, which [`super::EntityRef`] decodes, with the
/// handle counted from the spawn's first entity. Strings are borrowed from the payload.
pub(super) struct TokenDeserializer<'p, 's> {
	tokens: Tokens<'p, 's>,
	peeked: Option<Token<'p>>,
	/// The handle identity of the level's entity 0.
	first_entity: u32,
}

/// The `Error` struct says why a payload doesn't fit its kind, in serde's words.
#[derive(Debug)]
pub(super) struct Error(String);

impl fmt::Display for Error {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl std::error::Error for Error {}

impl de::Error for Error {
	fn custom<T: fmt::Display>(message: T) -> Self {
		Self(message.to_string())
	}
}

impl<'p, 's> TokenDeserializer<'p, 's> {
	/// Reads `tokens`, a run of a baked kind's payloads.
	pub(super) fn new(tokens: Tokens<'p, 's>, first_entity: u32) -> Self {
		Self {
			tokens,
			peeked: None,
			first_entity,
		}
	}

	fn next(&mut self) -> Result<Token<'p>, Error> {
		match self.peeked.take() {
			Some(token) => Ok(token),
			None => self.tokens.next().ok_or_else(malformed)?.map_err(|_| malformed()),
		}
	}

	fn peek(&mut self) -> Result<Token<'p>, Error> {
		let token = self.next()?;
		self.peeked = Some(token);
		Ok(token)
	}

	/// Reads the token that must close the container a visitor just read.
	fn end(&mut self, end: Token<'_>) -> Result<(), Error> {
		if self.next()? == end {
			Ok(())
		} else {
			Err(de::Error::custom("the payload has more elements than its type holds"))
		}
	}
}

impl<'de> de::Deserializer<'de> for &mut TokenDeserializer<'de, '_> {
	type Error = Error;

	fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
		match self.next()? {
			Token::Null => visitor.visit_unit(),
			Token::Bool(value) => visitor.visit_bool(value),
			Token::Unsigned(value) => visitor.visit_u64(value),
			Token::Signed(value) => visitor.visit_i64(value),
			Token::Float(value) => visitor.visit_f64(value),
			Token::String(value) => visitor.visit_borrowed_str(value),
			Token::SequenceStart => {
				let value = visitor.visit_seq(Elements { de: self })?;
				self.end(Token::SequenceEnd)?;
				Ok(value)
			}
			Token::ObjectStart => {
				let value = visitor.visit_map(Fields { de: self })?;
				self.end(Token::ObjectEnd)?;
				Ok(value)
			}
			Token::Entity(index) => {
				let handle = self.first_entity.checked_add(index).ok_or_else(malformed)?;
				let reference = std::iter::once((ENTITY_REFERENCE, u64::from(handle)));
				visitor.visit_map(MapDeserializer::<_, Error>::new(reference))
			}
			Token::Key(_) | Token::SequenceEnd | Token::ObjectEnd => Err(malformed()),
		}
	}

	fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
		if self.peek()? == Token::Null {
			self.next()?;
			visitor.visit_none()
		} else {
			visitor.visit_some(self)
		}
	}

	fn deserialize_newtype_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, Error> {
		visitor.visit_newtype_struct(self)
	}

	/// Reads a variant as its name, or as an object with one field that names it and holds its value.
	fn deserialize_enum<V: Visitor<'de>>(
		self,
		_: &'static str,
		_: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Error> {
		match self.next()? {
			Token::String(variant) => visitor.visit_enum(BorrowedStrDeserializer::new(variant)),
			Token::ObjectStart => {
				let value = visitor.visit_enum(Variant { de: self })?;
				self.end(Token::ObjectEnd)?;
				Ok(value)
			}
			_ => Err(de::Error::custom(
				"expected a variant: its name, or an object with one field named after it",
			)),
		}
	}

	serde::forward_to_deserialize_any! {
		bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit unit_struct seq
		tuple tuple_struct map struct identifier ignored_any
	}
}

/// The `Elements` struct reads a sequence's elements up to, not including, its end.
struct Elements<'a, 'p, 's> {
	de: &'a mut TokenDeserializer<'p, 's>,
}

impl<'de> SeqAccess<'de> for Elements<'_, 'de, '_> {
	type Error = Error;

	fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Error> {
		if self.de.peek()? == Token::SequenceEnd {
			return Ok(None);
		}
		seed.deserialize(&mut *self.de).map(Some)
	}
}

/// The `Fields` struct reads an object's fields up to, not including, its end.
struct Fields<'a, 'p, 's> {
	de: &'a mut TokenDeserializer<'p, 's>,
}

impl<'de> MapAccess<'de> for Fields<'_, 'de, '_> {
	type Error = Error;

	fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Error> {
		match self.de.peek()? {
			Token::ObjectEnd => Ok(None),
			Token::Key(key) => {
				self.de.next()?;
				seed.deserialize(BorrowedStrDeserializer::new(key)).map(Some)
			}
			_ => Err(malformed()),
		}
	}

	fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
		seed.deserialize(&mut *self.de)
	}
}

/// The `Variant` struct reads an object that holds one variant: its name as the key, its value as the value.
struct Variant<'a, 'p, 's> {
	de: &'a mut TokenDeserializer<'p, 's>,
}

impl<'de> EnumAccess<'de> for Variant<'_, 'de, '_> {
	type Error = Error;
	type Variant = Self;

	fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), Error> {
		let Token::Key(name) = self.de.next()? else {
			return Err(malformed());
		};
		Ok((seed.deserialize(BorrowedStrDeserializer::new(name))?, self))
	}
}

impl<'de> VariantAccess<'de> for Variant<'_, 'de, '_> {
	type Error = Error;

	fn unit_variant(self) -> Result<(), Error> {
		de::Deserialize::deserialize(&mut *self.de)
	}

	fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
		seed.deserialize(&mut *self.de)
	}

	fn tuple_variant<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Error> {
		de::Deserializer::deserialize_any(&mut *self.de, visitor)
	}

	fn struct_variant<V: Visitor<'de>>(self, _: &'static [&'static str], visitor: V) -> Result<V::Value, Error> {
		de::Deserializer::deserialize_any(&mut *self.de, visitor)
	}
}

/// Reports tokens that end early or don't hold the encoding.
fn malformed() -> Error {
	Error("the baked payload is malformed; bake the level again".to_string())
}
