//! The file tracker implements an easy to use file tracking system.
//! Directories can be watched during runtime and the file tracker will emit events when files are created, modified or deleted. Events will also be emitted for files that were changed when the watcher was offline.

use std::path::Path;

use notify_debouncer_full::{notify::{*}, new_debouncer, DebounceEventResult, FileIdMap, DebouncedEvent};
use polodb_core;

pub struct FileTracker {
	db: polodb_core::Database,
	debouncer: notify_debouncer_full::Debouncer<INotifyWatcher, FileIdMap>,
	rx: std::sync::mpsc::Receiver<notify_debouncer_full::DebouncedEvent>,
}

impl FileTracker {
	pub fn new() -> FileTracker {
		std::fs::create_dir_all(".byte-editor").unwrap();

		let db = polodb_core::Database::open_file(".byte-editor/files.db").unwrap();

		let (tx, rx) = std::sync::mpsc::channel();

		let mut debouncer = new_debouncer(std::time::Duration::from_secs(1), None, move |event: DebounceEventResult| {
			match event {
				Ok(events) => {
					for event in events {
						tx.send(event).unwrap();
					}
				}
				Err(errors) => {
					for err in errors {
						log::error!("{:?}", err);
					}
				}
			}
		}).unwrap();

		let watcher = debouncer.watcher();

		let col = db.collection::<polodb_core::bson::Document>("files");

		for doc in col.find(None).unwrap() {
			let d = doc.unwrap();
			let path = d.get_str("path").unwrap();
			let path = Path::new(path);
			
			if !path.exists() {
				// TODO: emit delete event
				// TODO: Delete document
				continue;
			}

			let metadata = std::fs::metadata(path).unwrap();
			let time = metadata.modified().unwrap();
			let _m = time.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();

			dbg!(&d);
			dbg!(_m as u64);

			if d.get_i64("last_modified").unwrap() as u64 != _m as u64 {
				log::trace!("File changed: {:?}", path);

				db.collection::<polodb_core::bson::Document>("files").update_one(polodb_core::bson::doc! {
					"path": path.to_str().unwrap(),
				}, polodb_core::bson::doc! {
					"$set": {
						"last_modified": _m as i64,
					}
				}).unwrap();
			}

			watcher.watch(path, RecursiveMode::Recursive).expect("Failed to watch path");
		}

		FileTracker {
			db,
			debouncer,
			rx,
		}
	}

	pub fn watch(&mut self, path: &Path) -> bool {
		let result = self.debouncer.watcher().watch(path, RecursiveMode::Recursive);

		if result.is_err() {
			log::warn!("Failed to watch path: {:?}", path);
			return false;
		}

		if !path.ends_with("*") { // TODO. when watching a directory, the directory itself is not added to the database, the files inside are.
			let metadata = std::fs::metadata(path).unwrap();
			let time = metadata.modified().unwrap();
			let _m = time.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();

			let res = self.db.collection::<polodb_core::bson::Document>("files").find_one(polodb_core::bson::doc! { "path": path.to_str().unwrap(),}).unwrap();

			if res.is_none() {
				self.db.collection("files").insert_one(polodb_core::bson::doc! {
					"path": path.to_str().unwrap(),
					"last_modified": _m as i64,
				}).unwrap();
			}
		}
		
		true
	}

	pub fn unwatch(&mut self, path: &Path) -> bool {
		let result = self.debouncer.watcher().unwatch(path);

		if result.is_err() {
			return false;
		}

		// self.db.collection("files").delete_one(polodb_core::bson::doc! {
		// 	"path": path.to_str().unwrap(),
		// }).unwrap();

		true
	}

	pub fn poll(&self) -> Vec<DebouncedEvent> {
		let mut events = Vec::new();

		loop {
			match self.rx.try_recv() {
				Ok(event) => {
					dbg!(&event);
					events.push(event);
				}
				Err(_) => break,
			}
		}

		events
	}
}