use std::hash::Hasher;

use serde::{ser::SerializeStruct, Serialize};

use crate::{
	asset::ResourceId,
	resource::{resource_handler::MultiResourceReader, ReadTargets, ReadTargetsMut},
	to_vec, DataStorage, LoadResults, Model, Resource, StreamDescription,
};

#[derive(Debug)]
/// Represents a resource reference and can be use to embed resources in other resources.
pub struct Reference<T: Resource> {
	pub id: String,
	pub hash: u64,
	pub size: usize,
	pub resource: T,
	reader: Option<MultiResourceReader>,
	streams: Option<Vec<StreamDescription>>,
}

impl<'a, T: Resource + 'a> Serialize for Reference<T> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		let mut state = serializer.serialize_struct("TypedDocument", 3)?;

		state.serialize_field("id", &self.id)?;
		state.serialize_field("hash", &self.hash)?;
		state.serialize_field("class", &self.resource.get_class())?;

		state.end()
	}
}

impl<'a, T: Resource + 'a> Reference<T> {
	pub fn from_model(model: ReferenceModel<T::Model>, resource: T, reader: MultiResourceReader) -> Self {
		Reference {
			id: model.id,
			hash: model.hash,
			size: model.size,
			resource,
			reader: Some(reader),
			streams: model.streams,
		}
	}

	pub fn id(&self) -> &str {
		&self.id
	}

	pub fn hash(&self) -> u64 {
		self.hash
	}

	pub fn get_hash(&self) -> u64 {
		self.hash
	}

	pub fn resource(&self) -> &T {
		&self.resource
	}

	pub fn resource_mut(&mut self) -> &mut T {
		&mut self.resource
	}

	pub fn into_resource(self) -> T {
		self.resource
	}

	pub fn consume_reader(&mut self) -> MultiResourceReader {
		self.reader.take().unwrap()
	}

	pub fn map(self, f: impl FnOnce(T) -> T) -> Self {
		Reference {
			resource: f(self.resource),
			..self
		}
	}

	/// Loads the resource's binary data from the storage backend.
	///
	/// If `read_target` requests backing storage, the reader serves resource-owned bytes directly.
	/// File-backed resources use mapped files when the storage backend supports them. If direct
	/// backing storage is unavailable, the resource falls back to an owned buffer. Explicit buffer,
	/// box, and stream targets are still filled by reading into the caller-selected target.
	pub fn load<'s>(&'s mut self, read_target: ReadTargetsMut<'a>) -> Result<ReadTargets<'a>, LoadResults> {
		let reader = self.reader.take().ok_or(LoadResults::NoReadTarget)?;

		if matches!(read_target, ReadTargetsMut::BackingStorage) {
			return match reader.into_backing_storage() {
				Ok(backing) => Ok(ReadTargets::Backing(backing)),
				Err(mut reader) => {
					let read_target = ReadTargetsMut::create_buffer(self);
					reader
						.read_into(self.streams.as_ref().map(|s| s.as_slice()), read_target)
						.map_err(|_| LoadResults::LoadFailed)
				}
			};
		}

		let mut reader = reader;
		reader
			.read_into(self.streams.as_ref().map(|s| s.as_slice()), read_target)
			.map_err(|_| LoadResults::LoadFailed)
	}
}

impl<T: Resource> std::hash::Hash for Reference<T> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.id.hash(state);
		self.hash.hash(state);
		self.size.hash(state);
		self.resource.get_class().hash(state);
	}
}

#[derive(Clone, Debug, serde::Deserialize, Serialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ReferenceModel<T: Model> {
	id: String,
	hash: u64,
	size: usize,
	class: String,
	pub(crate) resource: DataStorage, // TODO: remove this visibility and use proper methods
	#[serde(skip)]
	#[rkyv(with = rkyv::with::Skip)]
	phantom: std::marker::PhantomData<T>,
	streams: Option<Vec<StreamDescription>>,
}

impl<T: Model> ReferenceModel<T> {
	pub fn new(id: &str, hash: u64, size: usize, resource: &T, streams: Option<Vec<StreamDescription>>) -> Self {
		ReferenceModel {
			id: id.to_string(),
			hash,
			size,
			class: T::get_class().to_string(),
			resource: to_vec(resource).unwrap(),
			phantom: std::marker::PhantomData,
			streams,
		}
	}

	pub fn new_serialized(
		id: &str,
		hash: u64,
		size: usize,
		resource: DataStorage,
		streams: Option<Vec<StreamDescription>>,
	) -> Self {
		ReferenceModel {
			id: id.to_string(),
			hash,
			size,
			class: T::get_class().to_string(),
			resource,
			phantom: std::marker::PhantomData,
			streams,
		}
	}

	pub fn id(&self) -> ResourceId<'_> {
		ResourceId::new(&self.id)
	}

	pub fn class(&self) -> &str {
		&self.class
	}
}

#[cfg(test)]
mod tests {
	use std::{
		fs,
		io::Write,
		path::PathBuf,
		time::{SystemTime, UNIX_EPOCH},
	};

	use crate::{
		resource::{
			reader::{redb::FileResourceReader, ResourceReaderBacking},
			ReadTargets, ReadTargetsMut,
		},
		Model, Resource,
	};

	use super::{Reference, ReferenceModel};

	#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
	/// The `DefaultLoadModel` struct gives the reference test a serializable resource model.
	struct DefaultLoadModel;

	impl Model for DefaultLoadModel {
		fn get_class() -> &'static str {
			"DefaultLoad"
		}
	}

	#[derive(Debug)]
	/// The `DefaultLoadResource` struct gives the reference test a concrete resource type.
	struct DefaultLoadResource;

	impl Resource for DefaultLoadResource {
		type Model = DefaultLoadModel;

		fn get_class(&self) -> &'static str {
			DefaultLoadModel::get_class()
		}
	}

	fn temporary_file_path() -> PathBuf {
		std::env::temp_dir().join(format!(
			"byte-engine-reference-default-load-{}-{}.bin",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		))
	}

	#[test]
	fn default_reference_load_uses_reader_backing_storage() {
		let path = temporary_file_path();
		let expected = b"default-load-bytes";

		{
			let mut file = fs::File::create(&path).unwrap();
			file.write_all(expected).unwrap();
			file.sync_all().unwrap();
		}

		let model = ReferenceModel::<DefaultLoadModel>::new("default-load", 0, expected.len(), &DefaultLoadModel, None);
		let reader = Box::new(FileResourceReader::new(fs::File::open(&path).unwrap()));
		let mut reference = Reference::from_model(model, DefaultLoadResource, reader);
		let target = ReadTargetsMut::from(&reference);
		let result = reference.load(target).unwrap();

		assert_eq!(result.buffer().unwrap(), expected);
		assert!(matches!(result, ReadTargets::Backing(ResourceReaderBacking::MappedFile(_))));

		fs::remove_file(path).unwrap();
	}
}
