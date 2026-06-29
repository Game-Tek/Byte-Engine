use std::{
	future::Future,
	pin::Pin,
	sync::{Mutex, OnceLock},
	task::{Context, Poll, Waker},
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

struct TimerWaiter {
	deadline: Instant,
	waker: Waker,
}

fn timer_waiters() -> &'static Mutex<Vec<TimerWaiter>> {
	static WAITERS: OnceLock<Mutex<Vec<TimerWaiter>>> = OnceLock::new();
	WAITERS.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn wake_due_timers(now: Instant) {
	let mut waiters = timer_waiters().lock().expect("UI timer waiter lock poisoned");
	let mut i = 0;
	while i < waiters.len() {
		if waiters[i].deadline <= now {
			let waiter = waiters.swap_remove(i);
			waiter.waker.wake();
		} else {
			i += 1;
		}
	}
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
			self.armed = true;
			timer_waiters()
				.lock()
				.expect("UI timer waiter lock poisoned")
				.push(TimerWaiter {
					deadline: self.deadline,
					waker: cx.waker().clone(),
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
