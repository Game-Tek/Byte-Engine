/// The `QueryableValue` enum represents a property value that can be indexed by storage backends.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum QueryableValue {
	String(String),
}

/// The `QueryableProperty` struct represents a storage-visible property for resource queries.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueryableProperty {
	pub name: String,
	pub value: QueryableValue,
}

pub trait Model: for<'de> serde::Deserialize<'de> {
	fn get_class() -> &'static str;

	fn queryable_properties(&self, id: &str) -> Vec<QueryableProperty> {
		vec![QueryableProperty {
			name: "name".to_string(),
			value: QueryableValue::String(id.to_string()),
		}]
	}
}
