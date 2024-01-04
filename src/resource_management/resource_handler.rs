use std::pin::Pin;

use smol::fs::File;

use crate::utils;

use super::{resource_manager, ProcessedResources, Stream, Resource};

pub trait ResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool;

	/// Returns a tuple containing the resource description and it's associated binary data.\
	/// 
	/// The returned document is like the following:
	/// ```json
	/// { "class": "X", "resource": { ... }, "hash": 0, "required_resources":[{ "path": "..." }] }
	/// ```
	/// Fields:
	/// - **class**: The resource class. This is used to identify the resource type. Needs to be meaningful and will be a public constant.
	/// - **resource**: The resource data. Can look like anything.
	/// - **hash**(optional): The resource hash. This is used to identify the resource data. If the resource handler wants to generate a hash for the resource it can do so else the resource manager will generate a hash for it. This is because some resources can generate hashes inteligently (EJ: code generators can output same hash for different looking code if the code is semantically identical).
	/// - **required_resources**(optional): A list of resources that this resource depends on. This is used to load resources that depend on other resources.
	fn process<'a>(&'a self, resource_manager: &'a resource_manager::ResourceManager, asset_url: &'a str) -> utils::BoxedFuture<Result<Vec<ProcessedResources>, String>>;

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>)>;

	fn read<'a>(&'a self, _resource: &'a Box<dyn Resource>, file: &'a mut File, streams: &'a mut [Stream<'a>]) -> utils::BoxedFuture<()>;
}