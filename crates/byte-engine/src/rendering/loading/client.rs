//! The requesting half of the loading couple, owned by the render thread.

use std::collections::HashSet;

use super::lane::{LoadError, LoadPipeline, Submission};

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
	requests: kanal::AsyncSender<Submission<P>>,
	results: kanal::AsyncReceiver<(P::Key, Result<P::Resident, LoadError>)>,
	/// Keys this client already sent. Failed requests become eligible for retry. Lanes coalesce the rest.
	sent: HashSet<P::Key>,
}

impl<P: LoadPipeline> LoaderClient<P> {
	pub(super) fn new(
		requests: kanal::AsyncSender<Submission<P>>,
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
			let _ = self.requests.as_sync().try_send(Submission {
				key,
				request,
				reload: false,
			});
		}
	}

	/// Loads one resource again, even when it is already loading or resident.
	///
	/// Use this when the stored resource changed, for example after a development rebake. Its result arrives
	/// through [`Self::poll`] like any other, and the render thread replaces what it adopted before. Resources it
	/// depends on reload only when they are reloaded themselves.
	pub fn reload(&mut self, request: P::Request) {
		let key = P::key(&request);
		self.sent.insert(key.clone());
		let _ = self.requests.as_sync().try_send(Submission {
			key,
			request,
			reload: true,
		});
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

	/// Reads the next submission as `(key, request, reload)` so tests can compare it directly.
	fn next(worker_requests: &kanal::AsyncReceiver<Submission<TestPipeline>>) -> Option<(u32, u32, bool)> {
		worker_requests
			.as_sync()
			.try_recv()
			.unwrap()
			.map(|submission| (submission.key, submission.request, submission.reload))
	}

	#[test]
	fn requests_are_sent_once_until_they_fail() {
		let (requests, worker_requests) = kanal::unbounded_async();
		let (worker_results, results) = kanal::unbounded_async();
		let mut client = LoaderClient::<TestPipeline>::new(requests, results);

		client.request(1);
		client.request(1);
		assert_eq!(next(&worker_requests), Some((1, 1, false)));
		assert_eq!(next(&worker_requests), None);

		worker_results
			.as_sync()
			.send((1, Err(LoadError("test load failed".into()))))
			.unwrap();
		assert!(matches!(client.poll(), Some(Event::Failed { key: 1, .. })));
		client.request(1);
		assert_eq!(next(&worker_requests), Some((1, 1, false)));

		worker_results.as_sync().send((1, Ok(10))).unwrap();
		assert!(matches!(client.poll(), Some(Event::Ready { key: 1, resident: 10 })));
		client.request(1);
		assert_eq!(next(&worker_requests), None);
		assert!(client.poll().is_none());
	}

	#[test]
	fn reloads_are_sent_even_for_requested_keys() {
		let (requests, worker_requests) = kanal::unbounded_async();
		let (_worker_results, results) = kanal::unbounded_async();
		let mut client = LoaderClient::<TestPipeline>::new(requests, results);

		client.request(1);
		client.reload(1);
		assert_eq!(next(&worker_requests), Some((1, 1, false)));
		assert_eq!(next(&worker_requests), Some((1, 1, true)));
		client.request(1);
		assert_eq!(next(&worker_requests), None);
	}

	#[test]
	fn dropping_the_client_stops_lanes_that_can_still_send_dependencies() {
		let (requests, worker_requests) = kanal::unbounded_async::<Submission<TestPipeline>>();
		let (_worker_results, results) = kanal::unbounded_async();
		// A lane keeps a sender for the dependencies it discovers.
		let _dependencies = requests.clone();
		drop(LoaderClient::<TestPipeline>::new(requests, results));

		assert!(worker_requests.as_sync().recv().is_err());
	}
}
