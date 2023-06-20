//! The file tracker implements an easy to use file tracking system.
//! Directories can be watched during runtime and the file tracker will emit events when files are created, modified or deleted. Events will also be emitted for files that were changed when the watcher was offline.

use std::path::Path;

use notify::{Watcher, RecursiveMode};
use polodb_core;

pub struct FileTracker {
	db: polodb_core::Database,
	watcher: notify::RecommendedWatcher,
	rx: std::sync::mpsc::Receiver<std::result::Result<notify::Event, notify::Error>>,
}

impl FileTracker {
	pub fn new() -> FileTracker {
		let db = polodb_core::Database::open_file("files.db").unwrap();

		let (tx, rx) = std::sync::mpsc::channel();

		let mut watcher = notify::RecommendedWatcher::new(tx, notify::Config::default()).unwrap();

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
				println!("File changed: {:?}", path);

				db.collection::<polodb_core::bson::Document>("files").update_one(polodb_core::bson::doc! {
					"path": path.to_str().unwrap(),
				}, polodb_core::bson::doc! {
					"$set": {
						"last_modified": _m as i64,
					}
				}).unwrap();
			}

			watcher.watch(path, RecursiveMode::Recursive);
		}

		FileTracker {
			db,
			watcher,
			rx,
		}
	}

	pub fn watch(&mut self, path: &Path) -> bool {
		let result = self.watcher.watch(path, RecursiveMode::Recursive);

		if result.is_err() {
			println!("Failed to watch path: {:?}", path);
			return false;
		}

		let metadata = std::fs::metadata(path).unwrap();
		let time = metadata.modified().unwrap();
		let _m = time.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();

		let res = self.db.collection::<polodb_core::bson::Document>("files").find_one(polodb_core::bson::doc! { "path": path.to_str().unwrap(),}).unwrap();

		if !res.is_some() {
			self.db.collection("files").insert_one(polodb_core::bson::doc! {
				"path": path.to_str().unwrap(),
				"last_modified": _m as i64,
			}).unwrap();
		}
		
		true
	}

	pub fn unwatch(&mut self, path: &Path) -> bool {
		let result = self.watcher.unwatch(path);

		if result.is_err() {
			return false;
		}

		// self.db.collection("files").delete_one(polodb_core::bson::doc! {
		// 	"path": path.to_str().unwrap(),
		// }).unwrap();

		true
	}

	pub fn poll(&self) -> () {
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
	}
}