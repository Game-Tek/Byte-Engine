use std::hash::Hasher;

use serde::{ser::SerializeStruct, Deserialize, Serialize};

use crate::{asset::ResourceId, resource::{resource_handler::MultiResourceReader, ReadTargets, ReadTargetsMut}, DataStorage, LoadResults, Model, Resource, StreamDescription};

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

	pub fn get_hash(&self) -> u64 { self.hash }

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

	/// Loads the resource's binary data into memory from the storage backend.
	pub fn load<'s>(&'s mut self, read_target: ReadTargetsMut<'a>) -> Result<ReadTargets<'a>, LoadResults> {
		let mut reader = self.reader.take().ok_or(LoadResults::NoReadTarget)?;
		reader.read_into(self.streams.as_ref().map(|s| s.as_slice()), read_target).map_err(|_| LoadResults::LoadFailed)
	}
}

impl <T: Resource> std::hash::Hash for Reference<T> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.id.hash(state); self.hash.hash(state); self.size.hash(state); self.resource.get_class().hash(state);
	}
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReferenceModel<T: Model> {
    id: String,
    hash: u64,
	size: usize,
    class: String,
    pub(crate) resource: DataStorage, // TODO: remove this visibility and use proper methods
    #[serde(skip)]
    phantom: std::marker::PhantomData<T>,
	streams: Option<Vec<StreamDescription>>,
}

impl<T: Model> ReferenceModel<T> {
    pub fn new(id: &str, hash: u64, size: usize, resource: &T, streams: Option<Vec<StreamDescription>>) -> Self where T: Serialize {
        ReferenceModel {
            id: id.to_string(),
            hash,
			size,
            class: T::get_class().to_string(),
            resource: pot::to_vec(resource).unwrap(),
            phantom: std::marker::PhantomData,
			streams,
        }
    }

    pub fn new_serialized(id: &str, hash: u64, size: usize, resource: DataStorage, streams: Option<Vec<StreamDescription>>) -> Self {
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

	pub fn id(&self) -> ResourceId {
		ResourceId::new(&self.id)
	}

	pub fn class(&self) -> &str {
		&self.class
	}
}