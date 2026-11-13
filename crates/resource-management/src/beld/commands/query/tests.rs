use serde_json::{Value as Json, json};

use super::{parse_selection, write_query};
use crate::beld::{
	OutputFormat,
	commands::catalog::{
		Catalog,
		tests::{catalog, image},
	},
};
use crate::types::Formats;

/// Runs a query over the fixture catalog and returns what it prints.
fn run(terms: &[&str], group: Option<&str>, sort: Option<&str>, limit: Option<usize>, format: OutputFormat) -> String {
	let mut catalog = catalog(Vec::new());
	let terms = terms.iter().map(|term| term.to_string()).collect::<Vec<_>>();
	let (filter, group, sort) = parse_selection(&catalog, &terms, group, sort, false).unwrap();
	let mut output = Vec::new();
	write_query(
		&mut output,
		&mut catalog,
		&filter,
		group.as_ref(),
		&sort,
		limit,
		format,
		false,
		None,
	)
	.unwrap();
	String::from_utf8(output).unwrap()
}

/// Prints every resource of `catalog` as a human table for a terminal with `color` and `width`.
fn run_on_terminal(mut catalog: Catalog, color: bool, width: Option<usize>) -> String {
	let (filter, group, sort) = parse_selection(&catalog, &[], None, None, false).unwrap();
	let mut output = Vec::new();
	write_query(
		&mut output,
		&mut catalog,
		&filter,
		group.as_ref(),
		&sort,
		None,
		OutputFormat::Human,
		color,
		width,
	)
	.unwrap();
	String::from_utf8(output).unwrap()
}

fn run_json(terms: &[&str], group: Option<&str>, limit: Option<usize>) -> Json {
	serde_json::from_str(&run(terms, group, None, limit, OutputFormat::JSON)).unwrap()
}

fn ids(resources: &Json) -> Vec<&str> {
	resources
		.as_array()
		.unwrap()
		.iter()
		.map(|resource| resource["id"].as_str().unwrap())
		.collect()
}

#[test]
fn json_lists_the_matching_resources_in_order_with_their_fields_and_totals() {
	let output = run_json(&["class=Image", "model.format=RGBA8"], None, None);

	assert_eq!(output["count"], 2);
	assert_eq!(output["stored"], 8192);
	// JSON text round-trips a float only to within its last digit.
	assert!(
		(output["share"].as_f64().unwrap() - 8192.0 / 17_408.0).abs() < 1e-12,
		"{output}"
	);
	// Equal stored sizes fall back to ID order.
	assert_eq!(ids(&output["resources"]), ["crate.glb#images/0", "hero.glb#images/0"]);

	let resource = &output["resources"][0];
	assert_eq!(resource["class"], "Image");
	assert_eq!(resource["stored"], 4096);
	assert_eq!(resource["ratio"], 1);
	assert_eq!(resource["status"], "current");
	assert_eq!(resource["model.format"], "RGBA8");
	assert!(resource["uid"].is_string(), "{resource}");
	assert!(resource["properties"].is_object(), "{resource}");
	// Only the model fields the command names are added.
	assert!(resource.get("model.gamma").is_none(), "{resource}");
}

#[test]
fn a_limit_lists_the_first_resources_of_each_group_but_counts_every_match() {
	let output = run_json(&[], Some("class"), Some(1));

	assert_eq!(output["count"], 4);
	let groups = output["groups"].as_array().unwrap();
	let summary = groups
		.iter()
		.map(|group| {
			(
				group["name"].as_str().unwrap(),
				group["count"].as_u64().unwrap(),
				ids(&group["resources"]),
			)
		})
		.collect::<Vec<_>>();
	assert_eq!(
		summary,
		[("Image", 3, vec!["crate.glb#images/0"]), ("Audio", 1, vec!["music.wav"])]
	);

	let totals_only = run_json(&[], Some("class"), Some(0));
	assert_eq!(totals_only["groups"][0]["stored"], 9216);
	assert_eq!(totals_only["groups"][0]["resources"], json!([]));
}

#[test]
fn the_table_shows_named_fields_group_totals_and_what_the_limit_left_out() {
	let table = run(&["-music"], Some("class"), Some("model.format"), Some(2), OutputFormat::Human);
	let lines = table.lines().collect::<Vec<_>>();

	assert!(
		lines[0]
			.split_whitespace()
			.eq(["id", "class", "stored", "encoding", "ratio", "model.format"]),
		"{table}"
	);
	assert!(lines[1].starts_with("Image · 3 resources · 9 KiB stored"), "{table}");
	// Text fields sort in ascending order, so BC7 comes before RGBA8.
	assert!(lines[2].trim_start().starts_with("hero.glb#images/1"), "{table}");
	assert!(lines[2].ends_with("BC7"), "{table}");
	assert_eq!(lines[4], "  … 1 more resource", "{table}");
	assert!(lines[5].starts_with("3 resources · 9 KiB stored"), "{table}");
}

#[test]
fn unknown_fields_in_terms_groups_and_sorts_are_rejected() {
	let catalog = catalog(Vec::new());
	let terms = |terms: &[&str]| terms.iter().map(|term| term.to_string()).collect::<Vec<_>>();

	assert!(parse_selection(&catalog, &terms(&["bogus=1"]), None, None, false).is_err());
	assert!(parse_selection(&catalog, &terms(&[]), Some("bogus"), None, false).is_err());
	assert!(parse_selection(&catalog, &terms(&[]), None, Some("model."), false).is_err());
	assert!(parse_selection(&catalog, &terms(&["stored>1MiB"]), Some("class"), Some("model.format"), true).is_ok());
}

#[test]
fn a_narrow_terminal_shortens_ids_in_the_middle_so_resource_lines_fit() {
	let full = run_on_terminal(catalog(Vec::new()), false, None);
	let full_lines = full.lines().collect::<Vec<_>>();
	// Between the header and the totals line, every line is a resource line.
	let widest = full_lines[1..full_lines.len() - 1]
		.iter()
		.map(|line| line.chars().count())
		.max()
		.unwrap();
	let width = widest - 2;
	let table = run_on_terminal(catalog(Vec::new()), false, Some(width));
	let lines = table.lines().collect::<Vec<_>>();

	// The longest ID, `crate.glb#images/0`, loses its middle and keeps both ends.
	let crate_line = lines.iter().find(|line| line.starts_with("crate.")).unwrap();
	assert!(crate_line.starts_with("crate.gl…mages/0 "), "{table}");
	for line in &lines[1..lines.len() - 1] {
		assert!(line.chars().count() <= width, "{table}");
	}
	// The totals line keeps its full text.
	assert_eq!(lines.last(), full_lines.last(), "{table}");
}

#[test]
fn the_id_column_never_gets_narrower_than_16_characters() {
	let table = run_on_terminal(catalog(Vec::new()), false, Some(1));

	for id in table.lines().skip(1).filter_map(|line| line.split_whitespace().next()) {
		if id.contains('…') {
			assert_eq!(id.chars().count(), 16, "{table}");
		}
	}
	assert!(table.contains("hero.glb…mages/0"), "{table}");
	// An ID that already fits stays whole.
	assert!(table.contains("music.wav "), "{table}");
}

#[test]
fn color_marks_the_header_and_stale_lines_and_no_color_writes_no_escape_codes() {
	let stale_catalog = || {
		Catalog::new(
			vec![
				image("fresh.png", Formats::RGBA8, 1, 4096, 4096),
				image("old.png", Formats::RGBA8, 2, 1024, 1024),
			],
			Some(vec![Vec::new(), vec!["old.png".to_string()]]),
			Vec::new(),
			&[],
			crate::resource::StorageUsage::default(),
			0,
		)
	};

	let table = run_on_terminal(stale_catalog(), true, None);
	let lines = table.lines().collect::<Vec<_>>();
	// Cyan for the header, yellow for the stale resource, and no style for the current one.
	assert!(lines[0].starts_with("\x1b[36mid"), "{table:?}");
	assert!(lines[1].starts_with("fresh.png"), "{table:?}");
	assert!(lines[2].starts_with("\x1b[33mold.png"), "{table:?}");
	assert!(lines[2].ends_with("\x1b[0m"), "{table:?}");

	let plain = run_on_terminal(stale_catalog(), false, None);
	assert!(!plain.contains('\x1b'), "{plain:?}");
}
