use std::{
	future::Future,
	pin::Pin,
	task::{Context, Poll},
	time::{Duration, Instant},
};

use utils::r#async::FusedFuture;

/// Returns a future that completes after `duration`.
pub fn wait(duration: Duration) -> WaitFuture {
	WaitFuture {
		deadline: Instant::now() + duration,
		armed: false,
		complete: false,
	}
}

/// Returns a future that completes after `seconds`.
pub fn seconds(seconds: u64) -> WaitFuture {
	wait(Duration::from_secs(seconds))
}

pub struct WaitFuture {
	deadline: Instant,
	armed: bool,
	complete: bool,
}

impl Future for WaitFuture {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let now = Instant::now();
		if now >= self.deadline {
			self.complete = true;
			return Poll::Ready(());
		}

		if !self.armed {
			let waker = cx.waker().clone();
			let duration = self.deadline.saturating_duration_since(now);
			self.armed = true;
			std::thread::spawn(move || {
				std::thread::sleep(duration);
				waker.wake();
			});
		}

		Poll::Pending
	}
}

impl FusedFuture for WaitFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}
