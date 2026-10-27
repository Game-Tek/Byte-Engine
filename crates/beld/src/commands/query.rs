//! Print the resources that match a filter, grouped and sorted, for scripts and agents.
//!
//! The behavior is specified in the BELD command-line reference, under Query.

#[cfg(test)]
mod tests;

use std::{borrow::Cow, fmt::Write as _};

use serde_json::{Map, Value as Json};

#[cfg(debug_assertions)]
use super::shared::resource_trace_json;
use super::{
	catalog::{self, Catalog, Field, FieldKind, SHOWN_FIELDS, Value, share},
	filter::Filter,
	shared::{log_failure, plural, queryable_properties_json, write_json},
	view::{Group, Sort, view},
};
use crate::OutputFormat;

/// Prints the resources in the store at `destination_path` that match every term, grouped by `group` and ordered by
/// `sort`, which defaults to `stored`.
///
/// Statuses compare each resource with the sources at `source_path`. Pass a printed ID to [`crate::inspect`] to read
/// that resource's model and streams.
#[allow(clippy::too_many_arguments)]
pub async fn query(
	source_path: String,
	destination_path: String,
	terms: Vec<String>,
	group: Option<String>,
	sort: Option<String>,
	reverse: bool,
	limit: Option<usize>,
	format: OutputFormat,
) -> Result<(), i32> {
	let mut catalog = catalog::load(&source_path, &destination_path).await.map_err(log_failure)?;
	let (filter, group, sort) =
		parse_selection(&catalog, &terms, group.as_deref(), sort.as_deref(), reverse).map_err(|error| {
			log::error!("{error}");
			2
		})?;
	write_query(
		&mut std::io::stdout().lock(),
		&mut catalog,
		&filter,
		group.as_ref(),
		&sort,
		limit,
		format,
	)
}

/// Parses the filter terms and the group and sort field names against the store's declared properties.
fn parse_selection(
	catalog: &Catalog,
	terms: &[String],
	group: Option<&str>,
	sort: Option<&str>,
	reverse: bool,
) -> Result<(Filter, Option<Field>, Sort), String> {
	let filter = Filter::parse(&terms.join(" "), &catalog.properties)?;
	let group = group.map(|name| Field::parse(name, &catalog.properties)).transpose()?;
	let field = sort.map_or(Ok(Field::Stored), |name| Field::parse(name, &catalog.properties))?;
	Ok((
		filter,
		group,
		Sort {
			field,
			reversed: reverse,
		},
	))
}

/// Writes the matching resources in `format`, listing at most `limit` resources in each group.
fn write_query(
	output: &mut impl std::io::Write,
	catalog: &mut Catalog,
	filter: &Filter,
	group: Option<&Field>,
	sort: &Sort,
	limit: Option<usize>,
	format: OutputFormat,
) -> Result<(), i32> {
	let groups = view(catalog, filter, group, sort);
	// The fields the command names, once each and in order: the output adds those it doesn't always show.
	let mut named = Vec::<Field>::new();
	for field in filter.fields().chain(group).chain([&sort.field]) {
		if !named.contains(field) {
			named.push(field.clone());
		}
	}

	let limit = limit.unwrap_or(usize::MAX);
	match format {
		OutputFormat::JSON => write_json(output, &query_json(catalog, &groups, group.is_some(), &named, limit)),
		OutputFormat::Human => {
			let table = query_table(catalog, &groups, group.is_some(), &named, limit);
			output.write_all(table.as_bytes()).map_err(|error| {
				log::error!(
					"Failed to write the query output. The most likely cause is that standard output was closed, for example by a pipe that exited early. Error: {error}"
				);
				1
			})
		}
	}
}

/// Builds the JSON output: the totals of every match, then its resources or its groups.
fn query_json(catalog: &Catalog, groups: &[Group], grouped: bool, named: &[Field], limit: usize) -> Json {
	let store_stored = catalog.stored_total();
	let resources = |group: &Group| {
		Json::Array(
			group
				.rows
				.iter()
				.take(limit)
				.map(|&row| resource_json(catalog, row, named))
				.collect(),
		)
	};

	let (count, size, stored) = totals(groups);
	let mut output = totals_json(count, size, stored, store_stored);
	if grouped {
		let groups = groups
			.iter()
			.map(|group| {
				let mut object = totals_json(group.rows.len(), group.size, group.stored, store_stored);
				object.insert("name".into(), Json::from(group.name.clone()));
				object.insert("resources".into(), resources(group));
				Json::Object(object)
			})
			.collect();
		output.insert("groups".into(), Json::Array(groups));
	} else {
		// An ungrouped view is one unnamed group.
		output.insert("resources".into(), groups.first().map_or(Json::Array(Vec::new()), resources));
	}
	Json::Object(output)
}

/// Returns one resource's fields, UID, properties, and trace, as the reference specifies for a JSON resource.
fn resource_json(catalog: &Catalog, row: usize, named: &[Field]) -> Json {
	let entry = &catalog.rows[row];
	let mut object = Map::new();
	let model_fields = named.iter().filter(|field| matches!(field, Field::Model(_))).cloned();
	for field in Field::fixed().chain(model_fields) {
		if let Some(value) = catalog.value(row, &field) {
			object.insert(field.name().into_owned(), value_json(&value));
		}
	}
	if let Some(resource) = &entry.resource {
		object.insert("uid".into(), Json::from(resource.uid()));
		object.insert(
			"properties".into(),
			queryable_properties_json(resource.queryable_properties()),
		);
	}
	#[cfg(debug_assertions)]
	object.insert("trace".into(), resource_trace_json(&entry.trace));
	Json::Object(object)
}

/// Converts a field value to JSON, writing whole numbers as integers so sizes and counts print without a fraction.
fn value_json(value: &Value<'_>) -> Json {
	match value {
		Value::Text(text) => Json::from(text.as_ref()),
		// Integers up to 2^53 convert exactly.
		Value::Number(number) if number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 => Json::from(*number as i64),
		Value::Number(number) => Json::from(*number),
	}
}

fn totals_json(count: usize, size: u64, stored: u64, store_stored: u64) -> Map<String, Json> {
	let mut object = Map::new();
	object.insert("count".into(), Json::from(count));
	object.insert("size".into(), Json::from(size));
	object.insert("stored".into(), Json::from(stored));
	object.insert("share".into(), Json::from(share(stored, store_stored)));
	object
}

/// Returns the number of matching rows and the summed decoded and stored sizes of their resources.
fn totals(groups: &[Group]) -> (usize, u64, u64) {
	groups.iter().fold((0, 0, 0), |(count, size, stored), group| {
		(count + group.rows.len(), size + group.size, stored + group.stored)
	})
}

/// Lays the matching resources out as a text table with a header line, group lines, and a totals line.
fn query_table(catalog: &Catalog, groups: &[Group], grouped: bool, named: &[Field], limit: usize) -> String {
	let columns = SHOWN_FIELDS
		.iter()
		.chain(named.iter().filter(|field| !SHOWN_FIELDS.contains(field)))
		.collect::<Vec<_>>();
	let indent = if grouped { "  " } else { "" };

	// Every cell first, so each column can be as wide as its widest value.
	let listed = groups
		.iter()
		.map(|group| {
			group
				.rows
				.iter()
				.take(limit)
				.map(|&row| columns.iter().map(|field| catalog.cell(row, field)).collect::<Vec<_>>())
				.collect::<Vec<_>>()
		})
		.collect::<Vec<_>>();
	let header = columns.iter().map(|field| field.name()).collect::<Vec<_>>();
	let widths = (0..columns.len())
		.map(|column| {
			std::iter::once(&header)
				.chain(listed.iter().flatten())
				.map(|cells| cells[column].chars().count())
				.max()
				.unwrap_or(0)
		})
		.collect::<Vec<_>>();

	let mut table = String::new();
	let line = |table: &mut String, cells: &[Cow<'_, str>]| {
		table.push_str(indent);
		for (column, cell) in cells.iter().enumerate() {
			// Numbers line up on their last digit.
			let numeric = matches!(
				columns[column].kind(),
				FieldKind::Size | FieldKind::Ratio | FieldKind::Number | FieldKind::Age
			);
			let width = widths[column];
			let _ = if numeric {
				write!(table, "{cell:>width$}  ")
			} else {
				write!(table, "{cell:<width$}  ")
			};
		}
		table.truncate(table.trim_end().len());
		table.push('\n');
	};

	let store_stored = catalog.stored_total();
	let summary = |count: usize, size: u64, stored: u64| {
		format!(
			"{} · {} stored · {} decoded · {:.0}% of the store",
			plural(count, "resource"),
			utils::ByteSize(stored),
			utils::ByteSize(size),
			share(stored, store_stored) * 100.0
		)
	};

	line(&mut table, &header);
	for (group, rows) in groups.iter().zip(&listed) {
		if let Some(name) = &group.name {
			let _ = writeln!(table, "{name} · {}", summary(group.rows.len(), group.size, group.stored));
		}
		for cells in rows {
			line(&mut table, cells);
		}
		let left_out = group.rows.len() - rows.len();
		if left_out > 0 {
			let _ = writeln!(table, "{indent}… {}", plural(left_out, "more resource"));
		}
	}
	let (count, size, stored) = totals(groups);
	let _ = writeln!(table, "{}", summary(count, size, stored));
	table
}
