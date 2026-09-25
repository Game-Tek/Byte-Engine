/// The `QueryableValue` enum provides storage backends with indexable resource property values.
#[derive(
	Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum QueryableValue {
	String(String),
}

impl From<String> for QueryableValue {
	fn from(value: String) -> Self {
		Self::String(value)
	}
}

impl From<&str> for QueryableValue {
	fn from(value: &str) -> Self {
		Self::String(value.to_string())
	}
}

/// The `QueryableProperty` struct provides a named, indexable value for resource queries.
#[derive(
	Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct QueryableProperty {
	pub name: String,
	pub value: QueryableValue,
}

pub trait Model: crate::ResourceArchive {
	fn get_class() -> &'static str;

	fn queryable_properties(&self, id: &str) -> Vec<QueryableProperty> {
		vec![QueryableProperty {
			name: "name".to_string(),
			value: QueryableValue::String(id.to_string()),
		}]
	}
}
