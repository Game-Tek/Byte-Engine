//! The requesting half of the loading couple, owned by the render thread.

use std::collections::HashSet;

use super::lane::{LoadError, LoadPipeline};

/// The `Event` enum reports what a loader finished since the previous frame.
pub enum Event<P: LoadPipeline> {
	Ready { key: P::Key, resident: P::Resident },
	Failed { key: P::Key, error: LoadError },
}

/// The `LoaderClient` struct is the render thread's whole view of loading.
///
/// It sends requests and publishes results. It holds no GPU state, because everything a resource needs was
/// already done on a lane. Lanes schedule the dependencies they discover themselves, so their results arrive
/// here like any other.
pub struct LoaderClient<P: LoadPipeline> {
	requests: kanal::AsyncSender<(P::Key, P::Request)>,
	results: kanal::AsyncReceiver<(P::Key, Result<P::Resident, LoadError>)>,
	/// Keys this client already sent. Failed requests become eligible for retry. Lanes coalesce the rest.
	sent: HashSet<P::Key>,
}

impl<P: LoadPipeline> LoaderClient<P> {
	pub(super) fn new(
		requests: kanal::AsyncSender<(P::Key, P::Request)>,
		results: kanal::AsyncReceiver<(P::Key, Result<P::Resident, LoadError>)>,
	) -> Self {
		Self {
			requests,
			results,
			sent: HashSet::new(),
		}
	}

	/// Requests one resource, ignoring keys this client already requested.
	///
	/// A key that previously failed is retried. The request channel is unbounded and never blocks the
	/// render thread; coalescing bounds it by the number of distinct keys the scene asks for.
	pub fn request(&mut self, request: P::Request) {
		let key = P::key(&request);
		if self.sent.insert(key.clone()) {
			// A closed channel means every lane stopped, which the next poll reports as no progress.
			let _ = self.requests.as_sync().try_send((key, request));
		}
	}

	/// Returns the next completion without allocating an intermediate collection.
	///
	/// Call this until it returns `None` once per frame, before the frame reads scene state.
	pub fn poll(&mut self) -> Option<Event<P>> {
		let Ok(Some((key, result))) = self.results.as_sync().try_recv() else {
			return None;
		};
		match result {
			Ok(resident) => Some(Event::Ready { key, resident }),
			Err(error) => {
				self.sent.remove(&key);
				Some(Event::Failed { key, error })
			}
		}
	}
}

impl<P: LoadPipeline> Drop for LoaderClient<P> {
	/// Closes the request stream so idle lanes stop, even though each lane can still send it dependencies.
	fn drop(&mut self) {
		let _ = self.requests.close();
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

		async fn load(&self, request: u32, _lane: &mut LoaderLane<Self>) -> Result<u32, LoadError> {
			Ok(request)
		}
	}

	#[test]
	fn requests_are_sent_once_until_they_fail() {
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

		worker_results.as_sync().send((1, Ok(10))).unwrap();
		assert!(matches!(client.poll(), Some(Event::Ready { key: 1, resident: 10 })));
		client.request(1);
		assert_eq!(worker_requests.as_sync().try_recv().unwrap(), None);
		assert!(client.poll().is_none());
	}

	#[test]
	fn dropping_the_client_stops_lanes_that_can_still_send_dependencies() {
		let (requests, worker_requests) = kanal::unbounded_async::<(u32, u32)>();
		let (_worker_results, results) = kanal::unbounded_async();
		// A lane keeps a sender for the dependencies it discovers.
		let _dependencies = requests.clone();
		drop(LoaderClient::<TestPipeline>::new(requests, results));

		assert!(worker_requests.as_sync().recv().is_err());
	}
}
