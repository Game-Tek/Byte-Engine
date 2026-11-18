//! Values a UI component follows as they change outside it.
//!
//! A [`Reactive`] is a component's view of one changing value. Get one from [`Context::reactive`] for a value
//! that notifies, such as a [`Watch`](crate::ui::Watch) subscriber, or from [`Context::proactive`] for a value read
//! from the application context. Await [`Reactive::next`] in a loop or a `select!` beside events: the first wait
//! resolves at once with the current value, and later ones each time the value changed.
//!
//! [`Context::reactive`]: crate::ui::Context::reactive
//! [`Context::proactive`]: crate::ui::Context::proactive

use std::{
	future::Future,
	marker::PhantomData,
	pin::Pin,
	task::{Context, Poll},
};

use utils::r#async::FusedFuture;

use crate::ui::layout::engine::UiPoll;

/// The `Source` trait lets a UI component wait for a value that changes outside it.
///
/// Hand one to [`Context::reactive`](crate::ui::Context::reactive). Implement it for anything that can tell a
/// waiting task that its value may have changed, such as a channel's latest message. A
/// [`Subscriber`](crate::ui::Subscriber) and [`Polled`] implement it.
pub trait Source<C: 'static = ()> {
	type Value: 'static;

	/// Writes the newest value into `latest` and resolves when it changed since the value `latest` holds. With
	/// `latest` empty, as on the first poll, it always resolves. Otherwise it arranges for `cx`'s task to be woken
	/// on a change and returns [`Poll::Pending`].
	fn poll_update(&mut self, latest: &mut Option<Self::Value>, cx: &mut Context<'_>) -> Poll<()>;
}

/// The `Polled` struct is the [`Source`] for a value the UI has to read from the application context to notice a
/// change.
///
/// Get one through [`Context::proactive`](crate::ui::Context::proactive). It reads in evaluations that run anyway,
/// only after the context was written, and never asks for a tick of its own, so an idle interface stays idle. Keep
/// the value small, such as a flag, a count, or a position: every read builds it and compares it with the last.
/// Follow large state through a [`Watch`](crate::ui::Watch) instead.
pub struct Polled<F>(pub(crate) F);

impl<C: 'static, T: PartialEq + 'static, F: FnMut(&C) -> T> Source<C> for Polled<F> {
	type Value = T;

	fn poll_update(&mut self, latest: &mut Option<T>, cx: &mut Context<'_>) -> Poll<()> {
		let value = (self.0)(UiPoll::<C>::from_context(cx).read_context());
		if latest.as_ref() == Some(&value) {
			return Poll::Pending;
		}
		*latest = Some(value);
		Poll::Ready(())
	}
}

/// The `Reactive` struct keeps a component's view of one changing value.
///
/// Get one from [`Context::reactive`](crate::ui::Context::reactive) or
/// [`Context::proactive`](crate::ui::Context::proactive), then await [`Self::next`] in a loop or a `select!` beside
/// events, and update the elements that show the value. Changes that land while the component is busy collapse into
/// the newest value; none is queued.
pub struct Reactive<S: Source<C>, C: 'static = ()> {
	source: S,
	/// The value [`Self::next`] last handed out, which the source writes the next one into.
	latest: Option<S::Value>,
	ctx: PhantomData<fn() -> C>,
}

impl<S: Source<C>, C: 'static> Reactive<S, C> {
	pub(crate) fn new(source: S) -> Self {
		Self {
			source,
			latest: None,
			ctx: PhantomData,
		}
	}

	/// Resolves at once the first time, then each time the value changed, and lends the newest value.
	pub fn next(&mut self) -> Next<'_, S, C> {
		Next { reactive: Some(self) }
	}

	/// Returns the value the last [`Self::next`] lent, without waiting, or `None` before the first.
	pub fn latest(&self) -> Option<&S::Value> {
		self.latest.as_ref()
	}
}

/// The `Next` struct is the wait for a [`Reactive`] value's next change. Get one from [`Reactive::next`].
pub struct Next<'a, S: Source<C>, C: 'static = ()> {
	/// Taken when the wait resolves, so the lent value borrows the reactive value for the full `'a`.
	reactive: Option<&'a mut Reactive<S, C>>,
}

impl<S: Source<C>, C: 'static> Unpin for Next<'_, S, C> {}

impl<'a, S: Source<C>, C: 'static> Future for Next<'a, S, C> {
	type Output = &'a S::Value;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let Some(reactive) = self.reactive.as_deref_mut() else {
			return Poll::Pending;
		};
		if reactive.source.poll_update(&mut reactive.latest, cx).is_pending() {
			return Poll::Pending;
		}
		let reactive = self.reactive.take().unwrap();
		Poll::Ready(
			reactive
				.latest
				.as_ref()
				.expect("A reactive source resolved without a value. The most likely cause is a `Source` implementation that returns `Poll::Ready` without writing `latest`. Write the value before resolving."),
		)
	}
}

impl<S: Source<C>, C: 'static> FusedFuture for Next<'_, S, C> {
	fn is_terminated(&self) -> bool {
		self.reactive.is_none()
	}
}
