//! The requesting half of the loading couple, owned by the render thread.

use std::collections::HashMap;

use super::lane::{LoadError, LoadPipeline, Loaded};

/// The `RequestState` enum tracks one logical resource inside a client's registry.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestState {
	/// Handed to a lane, or waiting for lane capacity.
	Loading,
	/// Published to the render thread. Further requests for this key are ignored.
	Resident,
	/// Failed. A later request retries.
	Failed,
}

/// The `Event` enum reports what a loader finished since the previous frame.
pub enum Event<P: LoadPipeline> {
	Ready { key: P::Key, resident: P::Resident },
	Failed { key: P::Key, error: LoadError },
}

/// The `LoaderClient` struct is the render thread's whole view of loading.
///
/// It coalesces requests and publishes results. It holds no GPU state, because everything a resource
/// needs was already done on a lane.
pub struct LoaderClient<P: LoadPipeline> {
	requests: kanal::AsyncSender<(P::Key, P::Request)>,
	results: kanal::AsyncReceiver<(P::Key, Result<Loaded<P>, LoadError>)>,
	registry: HashMap<P::Key, RequestState>,
}

impl<P: LoadPipeline> LoaderClient<P> {
	pub(super) fn new(
		requests: kanal::AsyncSender<(P::Key, P::Request)>,
		results: kanal::AsyncReceiver<(P::Key, Result<Loaded<P>, LoadError>)>,
	) -> Self {
		Self {
			requests,
			results,
			registry: HashMap::new(),
		}
	}

	/// Requests one resource, ignoring keys already loading or resident.
	///
	/// A key that previously failed is retried. The request channel is unbounded and never blocks the
	/// render thread; coalescing bounds it by the number of distinct keys the scene asks for.
	pub fn request(&mut self, request: P::Request) {
		let key = P::key(&request);
		match self.registry.get(&key) {
			Some(RequestState::Loading | RequestState::Resident) => return,
			Some(RequestState::Failed) | None => {}
		}
		self.registry.insert(key.clone(), RequestState::Loading);
		// A closed channel means every lane stopped, which the next poll reports as no progress.
		let _ = self.requests.as_sync().try_send((key, request));
	}

	/// Returns the next completion without allocating an intermediate collection.
	///
	/// Call this until it returns `None` once per frame, before the frame reads scene state.
	pub fn poll(&mut self) -> Option<Event<P>> {
		let Ok(Some((key, result))) = self.results.as_sync().try_recv() else {
			return None;
		};
		match result {
			Ok(Loaded { resident, dependencies }) => {
				self.registry.insert(key.clone(), RequestState::Resident);
				for dependency in dependencies {
					self.request(dependency);
				}
				Some(Event::Ready { key, resident })
			}
			Err(error) => {
				self.registry.insert(key.clone(), RequestState::Failed);
				Some(Event::Failed { key, error })
			}
		}
	}
}
