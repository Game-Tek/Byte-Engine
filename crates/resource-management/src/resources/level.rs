//! Baked levels: entities and their components, laid out so spawning reads no text.
//!
//! Author a `.level` file and bake it into a [`Level`]. Request the level, load its payload, and hand both to the
//! engine's spawner, which decodes each kind's [`Tokens`] into components and publishes them.
//!
//! The payload holds every section a [`Section`] of the model names:
//!
//! - the parent of each entity, as little-endian `u32` entity indices, with [`NO_PARENT`] for the level's root;
//! - the `id` of each entity, as a little-endian `u32` byte length and that many UTF-8 bytes, with length 0 for none;
//! - for each kind, the index of each entity that holds one, ascending, as little-endian `u32`s;
//! - for each kind, its distinct strings, keys included, each as a little-endian `u32` byte length and its bytes;
//! - for each kind, every component's payload in entity order, as tokens that name strings by their index in the
//!   kind's strings;
//! - for each kind, where every [`RUN`]-th payload starts in its tokens, as little-endian `u32` byte offsets, so
//!   runs of payloads decode on separate threads.

/// The `Level` struct lets an application spawn a level's entities from pre-parsed sections instead of text.
///
/// Request it by the `.level` file's ID, then load the payload and pass both to the spawner. Entities are numbered
/// in the order they spawn, parents first.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Level {
	/// The number of entities the level places.
	pub entities: u32,
	/// The parent of each entity, read with [`Self::parents`].
	pub parents: Section,
	/// The `id` of each entity, read with [`Self::id`].
	pub ids: Section,
	/// The components of each kind, sorted by kind name.
	pub kinds: Vec<LevelKind>,
}

super::impl_direct_resource!(Level, "Level");

/// The `LevelKind` struct locates every component of one kind, so a spawner decodes them all in one pass.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct LevelKind {
	/// The kind name the spawner maps to a component type.
	pub name: String,
	/// The number of entities that hold a component of this kind.
	pub count: u32,
	/// The index of each entity that holds one, read with [`Self::entities`].
	pub entities: Section,
	/// The distinct strings and keys of the payloads, read with [`Self::strings`].
	pub strings: Section,
	/// The `count` payloads, one after another, as tokens.
	pub values: Section,
	/// Where each run of [`RUN`] payloads starts in `values`, read with [`Self::runs`].
	pub runs: Section,
}

/// The `Section` struct locates one byte range of a level's payload.
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct Section {
	pub offset: u64,
	pub size: u64,
}

/// The parent index of an entity that belongs to the level's root.
pub const NO_PARENT: u32 = u32::MAX;

/// How many payloads of a kind each recorded offset starts, so a spawner decodes runs of this many at once.
pub const RUN: u32 = 1024;

impl Section {
	/// Returns this section's bytes, or `None` when `payload` is too short to hold it.
	pub fn of(self, payload: &[u8]) -> Option<&[u8]> {
		let start = usize::try_from(self.offset).ok()?;
		let end = start.checked_add(usize::try_from(self.size).ok()?)?;
		payload.get(start..end)
	}
}

impl Level {
	/// Returns the parent of each entity in spawn order, `None` for an entity that belongs to the level's root.
	///
	/// Returns `None` when `payload` doesn't hold the section.
	pub fn parents<'p>(&self, payload: &'p [u8]) -> Option<impl ExactSizeIterator<Item = Option<u32>> + 'p> {
		Some(words(self.parents.of(payload)?).map(|parent| (parent != NO_PARENT).then_some(parent)))
	}

	/// Returns the `id` of `entity`, or `None` when it has none.
	///
	/// Scans the ids in order, so use it for messages, not on the spawning path.
	pub fn id<'p>(&self, payload: &'p [u8], entity: u32) -> Option<&'p str> {
		let mut ids = self.ids.of(payload)?;
		for index in 0..=entity {
			let (length, rest) = ids.split_first_chunk::<4>()?;
			let (id, rest) = rest.split_at_checked(u32::from_le_bytes(*length) as usize)?;
			if index == entity {
				return std::str::from_utf8(id).ok().filter(|id| !id.is_empty());
			}
			ids = rest;
		}
		None
	}
}

impl LevelKind {
	/// Returns the index of each entity that holds a component of this kind, ascending.
	pub fn entities<'p>(&self, payload: &'p [u8]) -> Option<impl ExactSizeIterator<Item = u32> + 'p> {
		Some(words(self.entities.of(payload)?))
	}

	/// Returns this kind's distinct strings, checked once so reading a token never checks one again.
	///
	/// Returns `None` when `payload` doesn't hold the section or a string isn't UTF-8.
	pub fn strings<'p>(&self, payload: &'p [u8]) -> Option<Vec<&'p str>> {
		let mut bytes = self.strings.of(payload)?;
		let mut strings = Vec::new();
		while let Some((length, rest)) = bytes.split_first_chunk::<4>() {
			let (text, rest) = rest.split_at_checked(u32::from_le_bytes(*length) as usize)?;
			strings.push(std::str::from_utf8(text).ok()?);
			bytes = rest;
		}
		Some(strings)
	}

	/// Returns the tokens of the run of payloads that starts with payload `run * RUN`, one value for each of its
	/// entities in [`Self::entities`].
	///
	/// `strings` is what [`Self::strings`] returned. The tokens continue past the run, so read only its payloads.
	pub fn run<'p, 's>(&self, payload: &'p [u8], strings: &'s [&'p str], run: u32) -> Option<Tokens<'p, 's>> {
		let offset = run as usize * 4;
		let start = self.runs.of(payload)?.get(offset..offset + 4)?;
		let start = u32::from_le_bytes(start.try_into().ok()?) as usize;
		Some(Tokens::new(self.values.of(payload)?.get(start..)?, strings))
	}

	/// Returns how many runs of up to [`RUN`] payloads this kind has.
	pub fn run_count(&self) -> u32 {
		self.count.div_ceil(RUN)
	}
}

/// Reads `bytes` as little-endian `u32`s.
fn words(bytes: &[u8]) -> impl ExactSizeIterator<Item = u32> + '_ {
	bytes.as_chunks::<4>().0.iter().map(|word| u32::from_le_bytes(*word))
}

/// The `Token` enum is one parsed piece of a component payload, so spawning never parses text or numbers.
///
/// A sequence or object starts with its start token and ends with its end token; an object holds a [`Token::Key`]
/// before each value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Token<'a> {
	Null,
	Bool(bool),
	Unsigned(u64),
	/// A negative integer.
	Signed(i64),
	Float(f64),
	String(&'a str),
	SequenceStart,
	SequenceEnd,
	ObjectStart,
	Key(&'a str),
	ObjectEnd,
	/// A reference to the entity with this index in the level.
	Entity(u32),
}

// One byte tags each token; the tag order is the encoding and must not change without a store version bump.
const NULL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const UNSIGNED: u8 = 3;
const SIGNED: u8 = 4;
const FLOAT: u8 = 5;
const STRING: u8 = 6;
const SEQUENCE_START: u8 = 7;
const SEQUENCE_END: u8 = 8;
const OBJECT_START: u8 = 9;
const KEY: u8 = 10;
const OBJECT_END: u8 = 11;
const ENTITY: u8 = 12;

/// The `TokenWriter` struct encodes one kind's payloads, storing each distinct string once.
#[derive(Default)]
pub struct TokenWriter {
	bytes: Vec<u8>,
	strings: Vec<String>,
	indices: std::collections::HashMap<String, u32>,
}

impl TokenWriter {
	/// Appends `token` in the payload encoding that [`Tokens`] reads.
	pub fn write(&mut self, token: Token<'_>) {
		match token {
			Token::Null => self.bytes.push(NULL),
			Token::Bool(value) => self.bytes.push(if value { TRUE } else { FALSE }),
			Token::Unsigned(value) => {
				self.bytes.push(UNSIGNED);
				self.bytes.extend_from_slice(&value.to_le_bytes());
			}
			Token::Signed(value) => {
				self.bytes.push(SIGNED);
				self.bytes.extend_from_slice(&value.to_le_bytes());
			}
			Token::Float(value) => {
				self.bytes.push(FLOAT);
				self.bytes.extend_from_slice(&value.to_le_bytes());
			}
			Token::String(value) => self.write_string(STRING, value),
			Token::SequenceStart => self.bytes.push(SEQUENCE_START),
			Token::SequenceEnd => self.bytes.push(SEQUENCE_END),
			Token::ObjectStart => self.bytes.push(OBJECT_START),
			Token::Key(value) => self.write_string(KEY, value),
			Token::ObjectEnd => self.bytes.push(OBJECT_END),
			Token::Entity(index) => {
				self.bytes.push(ENTITY);
				self.bytes.extend_from_slice(&index.to_le_bytes());
			}
		}
	}

	/// Returns where the next token starts, in bytes from the first.
	pub fn offset(&self) -> usize {
		self.bytes.len()
	}

	/// Appends a tagged string as its index in the kind's strings, adding it the first time it appears.
	fn write_string(&mut self, tag: u8, text: &str) {
		let index = match self.indices.get(text) {
			Some(&index) => index,
			None => {
				let index = self.strings.len() as u32;
				self.strings.push(text.to_string());
				self.indices.insert(text.to_string(), index);
				index
			}
		};
		self.bytes.push(tag);
		self.bytes.extend_from_slice(&index.to_le_bytes());
	}

	/// Returns the encoded tokens and the strings section that [`LevelKind::strings`] reads.
	pub fn finish(self) -> (Vec<u8>, Vec<u8>) {
		let mut strings = Vec::new();
		for text in &self.strings {
			strings.extend_from_slice(&(text.len() as u32).to_le_bytes());
			strings.extend_from_slice(text.as_bytes());
		}
		(self.bytes, strings)
	}
}

/// The `Tokens` struct reads a component payload sequence in order, borrowing its strings from the payload.
#[derive(Debug, Clone)]
pub struct Tokens<'a, 's> {
	bytes: &'a [u8],
	/// The kind's strings, which string and key tokens name by index.
	strings: &'s [&'a str],
}

/// The `MalformedTokens` struct reports a payload that doesn't hold the token encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MalformedTokens;

impl std::fmt::Display for MalformedTokens {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(
			"Level payload tokens are malformed. The most likely cause is a level baked by another engine version. Bake the level again.",
		)
	}
}

impl std::error::Error for MalformedTokens {}

impl<'a> Iterator for Tokens<'a, '_> {
	type Item = Result<Token<'a>, MalformedTokens>;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		let (&tag, rest) = self.bytes.split_first()?;
		self.bytes = rest;
		Some(self.token(tag))
	}
}

impl<'a, 's> Tokens<'a, 's> {
	/// Reads tokens from `bytes`, whose string and key tokens name `strings` by index.
	pub fn new(bytes: &'a [u8], strings: &'s [&'a str]) -> Self {
		Self { bytes, strings }
	}

	/// Reads the token that starts with `tag`.
	#[inline]
	fn token(&mut self, tag: u8) -> Result<Token<'a>, MalformedTokens> {
		Ok(match tag {
			NULL => Token::Null,
			FALSE => Token::Bool(false),
			TRUE => Token::Bool(true),
			UNSIGNED => Token::Unsigned(u64::from_le_bytes(self.take()?)),
			SIGNED => Token::Signed(i64::from_le_bytes(self.take()?)),
			FLOAT => Token::Float(f64::from_le_bytes(self.take()?)),
			STRING => Token::String(self.text()?),
			SEQUENCE_START => Token::SequenceStart,
			SEQUENCE_END => Token::SequenceEnd,
			OBJECT_START => Token::ObjectStart,
			KEY => Token::Key(self.text()?),
			OBJECT_END => Token::ObjectEnd,
			ENTITY => Token::Entity(u32::from_le_bytes(self.take()?)),
			_ => return Err(MalformedTokens),
		})
	}

	/// Takes the next `N` bytes.
	#[inline]
	fn take<const N: usize>(&mut self) -> Result<[u8; N], MalformedTokens> {
		let (taken, rest) = self.bytes.split_first_chunk::<N>().ok_or(MalformedTokens)?;
		self.bytes = rest;
		Ok(*taken)
	}

	/// Takes a string named by its index in the kind's strings.
	#[inline]
	fn text(&mut self) -> Result<&'a str, MalformedTokens> {
		let index = u32::from_le_bytes(self.take()?) as usize;
		self.strings.get(index).copied().ok_or(MalformedTokens)
	}
}
