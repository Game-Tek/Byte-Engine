//! Turn the catalog, a filter, a group field, and a sort into the groups and rows that `browse` and `query` show, in order.

use std::{borrow::Cow, cmp::Ordering, collections::HashMap};

use super::{
	catalog::{Catalog, Field, FieldKind, Value, ratio},
	filter::Filter,
};

/// The `Sort` struct picks the field rows are ordered by, and whether they run against the field's usual direction.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Sort {
	pub(super) field: Field,
	pub(super) reversed: bool,
}

impl Sort {
	/// Returns whether rows run largest first: sizes, ratios, and numbers do, unless reversed.
	pub(super) fn descending(&self) -> bool {
		descending(self.field.kind(), self.reversed)
	}
}

/// The `Group` struct is one group of matching rows and their totals.
#[derive(Debug, PartialEq)]
pub(super) struct Group {
	/// The group's value as text, `(none)` for rows without it, or `None` when the view isn't grouped.
	pub(super) name: Option<String>,
	/// The group's rows, in sort order.
	pub(super) rows: Vec<usize>,
	pub(super) size: u64,
	pub(super) stored: u64,
}

/// The group name of rows that don't have the group field.
pub(super) const NO_VALUE_GROUP: &str = "(none)";

/// One group while the view builds it: its name, its value, and its rows.
type Bucket<'a> = (Cow<'a, str>, Option<Value<'a>>, Vec<usize>);

/// Returns the rows that match `filter`, grouped by `group` and ordered by `sort`.
///
/// Without a group field, the result is one unnamed group. Model fields the filter, group, or sort use are extracted
/// into the catalog first.
pub(super) fn view(catalog: &mut Catalog, filter: &Filter, group: Option<&Field>, sort: &Sort) -> Vec<Group> {
	catalog.prepare(filter.fields().chain(group).chain([&sort.field]));
	let catalog = &*catalog;
	let matching = (0..catalog.rows.len()).filter(|&row| filter.matches(catalog, row));

	let mut groups = match group {
		None => vec![(None, matching.collect::<Vec<_>>())],
		Some(field) => {
			// Names borrow the catalog's text, so only number values allocate one.
			let mut groups: HashMap<Cow<'_, str>, (Option<Value<'_>>, Vec<usize>)> = HashMap::new();
			for row in matching {
				let value = catalog.value(row, field);
				let name = value.clone().map_or(Cow::Borrowed(NO_VALUE_GROUP), Value::into_text);
				groups.entry(name).or_insert_with(|| (value, Vec::new())).1.push(row);
			}
			let mut groups = groups
				.into_iter()
				.map(|(name, (value, rows))| (name, value, rows))
				.collect::<Vec<Bucket>>();
			sort_groups(catalog, &mut groups, field, sort);
			groups
				.into_iter()
				.map(|(name, _, rows)| (Some(name.into_owned()), rows))
				.collect()
		}
	};

	let descending = sort.descending();
	for (_, rows) in &mut groups {
		// Each row's value is read once, so a field that formats its value, such as `hash`, formats it once per row.
		let mut keyed = rows
			.iter()
			.map(|&row| (catalog.value(row, &sort.field), row))
			.collect::<Vec<_>>();
		keyed.sort_by(|(left_value, left), (right_value, right)| {
			compare_values(left_value.as_ref(), right_value.as_ref(), descending)
				.then_with(|| catalog.rows[*left].id.cmp(&catalog.rows[*right].id))
		});
		rows.clear();
		rows.extend(keyed.into_iter().map(|(_, row)| row));
	}

	groups
		.into_iter()
		.map(|(name, rows)| {
			let (size, stored) = rows
				.iter()
				.filter_map(|&row| catalog.rows[row].resource.as_ref())
				.fold((0, 0), |(size, stored), resource| {
					(size + resource.size() as u64, stored + resource.stored_size() as u64)
				});
			Group {
				name,
				rows,
				size,
				stored,
			}
		})
		.collect()
}

/// Returns whether a field of `kind` runs largest first: sizes, ratios, and numbers do, unless reversed.
fn descending(kind: FieldKind, reversed: bool) -> bool {
	matches!(kind, FieldKind::Size | FieldKind::Ratio | FieldKind::Number) != reversed
}

/// Orders groups by their summed sort field when it's a size, by their ratio for a ratio, and by their value otherwise.
///
/// Group values run in the group field's usual direction, which `r` reverses along with the rows.
fn sort_groups(catalog: &Catalog, groups: &mut Vec<Bucket<'_>>, group: &Field, sort: &Sort) {
	let values_descending = descending(group.kind(), sort.reversed);
	let sum = |rows: &[usize], field: &Field| -> f64 {
		rows.iter()
			.filter_map(|&row| catalog.value(row, field).and_then(|value| value.as_number()))
			.sum()
	};
	let aggregate = |rows: &[usize]| -> Option<f64> {
		match sort.field.kind() {
			FieldKind::Size => Some(sum(rows, &sort.field)),
			FieldKind::Ratio => Some(ratio(sum(rows, &Field::Stored) as u64, sum(rows, &Field::Size) as u64)),
			_ => None,
		}
	};

	let mut keyed = groups.drain(..).map(|group| (aggregate(&group.2), group)).collect::<Vec<_>>();
	keyed.sort_by(|(left_key, left), (right_key, right)| {
		let by_value =
			|| compare_values(left.1.as_ref(), right.1.as_ref(), values_descending).then_with(|| left.0.cmp(&right.0));
		match (left_key, right_key) {
			(Some(left_key), Some(right_key)) => {
				let ordering = left_key.total_cmp(right_key);
				if sort.descending() { ordering.reverse() } else { ordering }.then_with(by_value)
			}
			_ => by_value(),
		}
	});
	groups.extend(keyed.into_iter().map(|(_, group)| group));
}

/// Compares two values in the given direction; a missing value comes last either way.
///
/// Numbers come before text when a model field mixes them.
fn compare_values(left: Option<&Value<'_>>, right: Option<&Value<'_>>, descending: bool) -> Ordering {
	let ordering = match (left, right) {
		(None, None) => return Ordering::Equal,
		(None, Some(_)) => return Ordering::Greater,
		(Some(_), None) => return Ordering::Less,
		(Some(Value::Number(left)), Some(Value::Number(right))) => left.total_cmp(right),
		(Some(Value::Text(left)), Some(Value::Text(right))) => left.cmp(right),
		(Some(Value::Number(_)), Some(Value::Text(_))) => return Ordering::Less,
		(Some(Value::Text(_)), Some(Value::Number(_))) => return Ordering::Greater,
	};
	if descending { ordering.reverse() } else { ordering }
}
