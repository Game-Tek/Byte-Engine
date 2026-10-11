//! The `store` module keeps compiled pipeline binaries in one buffer that is also the persisted cache.
//!
//! It knows nothing about Vulkan calls. [`super::PipelineBinaries`] appends what the driver hands out to a [`Store`]
//! and lends the stored bytes back to the driver when a pipeline is created again.
//!
//! The buffer is an append-only log: a header naming the driver configuration, then records. A binary record is a
//! tag, the binary's key, a size, and the data. A pipeline record is a tag, the pipeline's key, a count, and the keys
//! of its binaries in driver order. A shader record is a tag, the caller's key for the shader's code, and the
//! driver's identifier for that code. Counts and sizes are little-endian. Because records are only ever appended, the
//! bytes that were loaded stay a prefix of the buffer, and saving is appending the rest to the same file.

use std::{mem::MaybeUninit, ops::Range};

use smallvec::SmallVec;
use utils::hash::HashMap;

/// The longest key a driver hands out, `VK_MAX_PIPELINE_BINARY_KEY_SIZE_KHR`.
const MAX_KEY_SIZE: usize = 32;
/// A key in the log: its size, then its bytes padded to the longest key.
const KEY_RECORD_SIZE: usize = 1 + MAX_KEY_SIZE;

/// Identifies the log layout. A log with another tag is ignored.
const MAGIC: [u8; 8] = *b"BEPIPBN2";
const HEADER_SIZE: usize = MAGIC.len() + KEY_RECORD_SIZE;

const BINARY_TAG: u8 = 1;
const PIPELINE_TAG: u8 = 2;
const SHADER_TAG: u8 = 3;

/// The `Key` struct names a pipeline, a pipeline binary, or a driver configuration with the opaque bytes the driver
/// chose for it.
///
/// Bytes past `size` are always zero, so two keys are equal exactly when the driver's keys are.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Key {
	size: u8,
	bytes: [u8; MAX_KEY_SIZE],
}

impl Key {
	/// Returns the key for `bytes`, or `None` when they are longer than any driver key.
	pub(crate) fn new(bytes: &[u8]) -> Option<Self> {
		let mut key = Self {
			size: u8::try_from(bytes.len()).ok().filter(|&size| size as usize <= MAX_KEY_SIZE)?,
			bytes: [0; MAX_KEY_SIZE],
		};
		key.bytes[..bytes.len()].copy_from_slice(bytes);
		Some(key)
	}

	pub(crate) fn bytes(&self) -> &[u8] {
		&self.bytes[..self.size as usize]
	}

	/// Reads a key from its log form. Rebuilding from the used bytes rejects oversized keys and zeroes the padding.
	fn read(record: &[u8]) -> Option<Self> {
		let (size, bytes) = record.get(..KEY_RECORD_SIZE)?.split_first()?;
		Self::new(bytes.get(..*size as usize)?)
	}

	fn write(&self, log: &mut Vec<u8>) {
		log.push(self.size);
		log.extend_from_slice(&self.bytes);
	}
}

/// The `Store` struct remembers which binaries each pipeline is made of, so a pipeline can be created again without
/// compiling its shaders.
///
/// It owns one buffer, the log, and indexes into it, so loading copies no binary, creating a stored pipeline lends
/// the driver bytes of the log, and saving writes the log as it is. Pipelines that share a shader share its binary.
/// It also remembers the driver's identifier for each shader the caller names with a key, so a later run can look
/// pipelines up without reading the shader's code. Start one with [`Self::new`] or [`Self::load`], fill it with
/// [`Self::insert`] and [`Self::insert_shader`], read it with [`Self::binaries`] and [`Self::shader_identifier`],
/// and persist it with [`Self::bytes`].
pub(crate) struct Store {
	log: Vec<u8>,
	/// How much of the front of `log` is, byte for byte, the blob it was loaded from.
	loaded: usize,
	/// Where each pipeline's binary keys start in `log`, and how many there are.
	pipelines: HashMap<Key, (usize, u32)>,
	/// Where each binary's data lies in `log`.
	binaries: HashMap<Key, Range<usize>>,
	/// The driver's identifier for each shader, by the caller's key for the shader's code.
	shaders: HashMap<u64, Key>,
}

impl Store {
	/// Starts an empty store for the driver configuration `identity` names.
	pub(crate) fn new(identity: &Key) -> Self {
		let mut log = Vec::with_capacity(HEADER_SIZE);
		log.extend_from_slice(&MAGIC);
		identity.write(&mut log);
		Self {
			log,
			loaded: 0,
			pipelines: HashMap::default(),
			binaries: HashMap::default(),
			shaders: HashMap::default(),
		}
	}

	/// Adopts a blob an earlier store lent out through [`Self::bytes`], without copying it.
	///
	/// A blob of another `identity`, such as a different driver version, gives an empty store. A blob cut short, as
	/// by an interrupted write, keeps the records before the cut. Either way [`Self::bytes`] then reports nothing as
	/// loaded, so the caller replaces the damaged blob instead of appending to it. A cache can always be rebuilt, so
	/// bad data is dropped instead of reported.
	pub(crate) fn load(identity: &Key, mut blob: Vec<u8>) -> Self {
		let header_matches = blob.get(..MAGIC.len()) == Some(&MAGIC[..])
			&& blob.get(MAGIC.len()..).and_then(Key::read).as_ref() == Some(identity);
		if !header_matches {
			return Self::new(identity);
		}

		// Counting first sizes every index exactly, so indexing allocates once per index no matter how many records
		// there are.
		let (mut pipeline_count, mut binary_count, mut shader_count) = (0, 0, 0);
		let valid = scan(&blob, |record| match record {
			Record::Binary { .. } => binary_count += 1,
			Record::Pipeline { .. } => pipeline_count += 1,
			Record::Shader { .. } => shader_count += 1,
		});

		let mut pipelines = HashMap::with_capacity_and_hasher(pipeline_count, Default::default());
		let mut binaries = HashMap::with_capacity_and_hasher(binary_count, Default::default());
		let mut shaders = HashMap::with_capacity_and_hasher(shader_count, Default::default());
		scan(&blob[..valid], |record| match record {
			Record::Shader { shader, identifier } => {
				shaders.insert(shader, identifier);
			}
			Record::Binary { key, data } => {
				binaries.insert(key, data);
			}
			Record::Pipeline { key, keys, count } => {
				// Binaries are logged before the pipeline that lists them, so a listed key without data means a
				// damaged record. `Self::binaries` relies on every listed key having data.
				let listed = blob[keys..].as_chunks::<KEY_RECORD_SIZE>().0;
				let mut listed = listed.iter().take(count as usize).map(|record| Key::read(record));
				if listed.all(|key| key.is_some_and(|key| binaries.contains_key(&key))) {
					pipelines.insert(key, (keys, count));
				}
			}
		});

		let loaded = if valid == blob.len() { valid } else { 0 };
		blob.truncate(valid);
		Self {
			log: blob,
			loaded,
			pipelines,
			binaries,
			shaders,
		}
	}

	/// Returns the driver's identifier for the shader whose code the caller names `shader`, when it is stored.
	pub(crate) fn shader_identifier(&self, shader: u64) -> Option<Key> {
		self.shaders.get(&shader).copied()
	}

	/// Appends the driver's identifier for the shader whose code the caller names `shader`.
	pub(crate) fn insert_shader(&mut self, shader: u64, identifier: Key) {
		if self.shaders.insert(shader, identifier) == Some(identifier) {
			return;
		}
		self.log.push(SHADER_TAG);
		shader_key(shader).write(&mut self.log);
		identifier.write(&mut self.log);
	}

	/// Returns the binaries of a pipeline in driver order, borrowed from the log, or `None` when the pipeline is not
	/// stored.
	pub(crate) fn binaries(&self, pipeline: &Key) -> Option<impl ExactSizeIterator<Item = (Key, &[u8])>> {
		let &(keys, count) = self.pipelines.get(pipeline)?;
		let keys = self.log[keys..keys + count as usize * KEY_RECORD_SIZE]
			.as_chunks::<KEY_RECORD_SIZE>()
			.0;
		Some(keys.iter().map(|record| {
			// Only pipelines whose keys all parsed and have data are indexed.
			let key = Key::read(record).expect(
				"Pipeline binary key is malformed. The most likely cause is a backend bug that indexed an unchecked pipeline record.",
			);
			(key, &self.log[self.binaries[&key].clone()])
		}))
	}

	/// Appends a pipeline as the ordered list of its binaries, and the data of every binary the store lacks.
	///
	/// `binaries` gives each binary's key and data size. For each one the store lacks, `fetch` is called with the
	/// binary's position in `binaries` and the unwritten log bytes its data belongs in, so the data is produced in
	/// place instead of copied in. When `fetch` returns `false`, nothing is stored and this returns `false`.
	///
	/// # Safety
	///
	/// `fetch` must initialize every byte of the slice it is given whenever it returns `true`.
	pub(crate) unsafe fn insert(
		&mut self,
		pipeline: Key,
		binaries: &[(Key, usize)],
		mut fetch: impl FnMut(usize, &mut [MaybeUninit<u8>]) -> bool,
	) -> bool {
		let checkpoint = self.log.len();
		// Indexed only once the whole pipeline is in the log, so a failed fetch leaves no entry pointing past the end.
		let mut added = SmallVec::<[(Key, Range<usize>); 4]>::new();

		for (position, &(key, size)) in binaries.iter().enumerate() {
			if self.binaries.contains_key(&key) || added.iter().any(|(added, _)| *added == key) {
				continue;
			}
			self.log.push(BINARY_TAG);
			key.write(&mut self.log);
			self.log.extend_from_slice(&(size as u64).to_le_bytes());
			self.log.reserve(size);
			let start = self.log.len();
			if !fetch(position, &mut self.log.spare_capacity_mut()[..size]) {
				self.log.truncate(checkpoint);
				return false;
			}
			// SAFETY: `size` bytes were reserved, and the caller guarantees a successful `fetch` initialized them.
			unsafe { self.log.set_len(start + size) };
			added.push((key, start..start + size));
		}

		self.log.push(PIPELINE_TAG);
		pipeline.write(&mut self.log);
		self.log.extend_from_slice(&(binaries.len() as u32).to_le_bytes());
		let keys = self.log.len();
		for (key, _) in binaries {
			key.write(&mut self.log);
		}

		self.binaries.extend(added);
		self.pipelines.insert(pipeline, (keys, binaries.len() as u32));
		true
	}

	/// Lends the log as the part that was loaded and the part added since.
	///
	/// Persist the store by appending the second part to the blob the first part was loaded from. When the first part
	/// is empty, nothing usable was loaded and the second part is the whole log, which replaces any earlier blob.
	pub(crate) fn bytes(&self) -> (&[u8], &[u8]) {
		self.log.split_at(self.loaded)
	}
}

/// The `Record` enum is one entry of the log, as positions into it.
enum Record {
	Binary {
		key: Key,
		data: Range<usize>,
	},
	/// `keys` is where the pipeline's `count` binary keys start.
	Pipeline {
		key: Key,
		keys: usize,
		count: u32,
	},
	Shader {
		shader: u64,
		identifier: Key,
	},
}

/// Returns the log form of a caller's shader key. Every record starts with a [`Key`], so the eight bytes ride in one.
fn shader_key(shader: u64) -> Key {
	Key::new(&shader.to_le_bytes())
		.expect("Shader key does not fit a log key. The most likely cause is a backend bug that shrank the log's key size.")
}

/// Visits every whole record after the header and returns where the well-formed part of the log ends.
///
/// It reads record headers only and skips over binary data, so scanning does not touch most of the log.
fn scan(log: &[u8], mut visit: impl FnMut(Record)) -> usize {
	let mut position = HEADER_SIZE;
	let mut read = || {
		let &tag = log.get(position)?;
		let key = Key::read(log.get(position + 1..)?)?;
		let body = position + 1 + KEY_RECORD_SIZE;
		let (record, end) = match tag {
			BINARY_TAG => {
				let size = usize::try_from(u64::from_le_bytes(log.get(body..body + 8)?.try_into().ok()?)).ok()?;
				let data = body + 8..(body + 8).checked_add(size)?;
				(Record::Binary { key, data: data.clone() }, data.end)
			}
			PIPELINE_TAG => {
				let count = u32::from_le_bytes(log.get(body..body + 4)?.try_into().ok()?);
				let end = (body + 4).checked_add((count as usize).checked_mul(KEY_RECORD_SIZE)?)?;
				(
					Record::Pipeline {
						key,
						keys: body + 4,
						count,
					},
					end,
				)
			}
			SHADER_TAG => {
				let shader = u64::from_le_bytes(key.bytes().try_into().ok()?);
				let identifier = Key::read(log.get(body..)?)?;
				(Record::Shader { shader, identifier }, body + KEY_RECORD_SIZE)
			}
			_ => return None,
		};
		// A record that runs past the end was cut short.
		(end <= log.len()).then(|| {
			position = end;
			record
		})
	};
	while let Some(record) = read() {
		visit(record);
	}
	position.min(log.len())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn key(byte: u8) -> Key {
		Key::new(&[byte; 20]).unwrap()
	}

	/// Inserts a pipeline whose binaries each hold 64 copies of their fill byte, and returns which were fetched.
	fn insert(store: &mut Store, pipeline: Key, binaries: &[(Key, u8)]) -> Vec<Key> {
		let sized = binaries.iter().map(|&(key, _)| (key, 64)).collect::<Vec<_>>();
		let mut fetched = Vec::new();
		// SAFETY: The closure writes every byte of the slice before it returns `true`.
		let stored = unsafe {
			store.insert(pipeline, &sized, |position, data| {
				data.fill(MaybeUninit::new(binaries[position].1));
				fetched.push(binaries[position].0);
				true
			})
		};
		assert!(stored);
		fetched
	}

	fn stored(store: &Store, pipeline: &Key) -> Option<Vec<(Key, Vec<u8>)>> {
		Some(store.binaries(pipeline)?.map(|(key, data)| (key, data.to_vec())).collect())
	}

	fn blob(store: &Store) -> Vec<u8> {
		let (loaded, added) = store.bytes();
		[loaded, added].concat()
	}

	#[test]
	fn pipelines_return_their_binaries_in_insertion_order() {
		let mut store = Store::new(&key(99));
		insert(&mut store, key(1), &[(key(11), 1), (key(10), 2)]);

		assert_eq!(
			stored(&store, &key(1)),
			Some(vec![(key(11), vec![1; 64]), (key(10), vec![2; 64])])
		);
		assert!(store.binaries(&key(2)).is_none());
	}

	#[test]
	fn binary_the_store_holds_is_not_fetched_again() {
		let mut store = Store::new(&key(99));
		insert(&mut store, key(1), &[(key(10), 1)]);

		let fetched = insert(&mut store, key(2), &[(key(10), 9), (key(11), 2)]);

		assert_eq!(fetched, [key(11)]);
		assert_eq!(
			stored(&store, &key(2)),
			Some(vec![(key(10), vec![1; 64]), (key(11), vec![2; 64])])
		);
	}

	#[test]
	fn failed_fetch_stores_nothing() {
		let mut store = Store::new(&key(99));
		insert(&mut store, key(1), &[(key(10), 1)]);
		let before = blob(&store);

		// SAFETY: The closure only returns `true` after writing every byte of the slice.
		let inserted = unsafe {
			store.insert(key(2), &[(key(11), 64), (key(12), 64)], |position, data| {
				data.fill(MaybeUninit::new(7));
				position == 0
			})
		};

		assert!(!inserted);
		assert!(store.binaries(&key(2)).is_none());
		assert_eq!(blob(&store), before);
	}

	#[test]
	fn loaded_store_returns_what_was_stored() {
		let mut store = Store::new(&key(99));
		insert(&mut store, key(1), &[(key(10), 1), (key(11), 2)]);
		insert(&mut store, key(2), &[(key(10), 1)]);

		let restored = Store::load(&key(99), blob(&store));

		assert_eq!(stored(&restored, &key(1)), stored(&store, &key(1)));
		assert_eq!(stored(&restored, &key(2)), stored(&store, &key(2)));
	}

	#[test]
	fn loaded_store_lends_only_what_was_added_since() {
		let mut store = Store::new(&key(99));
		insert(&mut store, key(1), &[(key(10), 1)]);
		let saved = blob(&store);

		let mut restored = Store::load(&key(99), saved.clone());
		assert_eq!(restored.bytes(), (&saved[..], &[][..]));

		insert(&mut restored, key(2), &[(key(11), 2)]);
		let (loaded, added) = restored.bytes();
		assert_eq!(loaded, saved);

		// Appending what was added to what was saved gives a blob with both pipelines.
		let appended = Store::load(&key(99), [loaded, added].concat());
		assert_eq!(stored(&appended, &key(1)), stored(&restored, &key(1)));
		assert_eq!(stored(&appended, &key(2)), stored(&restored, &key(2)));
	}

	#[test]
	fn loaded_store_returns_shader_identifiers() {
		let mut store = Store::new(&key(99));
		store.insert_shader(7, key(70));
		insert(&mut store, key(1), &[(key(10), 1)]);
		store.insert_shader(8, key(80));
		let saved = blob(&store);
		// Naming a shader the store already knows the same way adds nothing to save.
		store.insert_shader(7, key(70));
		assert_eq!(blob(&store), saved);

		let restored = Store::load(&key(99), saved);

		assert_eq!(restored.shader_identifier(7), Some(key(70)));
		assert_eq!(restored.shader_identifier(8), Some(key(80)));
		assert_eq!(restored.shader_identifier(9), None);
		assert_eq!(stored(&restored, &key(1)), stored(&store, &key(1)));
	}

	#[test]
	fn blob_from_another_identity_is_replaced() {
		let mut store = Store::new(&key(99));
		insert(&mut store, key(1), &[(key(10), 1)]);

		let restored = Store::load(&key(98), blob(&store));

		assert!(restored.binaries(&key(1)).is_none());
		// Nothing counts as loaded, so the caller writes a fresh blob for the new identity over the old one.
		let (loaded, added) = restored.bytes();
		assert!(loaded.is_empty());
		assert_eq!(added, blob(&Store::new(&key(98))));
	}

	#[test]
	fn truncated_blob_keeps_whole_records_and_is_replaced() {
		let mut store = Store::new(&key(99));
		insert(&mut store, key(1), &[(key(10), 1)]);
		let first = blob(&store);
		insert(&mut store, key(2), &[(key(11), 2)]);
		let both = blob(&store);

		for size in 0..both.len() {
			let restored = Store::load(&key(99), both[..size].to_vec());
			assert!(
				restored.binaries(&key(2)).is_none(),
				"A blob cut to {size} bytes kept its cut pipeline."
			);
			assert_eq!(
				restored.binaries(&key(1)).is_some(),
				size >= first.len(),
				"A blob cut to {size} bytes."
			);

			let (loaded, added) = restored.bytes();
			assert!(both.starts_with(&[loaded, added].concat()));
			// A cut inside a record leaves bytes the store dropped, so the blob must be replaced, not appended to.
			if loaded.len() + added.len() != size {
				assert!(loaded.is_empty(), "A blob cut to {size} bytes would be appended to.");
			}
		}
	}
}
