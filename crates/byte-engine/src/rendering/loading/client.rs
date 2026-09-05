//! The requesting half of the loading couple, owned by the render thread.

use std::collections::HashSet;

use super::lane::{LoadError, LoadPipeline, Loaded};

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
	/// Keys already loading or resident; failed requests become eligible for retry.
	registry: HashSet<P::Key>,
}

impl<P: LoadPipeline> LoaderClient<P> {
	pub(super) fn new(
		requests: kanal::AsyncSender<(P::Key, P::Request)>,
		results: kanal::AsyncReceiver<(P::Key, Result<Loaded<P>, LoadError>)>,
	) -> Self {
		Self {
			requests,
			results,
			registry: HashSet::new(),
		}
	}

	/// Requests one resource, ignoring keys already loading or resident.
	///
	/// A key that previously failed is retried. The request channel is unbounded and never blocks the
	/// render thread; coalescing bounds it by the number of distinct keys the scene asks for.
	pub fn request(&mut self, request: P::Request) {
		let key = P::key(&request);
		if self.registry.contains(&key) {
			return;
		}
		self.registry.insert(key.clone());
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
				for dependency in dependencies {
					self.request(dependency);
				}
				Some(Event::Ready { key, resident })
			}
			Err(error) => {
				self.registry.remove(&key);
				Some(Event::Failed { key, error })
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::rendering::loading::LoaderLane;

	/// The `TestPipeline` struct supplies plain values at the client-worker channel boundary.
	struct TestPipeline;

	impl LoadPipeline for TestPipeline {
		type Key = u32;
		type Request = u32;
		type Resident = u32;

		fn key(request: &u32) -> u32 {
			*request
		}

		async fn load(&self, request: u32, _lane: &mut LoaderLane<Self>) -> Result<Loaded<Self>, LoadError> {
			Ok(Loaded::new(request))
		}
	}

	#[test]
	fn requests_coalesce_until_failure_and_retry_dependencies_once() {
		let (requests, worker_requests) = kanal::unbounded_async();
		let (worker_results, results) = kanal::unbounded_async();
		let mut client = LoaderClient::<TestPipeline>::new(requests, results);

		client.request(1);
		client.request(1);
		assert_eq!(worker_requests.as_sync().try_recv().unwrap(), Some((1, 1)));
		assert_eq!(worker_requests.as_sync().try_recv().unwrap(), None);

		worker_results
			.as_sync()
			.send((1, Err(LoadError("test load failed".into()))))
			.unwrap();
		assert!(matches!(client.poll(), Some(Event::Failed { key: 1, .. })));
		client.request(1);
		assert_eq!(worker_requests.as_sync().try_recv().unwrap(), Some((1, 1)));

		worker_results
			.as_sync()
			.send((
				1,
				Ok(Loaded {
					resident: 10,
					dependencies: vec![1, 2, 2],
				}),
			))
			.unwrap();
		assert!(matches!(client.poll(), Some(Event::Ready { key: 1, resident: 10 })));
		client.request(1);
		assert_eq!(worker_requests.as_sync().try_recv().unwrap(), Some((2, 2)));
		assert_eq!(worker_requests.as_sync().try_recv().unwrap(), None);
		assert!(client.poll().is_none());
	}
}
