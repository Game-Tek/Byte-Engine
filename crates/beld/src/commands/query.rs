use resource_management::resource::{
	ReDBStorageBackend, ReadStorageBackend,
	storage_backend::{Query, QueryCursor, QueryError},
};

#[cfg(debug_assertions)]
use crate::commands::shared::{insert_trace_json, print_human_trace, read_resource_trace};
use crate::{
	OutputFormat,
	commands::shared::{
		decode_query_cursor, encode_query_cursor, open_read_only_storage, print_queryable_value, queryable_properties_json,
	},
};

/// Finds resources by class and indexed property values.
///
/// Pass a returned ID to [`crate::inspect`] when you need the full resource metadata.
pub async fn query(
	destination_path: String,
	class: String,
	properties: Vec<String>,
	limit: Option<usize>,
	cursor: Option<String>,
	format: OutputFormat,
) -> Result<(), i32> {
	let storage_backend = open_read_only_storage(destination_path, "query").await?;
	let mut query = Query::new(&class);

	if let Some(limit) = limit {
		query = query.limit(limit);
	}

	for property in properties {
		let (name, value) = parse_query_property(&property)?;
		query = query.eq(name, value);
	}

	if let Some(cursor) = cursor {
		query = query.cursor(decode_query_cursor(&cursor)?);
	}

	let page = storage_backend.query(query).await.map_err(|error| {
		log::error!("{}", query_error_message(error));
		1
	})?;

	match format {
		OutputFormat::Human => print_human_query_page(&storage_backend, &page.items, page.cursor.as_ref()).await?,
		OutputFormat::JSON => print_json_query_page(&storage_backend, &page.items, page.cursor.as_ref()).await?,
	}

	Ok(())
}

/// Parses one `name=value` property filter from the command line.
pub(super) fn parse_query_property(property: &str) -> Result<(&str, &str), i32> {
	let Some((name, value)) = property.split_once('=') else {
		log::error!(
			"Invalid query property '{}'. The most likely cause is that the filter is not in `property=value` form.",
			property
		);
		return Err(1);
	};

	if name.is_empty() || value.is_empty() {
		log::error!(
			"Invalid query property '{}'. The most likely cause is an empty property name or value.",
			property
		);
		return Err(1);
	}

	Ok((name, value))
}

/// Returns a concise command-line message for a storage query error.
pub(super) fn query_error_message(error: QueryError) -> &'static str {
	match error {
		QueryError::InvalidCursor => "Failed to query resources. The most likely cause is that the provided cursor is invalid.",
		QueryError::StorageFailure => {
			"Failed to query resources. The most likely cause is that the resources database could not be read."
		}
	}
}

/// Prints query results in a compact human-readable form.
async fn print_human_query_page(
	storage_backend: &ReDBStorageBackend,
	items: &[(
		resource_management::SerializableResource,
		resource_management::resource::resource_handler::MultiResourceReader,
	)],
	cursor: Option<&QueryCursor>,
) -> Result<(), i32> {
	if items.is_empty() {
		log::info!("No resources found.");
	}

	for (resource, _) in items {
		println!("{}", resource.id());
		for property in resource.queryable_properties() {
			print!("  {}: ", property.name);
			print_queryable_value(&property.value);
			println!();
		}
		#[cfg(debug_assertions)]
		print_human_trace(&read_resource_trace(storage_backend, resource.id()).await?, 2);
	}

	if let Some(cursor) = cursor {
		println!("cursor: {}", encode_query_cursor(cursor));
	}

	Ok(())
}

/// Prints query results as JSON for scripts and editor integrations.
async fn print_json_query_page(
	storage_backend: &ReDBStorageBackend,
	items: &[(
		resource_management::SerializableResource,
		resource_management::resource::resource_handler::MultiResourceReader,
	)],
	cursor: Option<&QueryCursor>,
) -> Result<(), i32> {
	let mut resources = Vec::with_capacity(items.len());
	for (resource, _) in items {
		let mut value = serde_json::json!({
			"id": resource.id(),
			"uid": resource.uid(),
			"class": resource.class(),
			"properties": queryable_properties_json(resource.queryable_properties()),
		});
		#[cfg(debug_assertions)]
		insert_trace_json(&mut value, &read_resource_trace(storage_backend, resource.id()).await?);
		resources.push(value);
	}

	let output = serde_json::json!({
		"resources": resources,
		"cursor": cursor.map(encode_query_cursor),
	});

	println!(
		"{}",
		serde_json::to_string_pretty(&output).map_err(|error| {
			log::error!(
				"Failed to print query JSON. The most likely cause is an invalid JSON value. Error: {}",
				error
			);
			1
		})?
	);

	Ok(())
}
