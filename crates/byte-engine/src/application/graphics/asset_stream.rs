//! The asset manager's bakes, as the `bakes` debug data stream.
//!
//! [`open`] gives the asset manager a [`StreamBakeReporter`] and spawns the [`DebugDataStream`] it writes to, so tools
//! that read streams, such as the console, show bakes while an application runs. The
//! [bake stream reference](/docs/reference/resource-management#bake-stream) specifies the records.

use resource_management::asset::{
	bake_state::{BakeReporter, BakeState},
	manager::AssetManager,
};
use utils::sync::Mutex;

use crate::{
	core::{
		factory::{CreateMessage, Creator as _, Handle},
		listener::{DefaultListener, Listener as _},
		message_bus::MessageScope,
	},
	inspector::{DebugDataStream, DebugDataStreamWriter},
};

/// The name the asset manager's stream carries in every record.
const STREAM_NAME: &str = "Assets";

/// Reports `asset_manager`'s bakes as the `bakes` stream of a new entity in `world`, named [`STREAM_NAME`].
///
/// Call it while configuring the asset manager, before it is shared. The stream writes once the application's
/// streams are open, which a `debug.streams` destination or the console does.
pub(crate) fn open(asset_manager: &mut AssetManager, world: &MessageScope) {
	let entity = Handle::new();
	// The listener subscribes before the stream is spawned, so it sees the writer the application derives for it.
	let reporter = StreamBakeReporter {
		entity,
		writer: Mutex::new(WriterSlot {
			writer: None,
			creations: world.factory().listener(),
		}),
	};
	asset_manager.set_bake_reporter(std::sync::Arc::new(reporter));
	world.create(DebugDataStream::new(entity, STREAM_NAME));
}

/// The `StreamBakeReporter` struct writes an asset manager's [`BakeState`] to the stream of the entity that stands for
/// the asset manager.
struct StreamBakeReporter {
	entity: Handle,
	writer: Mutex<WriterSlot>,
}

/// The writer for the asset manager's stream, once the application derived it.
struct WriterSlot {
	writer: Option<DebugDataStreamWriter>,
	creations: DefaultListener<CreateMessage<DebugDataStreamWriter>>,
}

/// The part of a [`BakeState`] whose change warrants a record: the running bakes and the totals, not their times.
#[derive(serde::Serialize)]
struct Key<'a> {
	baking: Vec<&'a str>,
	baked: u64,
	failed: u64,
}

impl BakeReporter for StreamBakeReporter {
	fn report(&self, state: &BakeState) {
		let mut slot = self.writer.lock();
		let WriterSlot { writer, creations } = &mut *slot;
		// The writer arrives on a later tick than the request. States reported before it are skipped; each record
		// carries the whole state, so the first one written shows everything so far.
		while let Some(created) = creations.read() {
			if created.data().entity() == self.entity {
				*writer = Some(created.data().clone());
			}
		}
		let Some(writer) = writer else { return };
		let key = Key {
			baking: state.baking.iter().map(|bake| bake.id.as_str()).collect(),
			baked: state.baked,
			failed: state.failed,
		};
		writer.write("bakes", &key, state);
	}
}

#[cfg(test)]
mod tests {
	use std::{sync::Arc, time::Instant};

	use resource_management::asset::bake_state::BakingAsset;

	use super::*;
	use crate::{
		core::message_bus::MessageBus,
		inspector::{DebugDataStreams, RetainedStreams, StreamOutput},
	};

	fn state(baked: u64) -> BakeState {
		BakeState {
			baking: vec![BakingAsset {
				id: "models/tree.glb".to_string(),
				elapsed_ms: 1.0,
			}],
			baked,
			..BakeState::default()
		}
	}

	#[test]
	fn the_reporter_writes_once_the_application_derives_its_stream() {
		let message_bus = MessageBus::default();
		let world = message_bus.new_scope("asset-stream-test-world");
		let retained = Arc::new(RetainedStreams::default());
		let output = Arc::new(StreamOutput::open(None, false, Instant::now(), Arc::clone(&retained)).expect("opens"));
		let mut streams = DebugDataStreams::new(&world, Some(output));

		let entity = Handle::new();
		let reporter = StreamBakeReporter {
			entity,
			writer: Mutex::new(WriterSlot {
				writer: None,
				creations: world.factory().listener(),
			}),
		};
		world.create(DebugDataStream::new(entity, STREAM_NAME));

		reporter.report(&state(1));
		assert!(
			retained.latest("bakes").is_none(),
			"no writer exists before the application derives it"
		);

		streams.update();
		reporter.report(&state(2));

		let (_, record) = retained.latest("bakes").expect("a retained record");
		assert_eq!(record["name"], STREAM_NAME);
		assert_eq!(record["entity"], entity.id());
		let reported: BakeState = serde_json::from_value(record["data"].clone()).expect("the data is a bake state");
		assert_eq!(reported, state(2));
	}
}
