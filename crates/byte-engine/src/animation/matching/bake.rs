//! Baking `.motion.bead` files into motion databases.

use resource_management::{
	AssetHandler, BakeContext, ProcessedAsset,
	asset::{ResourceId, handler::LoadErrors},
	resources::animation::{Animation, AnimationModel},
};

use super::{bead::MotionBead, features::build_database};

/// The `MotionDatabaseAssetHandler` struct lets an asset manager bake `.motion.bead` files into the motion databases
/// matched animation states search.
///
/// Baking a database bakes every clip it names, samples them, and stores one
/// [`MotionDatabase`](resource_management::resources::motion_database::MotionDatabase) under the file's ID. Register it
/// with [`AssetManager::add_asset_handler`](resource_management::asset::manager::AssetManager::add_asset_handler);
/// next, build a matched state that names the file with
/// [`AnimationGraphStateBuilder::matched`](crate::animation::graph::AnimationGraphStateBuilder::matched).
#[derive(Clone, Copy, Debug, Default)]
pub struct MotionDatabaseAssetHandler;

impl AssetHandler for MotionDatabaseAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("motion.bead")
	}

	/// Parses the file, bakes and samples its clips, and stores the database they form.
	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (source, asset_type) = context.resolve(id).await?;
		if !self.can_handle(&asset_type) {
			return Err(LoadErrors::UnsupportedType);
		}
		let invalid = |cause: &str| {
			context.error(format_args!(
				"Motion database '{}' is invalid. The most likely cause is {cause}. Fix the file as the format specifies; see {}.",
				id.as_ref(),
				utils::online_docs_url(DOCS_PATH)
			));
			LoadErrors::FailedToProcess
		};
		let bead = MotionBead::parse(&source).map_err(|cause| invalid(&cause))?;
		let clip_ids = clip_ids(&context, &bead.clips).await.map_err(|cause| invalid(&cause))?;

		let models = context.bake_dependencies::<AnimationModel>(&clip_ids, 8).await?;
		let mut animations = Vec::with_capacity(models.len());
		for (clip, model) in clip_ids.iter().zip(models) {
			let animation = context
				.solve(model)
				.await
				.map_err(|_| invalid(&format!("that '{clip}' names a resource that isn't an animation clip")))?;
			animations.push(animation);
		}
		let clips: Vec<(String, &Animation)> = clip_ids
			.iter()
			.cloned()
			.zip(animations.iter().map(|animation| animation.resource()))
			.collect();

		let database = build_database(&bead, &clips).map_err(|error| {
			context.error(format_args!(
				"Motion database '{}' could not be built. {error} See {}.",
				id.as_ref(),
				utils::online_docs_url(DOCS_PATH)
			));
			LoadErrors::FailedToProcess
		})?;
		context.store_primary(ProcessedAsset::new(id, database), &[]).await
	}
}

/// Resolves the `clips` entries into clip IDs in database order: an ID names itself, and a pattern names the source
/// files it matches in ascending ID order. A clip named twice keeps the place of its first entry.
///
/// Returns why an entry names nothing.
async fn clip_ids(context: &BakeContext<'_>, entries: &[String]) -> Result<Vec<String>, String> {
	// Only patterns need the source list, so a database of plain IDs never enumerates the sources.
	let mut sources: Option<Vec<String>> = None;
	let mut ids: Vec<String> = Vec::with_capacity(entries.len());
	for entry in entries {
		let named = if entry.contains(['*', '?']) {
			if sources.is_none() {
				let mut discovered = context
					.source_ids()
					.await
					.map_err(|_| format!("that the sources could not be listed to match '{entry}'"))?;
				discovered.sort_unstable();
				sources = Some(discovered);
			}
			let pattern = utils::glob::Glob::new(entry);
			let matches: Vec<String> = sources
				.iter()
				.flatten()
				.filter(|source| pattern.matches(source))
				.cloned()
				.collect();
			if matches.is_empty() {
				return Err(format!("that the pattern '{entry}' in `clips` matches no source file"));
			}
			matches
		} else {
			vec![entry.clone()]
		};
		for id in named {
			if !ids.contains(&id) {
				ids.push(id);
			}
		}
	}
	Ok(ids)
}

const DOCS_PATH: &str = "reference/assets#motion-databases";

#[cfg(test)]
mod tests {
	use resource_management::{
		asset::{handler::implementations::FBXAssetHandler, manager::AssetManager, storage_backend::FileStorageBackend},
		resource::{resource_manager::ResourceManager, storage_backend::MemoryStorageBackend},
		resources::motion_database::MotionDatabase,
	};

	use super::MotionDatabaseAssetHandler;

	/// A one-second clip whose node `AnimatedNode` moves along X at a steady pace.
	const MOVING_FBX: &[u8] = include_bytes!("../../../../resource-management/src/asset/test_data/animation_only_ascii.fbx");

	/// A source directory of its own for each test, removed when the test ends.
	struct Sources(std::path::PathBuf);

	impl Sources {
		fn new(name: &str) -> Self {
			let directory = std::env::temp_dir().join(format!("byte-engine-motion-{name}-{}", std::process::id()));
			let _ = std::fs::remove_dir_all(&directory);
			std::fs::create_dir_all(directory.join("clips")).expect("the test source directory should be created");
			Self(directory)
		}

		fn add(&self, id: &str, contents: &[u8]) {
			std::fs::write(self.0.join(id), contents).expect("the test source should be written");
		}

		fn asset_manager(&self, resources: MemoryStorageBackend) -> AssetManager {
			let mut asset_manager = AssetManager::new(FileStorageBackend::new(self.0.clone()), resources);
			asset_manager.add_asset_handler(FBXAssetHandler::default());
			asset_manager.add_asset_handler(MotionDatabaseAssetHandler);
			asset_manager
		}
	}

	impl Drop for Sources {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.0);
		}
	}

	#[compio::test]
	async fn a_motion_bead_bakes_its_clips_into_a_database_in_entry_order() {
		let sources = Sources::new("order");
		sources.add("clips/b.fbx", MOVING_FBX);
		sources.add("clips/a.fbx", MOVING_FBX);
		sources.add("solo.fbx", MOVING_FBX);
		sources.add(
			"moving.motion.bead",
			br#"{
				clips: ["solo.fbx", "clips/*.fbx", "clips/a.fbx"],
				root: { node: "AnimatedNode", position_smoothing: 0 },
				trajectory: { times: [0.5] },
			}"#,
		);
		let resources = MemoryStorageBackend::new();

		sources
			.asset_manager(resources.clone())
			.bake("moving.motion.bead")
			.await
			.expect("the motion database and its clips should bake");

		let database = ResourceManager::new(resources)
			.request::<MotionDatabase>("moving.motion.bead")
			.await
			.expect("the baked database should be requested");
		let database = database.resource();
		let clips: Vec<_> = database
			.clips
			.iter()
			.map(|clip| (clip.id.as_str(), clip.frames, clip.pickable))
			.collect();
		assert_eq!(
			clips,
			[("solo.fbx", 31, 16), ("clips/a.fbx", 31, 16), ("clips/b.fbx", 31, 16)],
			"patterns match in ascending ID order, and a clip named twice keeps its first place"
		);
		// The clip moves its node along X at a steady pace, so half way the ground root has covered half the travel.
		let ([half, z, _], [whole, ..]) = (database.ground[15], database.ground[30]);
		assert!(
			whole > 0.0 && (half - whole / 2.0).abs() < 1.0e-5 && z.abs() < 1.0e-6,
			"{half}, {z}, {whole}"
		);
	}

	#[compio::test]
	async fn a_motion_bead_whose_clips_cannot_form_a_database_fails_to_bake() {
		for (case, bead) in [
			(
				"pattern",
				&br#"{ clips: ["missing/*.fbx"], root: { node: "AnimatedNode" } }"#[..],
			),
			("node", &br#"{ clips: ["solo.fbx"], root: { node: "Hips" } }"#[..]),
			(
				"unknown",
				&br#"{ clips: ["solo.fbx"], root: { node: "AnimatedNode" }, speed: 2 }"#[..],
			),
		] {
			let sources = Sources::new(case);
			sources.add("solo.fbx", MOVING_FBX);
			sources.add("bad.motion.bead", bead);

			let baked = sources
				.asset_manager(MemoryStorageBackend::new())
				.bake("bad.motion.bead")
				.await;
			assert!(baked.is_err(), "the {case} case should fail to bake");
		}
	}
}
