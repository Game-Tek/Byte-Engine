//! Read a resource store's metadata once into rows that `browse` and `query` filter, group, and sort.

use std::{
	borrow::Cow,
	collections::{BTreeSet, HashMap, HashSet},
};

#[cfg(debug_assertions)]
use resource_management::ResourceTraceItem;
use resource_management::{
	QueryableValue, SerializableResource,
	asset::{FileStorageBackend, manager::stale_sources},
	resource::{ReadStorageBackend as _, ResourceId as ResourceUid, StorageUsage},
};
use serde_json::Value as Json;

use crate::commands::{
	filter::{glob_matches, glob_pattern},
	shared::open_read_only_storage,
};

#[cfg(test)]
pub(super) mod tests;

/// The `Status` enum says whether a row's resource is what a bake would produce now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
	Current,
	Stale,
	Unknown,
	Failed,
}

impl Status {
	pub(super) fn as_str(self) -> &'static str {
		match self {
			Status::Current => "current",
			Status::Stale => "stale",
			Status::Unknown => "unknown",
			Status::Failed => "failed",
		}
	}
}

/// The `Row` struct holds one resource, or one failed bake, as `browse` and `query` show it.
pub(super) struct Row {
	pub(super) id: String,
	/// The resource's metadata, or `None` for a failed bake, which stored no resource.
	pub(super) resource: Option<SerializableResource>,
	pub(super) status: Status,
	/// The recorded source files that changed or no longer exist.
	pub(super) stale_sources: Vec<String>,
	/// The messages of the latest bake.
	#[cfg(debug_assertions)]
	pub(super) trace: Vec<ResourceTraceItem>,
	/// The IDs the resource's model names. Release builds record none.
	pub(super) references: Vec<String>,
	/// The rows whose resources reference this one.
	pub(super) used_by: Vec<usize>,
	/// Whether the ID matches a configured root pattern.
	pub(super) root: bool,
	/// Whether the row is a root or a reachable row references it.
	pub(super) reachable: bool,
	/// The stored size of this resource and of everything it references, each counted once.
	pub(super) total: u64,
	/// Whether another resource has the same non-empty payload.
	pub(super) duplicate: bool,
}

/// The `Value` enum is one field's value for one row, borrowed from the catalog where possible.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value<'a> {
	Text(Cow<'a, str>),
	Number(f64),
}

impl<'a> Value<'a> {
	/// Returns the value as the text that grouping and `~` filters see.
	pub(super) fn to_text(&self) -> Cow<'_, str> {
		match self {
			Value::Text(text) => Cow::Borrowed(text),
			Value::Number(number) => Cow::Owned(number.to_string()),
		}
	}

	/// Turns the value into its text, keeping text borrowed from the catalog borrowed.
	pub(super) fn into_text(self) -> Cow<'a, str> {
		match self {
			Value::Text(text) => text,
			Value::Number(number) => Cow::Owned(number.to_string()),
		}
	}

	/// Borrows the value, so reading a cached value doesn't copy its text.
	fn borrowed(&self) -> Value<'_> {
		match self {
			Value::Text(text) => Value::Text(Cow::Borrowed(text)),
			Value::Number(number) => Value::Number(*number),
		}
	}

	pub(super) fn as_number(&self) -> Option<f64> {
		match self {
			Value::Number(number) => Some(*number),
			Value::Text(_) => None,
		}
	}
}

/// The `FieldKind` enum decides how a field's filter values parse, how its values print, and which way a newly
/// picked sort runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FieldKind {
	Text,
	Number,
	Size,
	/// Stored bytes per decoded byte, which groups compute from their summed sizes.
	Ratio,
	Age,
	/// A model field, whose values may be numbers or text.
	Model,
}

/// The `Field` enum names one value that `browse` and `query` filter, group, and sort by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Field {
	Id,
	Class,
	Source,
	Encoding,
	Hash,
	Status,
	Size,
	Stored,
	Ratio,
	Metadata,
	Baked,
	Refs,
	UsedBy,
	Total,
	/// A declared queryable property.
	Property(String),
	/// A path into the model JSON that `beld inspect` prints, without the `model.` prefix.
	Model(String),
}

/// The fields with fixed names, in the order pickers list them.
const NAMED_FIELDS: [(&str, Field); 14] = [
	("id", Field::Id),
	("class", Field::Class),
	("source", Field::Source),
	("encoding", Field::Encoding),
	("hash", Field::Hash),
	("status", Field::Status),
	("size", Field::Size),
	("stored", Field::Stored),
	("ratio", Field::Ratio),
	("metadata", Field::Metadata),
	("baked", Field::Baked),
	("refs", Field::Refs),
	("used-by", Field::UsedBy),
	("total", Field::Total),
];

/// The columns every resource table starts with. `browse` and `query` add the fields a command names after them.
///
/// It's a `static` so tables can borrow column names from it.
pub(super) static SHOWN_FIELDS: [Field; 5] = [Field::Id, Field::Class, Field::Stored, Field::Encoding, Field::Ratio];

impl Field {
	/// Resolves a field name: fixed names first, then `model.` paths, then the store's declared properties.
	pub(super) fn parse(name: &str, properties: &BTreeSet<String>) -> Result<Field, String> {
		let field = if let Some((_, field)) = NAMED_FIELDS.iter().find(|(candidate, _)| *candidate == name) {
			field.clone()
		} else if let Some(path) = name.strip_prefix("model.") {
			if path.is_empty() || path.split('.').any(str::is_empty) {
				return Err(format!(
					"Invalid field '{name}'. The most likely cause is an empty step in the model path. Write the path as keys and indexes separated by single dots, such as `model.extent.width`."
				));
			}
			Field::Model(path.to_string())
		} else if properties.contains(name) {
			Field::Property(name.to_string())
		} else {
			return Err(format!(
				"Unknown field '{name}'. The most likely cause is a misspelled field or a property no resource in this store declares. Use a field from the BELD field list, or `model.<path>` for a model field. See {}.",
				utils::online_docs_url("api/beld#fields")
			));
		};

		if field.debug_only() && !cfg!(debug_assertions) {
			return Err(format!(
				"The field '{name}' needs a debug build of BELD. The most likely cause is that this BELD is a release build, which reads no references. Run a debug build of BELD to use it."
			));
		}
		Ok(field)
	}

	/// Returns the fields with fixed names that this build reads, in the order pickers list them.
	pub(super) fn fixed() -> impl Iterator<Item = Field> {
		NAMED_FIELDS
			.iter()
			.map(|(_, field)| field.clone())
			.filter(|field| cfg!(debug_assertions) || !field.debug_only())
	}

	/// Returns every field the pickers offer: the fixed names, then the declared properties.
	pub(super) fn all(properties: &BTreeSet<String>) -> Vec<Field> {
		Field::fixed()
			.chain(
				properties
					.iter()
					.filter(|property| !NAMED_FIELDS.iter().any(|(name, _)| name == property))
					.map(|property| Field::Property(property.clone())),
			)
			.collect()
	}

	pub(super) fn name(&self) -> Cow<'_, str> {
		match self {
			Field::Property(name) => Cow::Borrowed(name),
			Field::Model(path) => Cow::Owned(format!("model.{path}")),
			field => Cow::Borrowed(
				NAMED_FIELDS
					.iter()
					.find(|(_, candidate)| candidate == field)
					.map_or("", |(name, _)| name),
			),
		}
	}

	pub(super) fn kind(&self) -> FieldKind {
		match self {
			Field::Id | Field::Class | Field::Source | Field::Encoding | Field::Hash | Field::Status | Field::Property(_) => {
				FieldKind::Text
			}
			Field::Size | Field::Stored | Field::Metadata | Field::Total => FieldKind::Size,
			Field::Ratio => FieldKind::Ratio,
			Field::Refs | Field::UsedBy => FieldKind::Number,
			Field::Baked => FieldKind::Age,
			Field::Model(_) => FieldKind::Model,
		}
	}

	/// Returns whether the field reads references, which only debug builds record.
	fn debug_only(&self) -> bool {
		matches!(self, Field::Refs | Field::UsedBy | Field::Total)
	}
}

/// The `Catalog` struct holds everything `browse` and `query` show, read from the store in one pass.
pub(super) struct Catalog {
	pub(super) rows: Vec<Row>,
	/// The row of each ID, for following references.
	index: HashMap<String, usize>,
	/// Every declared property name in the store.
	pub(super) properties: BTreeSet<String>,
	pub(super) usage: StorageUsage,
	/// Whether the source directory could be read, so statuses mean something.
	pub(super) sources_checked: bool,
	/// When the store was read, in nanoseconds since the Unix epoch. Ages count from it.
	pub(super) read_at: u64,
	/// The summed stored size of every resource, which shares of the store divide by.
	stored_total: u64,
	/// Model values extracted on first use, one per row, by path.
	model_columns: HashMap<String, Vec<Option<Value<'static>>>>,
	/// Whether the configuration names any root pattern, so reachability means something.
	pub(super) has_roots: bool,
}

impl Catalog {
	/// Builds the rows and their cross-references from what was read from the store.
	///
	/// `stale` holds each resource's changed sources in the order of `resources`, or is `None` when the source
	/// directory couldn't be read. `references` holds the debug records by ID; release builds pass none. `roots` are
	/// the configured `resources.roots` glob patterns. In debug builds, call [`Self::with_traces`] next.
	pub(super) fn new(
		resources: Vec<SerializableResource>,
		stale: Option<Vec<Vec<String>>>,
		references: Vec<(String, Vec<String>)>,
		roots: &[String],
		usage: StorageUsage,
		read_at: u64,
	) -> Catalog {
		let sources_checked = stale.is_some();
		let mut stale = stale.map(Vec::into_iter);
		let mut references = references.into_iter().collect::<HashMap<_, _>>();
		let mut properties = BTreeSet::new();
		let mut stored_total = 0;
		// Root patterns match like filter globs: the whole ID, ignoring case.
		let roots = roots.iter().map(|pattern| glob_pattern(pattern)).collect::<Vec<_>>();

		let mut rows = Vec::with_capacity(resources.len());
		for resource in resources {
			let id = resource.id().to_string();
			let stale_sources = stale.as_mut().and_then(Iterator::next).unwrap_or_default();
			let status = match (sources_checked, stale_sources.is_empty()) {
				(false, _) => Status::Unknown,
				(true, true) => Status::Current,
				(true, false) => Status::Stale,
			};
			for property in resource.queryable_properties() {
				// Most resources declare the same names, so only the first of each is copied.
				if !properties.contains(&property.name) {
					properties.insert(property.name.clone());
				}
			}
			stored_total += resource.stored_size() as u64;
			rows.push(Row {
				#[cfg(debug_assertions)]
				trace: Vec::new(),
				references: references.remove(&id).unwrap_or_default(),
				root: roots.iter().any(|pattern| glob_matches(pattern, &id)),
				id,
				resource: Some(resource),
				status,
				stale_sources,
				used_by: Vec::new(),
				reachable: false,
				total: 0,
				duplicate: false,
			});
		}
		let index = rows
			.iter()
			.enumerate()
			.map(|(row, entry)| (entry.id.clone(), row))
			.collect::<HashMap<_, _>>();
		let edges = reference_edges(&rows, &index);
		link_references(&mut rows, &edges);
		mark_reachable(&mut rows, &edges);
		mark_duplicates(&mut rows);

		Catalog {
			rows,
			index,
			properties,
			usage,
			sources_checked,
			read_at,
			stored_total,
			model_columns: HashMap::new(),
			has_roots: !roots.is_empty(),
		}
	}

	/// Gives each resource the trace of its latest bake, and adds a failed row for each trace without a resource.
	#[cfg(debug_assertions)]
	pub(super) fn with_traces(mut self, traces: Vec<(String, Vec<ResourceTraceItem>)>) -> Catalog {
		for (id, trace) in traces {
			match self.index.get(&id) {
				Some(&row) => self.rows[row].trace = trace,
				// A trace without a resource is a bake that stored nothing.
				None => {
					self.index.insert(id.clone(), self.rows.len());
					self.rows.push(Row {
						id,
						resource: None,
						status: Status::Failed,
						stale_sources: Vec::new(),
						trace,
						references: Vec::new(),
						used_by: Vec::new(),
						root: false,
						reachable: false,
						total: 0,
						duplicate: false,
					});
				}
			}
		}
		self
	}

	/// Returns one row's value for a field, or `None` when the row doesn't have it.
	///
	/// A model field has values only after [`Self::prepare`], which [`super::view::view`] calls.
	pub(super) fn value(&self, row: usize, field: &Field) -> Option<Value<'_>> {
		let entry = &self.rows[row];
		match field {
			Field::Id => return Some(Value::Text(Cow::Borrowed(&entry.id))),
			Field::Source => return Some(Value::Text(Cow::Borrowed(source_of(&entry.id)))),
			Field::Status => return Some(Value::Text(Cow::Borrowed(entry.status.as_str()))),
			Field::Model(path) => return self.model_columns.get(path)?.get(row)?.as_ref().map(Value::borrowed),
			_ => {}
		}

		let resource = entry.resource.as_ref()?;
		let number = |value: usize| Some(Value::Number(value as f64));
		match field {
			Field::Class => Some(Value::Text(Cow::Borrowed(resource.class()))),
			Field::Encoding => Some(Value::Text(Cow::Borrowed(resource.encoding().as_str()))),
			Field::Hash => Some(Value::Text(Cow::Owned(format!("{:016x}", resource.hash())))),
			Field::Size => number(resource.size()),
			Field::Stored => number(resource.stored_size()),
			Field::Ratio => Some(Value::Number(ratio(resource.stored_size() as u64, resource.size() as u64))),
			Field::Metadata => number(resource.resource().len()),
			Field::Baked => Some(Value::Number(
				self.read_at.saturating_sub(resource.baked_at()) as f64 / 1_000_000_000.0,
			)),
			Field::Refs => number(entry.references.len()),
			Field::UsedBy => number(entry.used_by.len()),
			Field::Total => Some(Value::Number(entry.total as f64)),
			Field::Property(name) => resource
				.queryable_properties()
				.iter()
				.find(|property| &property.name == name)
				.map(|property| match &property.value {
					QueryableValue::String(value) => Value::Text(Cow::Borrowed(value)),
				}),
			Field::Id | Field::Source | Field::Status | Field::Model(_) => unreachable!("handled before the resource lookup"),
		}
	}

	/// Formats one row's value for a table column, in its field's unit, showing a failed bake as `failed` in the class
	/// column.
	pub(super) fn cell(&self, row: usize, field: &Field) -> Cow<'_, str> {
		match self.value(row, field) {
			Some(Value::Text(text)) => text,
			Some(Value::Number(number)) => Cow::Owned(match field.kind() {
				FieldKind::Size => utils::ByteSize(number as u64).to_string(),
				FieldKind::Ratio => format!("{number:.2}"),
				FieldKind::Age => age(number),
				FieldKind::Text | FieldKind::Number | FieldKind::Model => number.to_string(),
			}),
			None if *field == Field::Class && self.rows[row].resource.is_none() => Cow::Borrowed("failed"),
			None => Cow::Borrowed(""),
		}
	}

	/// Extracts the model fields among `fields` for every row, the first time a filter, group, or sort uses them.
	///
	/// Each row's model is decoded once for all the new paths, and values are cached by path, so the catalog never
	/// holds every model's JSON at once.
	pub(super) fn prepare<'a>(&mut self, fields: impl IntoIterator<Item = &'a Field>) {
		let mut paths = Vec::<&str>::new();
		for field in fields {
			if let Field::Model(path) = field
				&& !self.model_columns.contains_key(path)
				&& !paths.contains(&path.as_str())
			{
				paths.push(path);
			}
		}
		if paths.is_empty() {
			return;
		}

		let mut columns = paths.iter().map(|_| Vec::with_capacity(self.rows.len())).collect::<Vec<_>>();
		for row in &self.rows {
			let model = row
				.resource
				.as_ref()
				.and_then(|resource| resource_management::inspect::inspect_model(resource).ok());
			for (column, path) in columns.iter_mut().zip(&paths) {
				column.push(model.as_ref().and_then(|model| model_value(model, path)));
			}
		}
		for (path, column) in paths.into_iter().zip(columns) {
			self.model_columns.insert(path.to_string(), column);
		}
	}

	/// Returns the summed stored size of every resource in the store, which shares of the store divide by.
	pub(super) fn stored_total(&self) -> u64 {
		self.stored_total
	}

	/// Returns the row a reference names, or `None` when the store has no resource with that ID.
	pub(super) fn row_of(&self, id: &str) -> Option<usize> {
		self.index.get(id).copied()
	}

	/// Returns the number of duplicate groups and the stored bytes all but one resource in each group waste.
	pub(super) fn duplicate_waste(&self) -> (usize, u64) {
		// Each group keeps its smallest stored copy; the others are waste.
		let mut groups: HashMap<u64, (u64, u64)> = HashMap::new();
		for resource in self
			.rows
			.iter()
			.filter(|row| row.duplicate)
			.filter_map(|row| row.resource.as_ref())
		{
			let stored = resource.stored_size() as u64;
			let group = groups.entry(resource.hash()).or_insert((0, u64::MAX));
			group.0 += stored;
			group.1 = group.1.min(stored);
		}
		groups.values().fold((0, 0), |(count, wasted), (sum, smallest)| {
			(count + 1, wasted + sum - smallest)
		})
	}

	/// Returns the number and total size of payload files whose UID, hash, and encoding match no resource.
	pub(super) fn orphaned_payloads(&self) -> (usize, u64) {
		let recorded = self
			.rows
			.iter()
			.filter_map(|row| row.resource.as_ref())
			.map(|resource| {
				(
					ResourceUid::from(resource.id()),
					resource.hash(),
					resource.encoding().as_str(),
				)
			})
			.collect::<HashSet<_>>();
		self.usage
			.payload_files
			.iter()
			.filter(|file| !recorded.contains(&(file.uid, file.hash, file.encoding.as_str())))
			.fold((0, 0), |(count, size), file| (count + 1, size + file.size))
	}
}

/// Reads the store at `destination_path` and checks every resource against the sources at `source_path`.
///
/// Returns the error message instead of logging it, so `browse` can show a failed reload in place.
///
/// `roots` are the configured `resources.roots` glob patterns, which decide what is reachable.
pub(super) async fn load(source_path: &str, destination_path: &str, roots: &[String]) -> Result<Catalog, String> {
	let storage = open_read_only_storage(destination_path.to_string(), "read").await?;
	let resources = storage.list().await?;
	let usage = storage.usage()?;
	#[cfg(debug_assertions)]
	let (traces, references) = (storage.traces()?, storage.references()?);
	#[cfg(not(debug_assertions))]
	let references = Vec::new();
	drop(storage);

	let read_at = resource_management::unix_time_nanos();
	let stale = check_sources(source_path, &resources).await;

	let catalog = Catalog::new(resources, stale, references, roots, usage, read_at);
	#[cfg(debug_assertions)]
	let catalog = catalog.with_traces(traces);
	Ok(catalog)
}

/// Returns each resource's changed sources, or `None` when `source_path` isn't a readable directory.
///
/// The source directory is only read, never created, unlike a bake.
async fn check_sources(source_path: &str, resources: &[SerializableResource]) -> Option<Vec<Vec<String>>> {
	use utils::r#async::StreamExt as _;

	let source_path = std::path::PathBuf::from(source_path);
	if !source_path.is_dir() {
		return None;
	}
	let sources = FileStorageBackend::new(source_path);
	let sources = &sources;

	// Resources are independent, so their version reads overlap.
	let checks = resources.iter().map(|resource| async move {
		stale_sources(resource, sources)
			.await
			.into_iter()
			.map(str::to_string)
			.collect::<Vec<_>>()
	});
	Some(utils::r#async::stream::iter(checks).buffered(16).collect().await)
}

/// Returns the rows each row references, leaving out IDs the store doesn't have.
fn reference_edges(rows: &[Row], index: &HashMap<String, usize>) -> Vec<Vec<usize>> {
	rows.iter()
		.map(|row| {
			row.references
				.iter()
				.filter_map(|id| index.get(id).copied())
				.collect::<Vec<_>>()
		})
		.collect()
}

/// Marks every root, and every row a marked row references, as reachable.
fn mark_reachable(rows: &mut [Row], edges: &[Vec<usize>]) {
	// One walk from every root at once: each row is pushed at most once, when it is first marked.
	let mut pending = Vec::new();
	for (row, entry) in rows.iter_mut().enumerate() {
		if entry.root {
			entry.reachable = true;
			pending.push(row);
		}
	}
	while let Some(row) = pending.pop() {
		for &target in &edges[row] {
			if !rows[target].reachable {
				rows[target].reachable = true;
				pending.push(target);
			}
		}
	}
}

/// Records which rows reference each row, and each row's total size through its references.
fn link_references(rows: &mut [Row], edges: &[Vec<usize>]) {
	for (row, targets) in edges.iter().enumerate() {
		for &target in targets {
			rows[target].used_by.push(row);
		}
	}

	// Each walk marks visited rows with its own number, so one buffer serves every walk without clearing.
	let mut visited = vec![usize::MAX; rows.len()];
	let mut pending = Vec::new();
	for start in 0..rows.len() {
		let mut total = 0;
		pending.push(start);
		visited[start] = start;
		while let Some(row) = pending.pop() {
			total += rows[row]
				.resource
				.as_ref()
				.map_or(0, |resource| resource.stored_size() as u64);
			for &target in &edges[row] {
				if visited[target] != start {
					visited[target] = start;
					pending.push(target);
				}
			}
		}
		rows[start].total = total;
	}
}

/// Marks every resource whose non-empty payload has the same hash as another resource's.
fn mark_duplicates(rows: &mut [Row]) {
	let mut counts: HashMap<u64, usize> = HashMap::new();
	let payload_hash = |row: &Row| {
		row.resource
			.as_ref()
			.filter(|resource| resource.size() > 0)
			.map(SerializableResource::hash)
	};
	for hash in rows.iter().filter_map(payload_hash) {
		*counts.entry(hash).or_default() += 1;
	}
	for row in rows.iter_mut() {
		row.duplicate = payload_hash(row).is_some_and(|hash| counts[&hash] > 1);
	}
}

/// Returns the ID without its fragment: the source file a resource was baked from.
fn source_of(id: &str) -> &str {
	id.split_once('#').map_or(id, |(source, _)| source)
}

/// Returns `stored` divided by `size`, or 1 for an empty payload.
pub(super) fn ratio(stored: u64, size: u64) -> f64 {
	if size == 0 { 1.0 } else { stored as f64 / size as f64 }
}

/// Returns the part of `whole` that `part` makes up, or 0 when `whole` is empty.
pub(super) fn share(part: u64, whole: u64) -> f64 {
	if whole == 0 { 0.0 } else { part as f64 / whole as f64 }
}

/// Follows a dotted path of object keys and array indexes, and returns the scalar there.
///
/// Objects and arrays become compact JSON text; null counts as no value.
fn model_value(json: &Json, path: &str) -> Option<Value<'static>> {
	let value = path.split('.').try_fold(json, |value, step| match value {
		Json::Object(object) => object.get(step),
		Json::Array(array) => array.get(step.parse::<usize>().ok()?),
		_ => None,
	})?;
	match value {
		Json::Null => None,
		Json::Number(number) => number.as_f64().map(Value::Number),
		Json::String(text) => Some(Value::Text(Cow::Owned(text.clone()))),
		Json::Bool(flag) => Some(Value::Text(Cow::Borrowed(if *flag { "true" } else { "false" }))),
		Json::Array(_) | Json::Object(_) => Some(Value::Text(Cow::Owned(value.to_string()))),
	}
}

/// Formats seconds in the largest whole unit: seconds, minutes, hours, or days.
fn age(seconds: f64) -> String {
	match seconds {
		seconds if seconds < 60.0 => format!("{seconds:.0}s"),
		seconds if seconds < 3600.0 => format!("{:.0}m", seconds / 60.0),
		seconds if seconds < 86_400.0 => format!("{:.0}h", seconds / 3600.0),
		seconds => format!("{:.0}d", seconds / 86_400.0),
	}
}
