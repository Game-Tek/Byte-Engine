use std::{
	future::Future,
	marker::PhantomData,
	pin::Pin,
	task::{Context, Poll},
	time::{Duration, Instant},
};

use utils::r#async::FusedFuture;

use crate::ui::layout::engine::UiPoll;

/// The `WaitFuture` struct lets a mounted UI component pause until a deadline.
///
/// Get one from [`crate::ui::Context::wait`] or [`crate::ui::Context::seconds`] and await it inside a component. The
/// wait registers with the component's engine, which wakes it from [`crate::ui::Engine::evaluate`] once the deadline
/// passes and schedules that evaluation through [`crate::ui::Engine::next_tick`]. Like other UI waits, it stays
/// registered only while its task keeps polling it.
pub struct WaitFuture<C = ()> {
	deadline: Instant,
	complete: bool,
	/// The engine context type, which names the poll state this wait reaches the runtime through.
	ctx: PhantomData<fn() -> C>,
}

impl<C> WaitFuture<C> {
	/// Makes a wait that completes `duration` from now.
	pub(crate) fn new(duration: Duration) -> Self {
		Self {
			deadline: Instant::now() + duration,
			complete: false,
			ctx: PhantomData,
		}
	}
}

impl<C: 'static> Future for WaitFuture<C> {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		if Instant::now() >= self.deadline {
			self.complete = true;
			return Poll::Ready(());
		}

		UiPoll::<C>::from_context(cx).wait_until(self.deadline);
		Poll::Pending
	}
}

impl<C: 'static> FusedFuture for WaitFuture<C> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}
