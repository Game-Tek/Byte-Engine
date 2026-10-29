use resource_management::{
	ProcessedAsset, SerializableResource,
	asset::ResourceId,
	resource::{ResourcePayloadEncoding, StorageUsage},
	resources::{audio::Audio, image::Image},
	types::{BitDepths, Formats, Gamma},
};

use super::{Catalog, Field};
use crate::commands::{
	filter::Filter,
	view::{Sort, view},
};

pub(in crate::commands) fn image(id: &str, format: Formats, hash: u64, size: usize, stored: usize) -> SerializableResource {
	let image = Image {
		format,
		gamma: Gamma::SRGB,
		extent: [4, 4, 1],
		mip_count: 1,
		ibl: None,
		photometry: None,
	};
	ProcessedAsset::new(ResourceId::new(id), image).into_serializable(hash, size, stored, ResourcePayloadEncoding::Raw)
}

fn audio(id: &str, hash: u64, size: usize) -> SerializableResource {
	let audio = Audio {
		bit_depth: BitDepths::Sixteen,
		channel_count: 2,
		sample_rate: 48_000,
		sample_count: 1,
	};
	ProcessedAsset::new(ResourceId::new(id), audio).into_serializable(hash, size, size, ResourcePayloadEncoding::Raw)
}

/// The usage of an empty files-mode store.
pub(in crate::commands) fn files_usage() -> StorageUsage {
	StorageUsage {
		database_size: 0,
		payload_files: Vec::new(),
		staging_files: (0, 0),
		pack: None,
	}
}

/// Two hero images, a crate image with the same payload as the first hero image, and a sound.
pub(in crate::commands) fn catalog(references: Vec<(String, Vec<String>)>) -> Catalog {
	rooted_catalog(references, &[])
}

/// The resources of [`catalog`], with `roots` as the configured root patterns.
fn rooted_catalog(references: Vec<(String, Vec<String>)>, roots: &[&str]) -> Catalog {
	let resources = vec![
		image("hero.glb#images/0", Formats::RGBA8, 1, 4096, 4096),
		image("hero.glb#images/1", Formats::BC7, 2, 4096, 1024),
		image("crate.glb#images/0", Formats::RGBA8, 1, 4096, 4096),
		audio("music.wav", 3, 8192),
	];
	let stale = Some(vec![Vec::new(); resources.len()]);
	let roots = roots.iter().map(|root| root.to_string()).collect::<Vec<_>>();
	Catalog::new(resources, stale, references, &roots, files_usage(), 0)
}

fn by_stored() -> Sort {
	Sort {
		field: Field::Stored,
		reversed: false,
	}
}

/// Returns the IDs a filter shows, sorted by stored size.
fn ids(catalog: &mut Catalog, filter: &str) -> Vec<String> {
	let filter = Filter::parse(filter, catalog).unwrap();
	view(catalog, &filter, None, &by_stored())
		.into_iter()
		.flat_map(|group| group.rows)
		.map(|row| catalog.rows[row].id.clone())
		.collect()
}

#[test]
fn filters_reject_unknown_fields_values_and_operators() {
	let catalog = catalog(Vec::new());
	for invalid in [
		"bogus=1",
		"stored>big",
		"class>Image",
		"=Image",
		"stored>",
		"is:nothing",
		"model..format=x",
	] {
		assert!(Filter::parse(invalid, &catalog).is_err(), "'{invalid}' parsed");
	}
	for valid in [
		"hero",
		"HERO*",
		"-crate",
		"stored>=1.5MiB",
		"baked<2h",
		"class~ima",
		"model.format=BC7",
		"is:stale",
	] {
		assert!(Filter::parse(valid, &catalog).is_ok(), "'{valid}' failed");
	}
}

#[test]
fn filter_terms_all_match_and_globs_cover_the_whole_id_ignoring_case() {
	let mut catalog = catalog(Vec::new());

	assert_eq!(ids(&mut catalog, "class=Image stored>=2KiB -crate"), ["hero.glb#images/0"]);
	assert_eq!(ids(&mut catalog, "HERO*"), ["hero.glb#images/0", "hero.glb#images/1"]);
	assert_eq!(ids(&mut catalog, "*/1"), ["hero.glb#images/1"]);
	assert_eq!(ids(&mut catalog, "hero"), ["hero.glb#images/0", "hero.glb#images/1"]);
	assert_eq!(ids(&mut catalog, "model.format=BC7"), ["hero.glb#images/1"]);
}

#[test]
fn sorting_by_stored_runs_largest_first_and_reversing_flips_it() {
	let mut catalog = catalog(Vec::new());
	assert_eq!(
		ids(&mut catalog, ""),
		["music.wav", "crate.glb#images/0", "hero.glb#images/0", "hero.glb#images/1"]
	);

	let reversed = Sort {
		field: Field::Stored,
		reversed: true,
	};
	let rows = view(&mut catalog, &Filter::default(), None, &reversed)
		.into_iter()
		.flat_map(|group| group.rows)
		.map(|row| catalog.rows[row].id.as_str())
		.collect::<Vec<_>>();
	assert_eq!(
		rows,
		["hero.glb#images/1", "crate.glb#images/0", "hero.glb#images/0", "music.wav"]
	);
}

#[test]
fn grouping_by_class_sums_each_group_and_orders_groups_by_the_sum() {
	let mut catalog = catalog(Vec::new());
	let groups = view(&mut catalog, &Filter::default(), Some(&Field::Class), &by_stored());

	let summary = groups
		.iter()
		.map(|group| (group.name.as_deref().unwrap(), group.rows.len(), group.size, group.stored))
		.collect::<Vec<_>>();
	assert_eq!(summary, [("Image", 3, 12_288, 9216), ("Audio", 1, 8192, 8192)]);
}

#[test]
fn identical_payloads_are_duplicates_and_all_but_the_smallest_copy_is_waste() {
	let mut catalog = catalog(Vec::new());

	assert_eq!(ids(&mut catalog, "is:duplicate"), ["crate.glb#images/0", "hero.glb#images/0"]);
	assert_eq!(catalog.duplicate_waste(), (1, 4096));
}

#[cfg(debug_assertions)]
#[test]
fn total_counts_a_resource_referenced_twice_once() {
	// The sound uses both hero images, and the second hero image uses the first.
	let references = vec![
		(
			"music.wav".to_string(),
			vec!["hero.glb#images/0".to_string(), "hero.glb#images/1".to_string()],
		),
		("hero.glb#images/1".to_string(), vec!["hero.glb#images/0".to_string()]),
	];
	let mut catalog = catalog(references);
	let total = |catalog: &Catalog, id: &str| catalog.rows[catalog.row_of(id).unwrap()].total;

	assert_eq!(total(&catalog, "music.wav"), 8192 + 4096 + 1024);
	assert_eq!(total(&catalog, "hero.glb#images/1"), 1024 + 4096);
	assert_eq!(ids(&mut catalog, "used-by>=2"), ["hero.glb#images/0"]);
	assert_eq!(ids(&mut catalog, "used-by=0"), ["music.wav", "crate.glb#images/0"]);
}

#[cfg(debug_assertions)]
#[test]
fn unreachable_resources_are_those_no_root_uses_directly_or_indirectly() {
	// The sound is the only root. It uses the second hero image, which uses the first; the crate image is unused.
	let references = vec![
		("music.wav".to_string(), vec!["hero.glb#images/1".to_string()]),
		("hero.glb#images/1".to_string(), vec!["hero.glb#images/0".to_string()]),
	];
	let mut catalog = rooted_catalog(references, &["MUSIC.*"]);

	assert_eq!(ids(&mut catalog, "is:root"), ["music.wav"]);
	assert_eq!(ids(&mut catalog, "is:unreachable"), ["crate.glb#images/0"]);
}

#[cfg(debug_assertions)]
#[test]
fn a_chain_that_no_root_uses_is_unreachable_as_a_whole() {
	// The crate image uses both hero images, but nothing uses the crate image, and only the sound is a root.
	let references = vec![(
		"crate.glb#images/0".to_string(),
		vec!["hero.glb#images/0".to_string(), "hero.glb#images/1".to_string()],
	)];
	let mut catalog = rooted_catalog(references, &["music.wav"]);

	assert_eq!(
		ids(&mut catalog, "is:unreachable"),
		["crate.glb#images/0", "hero.glb#images/0", "hero.glb#images/1"]
	);
}

#[test]
fn unreachable_without_roots_is_an_error() {
	let catalog = catalog(Vec::new());

	let error = Filter::parse("is:unreachable", &catalog).unwrap_err();
	assert!(error.contains("resources.roots"), "{error}");
}

#[cfg(debug_assertions)]
#[test]
fn dangling_resources_reference_an_id_the_store_does_not_have() {
	let references = vec![
		("music.wav".to_string(), vec!["renamed.png".to_string()]),
		("hero.glb#images/1".to_string(), vec!["hero.glb#images/0".to_string()]),
	];
	let mut catalog = catalog(references);

	assert_eq!(ids(&mut catalog, "is:dangling"), ["music.wav"]);
}

#[cfg(debug_assertions)]
#[test]
fn a_trace_without_a_resource_is_a_failed_row() {
	use resource_management::{ResourceTraceItem, ResourceTraceLevel};

	let trace = vec![ResourceTraceItem::new(ResourceTraceLevel::Error, "Bad PNG.".to_string())];
	let mut catalog = Catalog::new(
		vec![audio("music.wav", 3, 8192)],
		Some(vec![Vec::new()]),
		Vec::new(),
		&[],
		files_usage(),
		0,
	)
	.with_traces(vec![("broken.png".to_string(), trace)]);

	assert_eq!(ids(&mut catalog, "is:failed has:errors"), ["broken.png"]);
	assert_eq!(ids(&mut catalog, "status=current"), ["music.wav"]);
}
