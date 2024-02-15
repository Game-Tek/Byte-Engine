use smol::{fs::File, future::FutureExt};

use crate::{CreateInfo, CreateResource, ProcessedResources, Resource, Stream};

use super::{resource_manager,};

pub trait ResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool;

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>)>;

	fn read<'a>(&'a self, _resource: &'a dyn Resource, file: &'a mut File, streams: &'a mut [Stream<'a>]) -> utils::BoxedFuture<()>;
}