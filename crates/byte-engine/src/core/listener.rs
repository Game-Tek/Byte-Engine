use std::marker::PhantomData;

use trotcast::Receiver;

/// The `Listener` trait exists to decouple message consumption from transport details.
pub trait Listener<M> {
    fn read(&mut self) -> Option<M>;

    fn iter(&mut self) -> ListenerIterator<'_, Self, M>
    where
        Self: Sized,
    {
        ListenerIterator::new(self)
    }
}

/// The `ListenerIterator` struct exists to provide iterator semantics for any listener implementation.
pub struct ListenerIterator<'a, L: ?Sized, M>
where
    L: Listener<M>,
{
    listener: &'a mut L,
    _marker: PhantomData<M>,
}

impl<'a, L: ?Sized, M> ListenerIterator<'a, L, M>
where
    L: Listener<M>,
{
    fn new(listener: &'a mut L) -> Self {
        Self {
            listener,
            _marker: PhantomData,
        }
    }
}

impl<'a, L: ?Sized, M> Iterator for ListenerIterator<'a, L, M>
where
    L: Listener<M>,
{
    type Item = M;

    fn next(&mut self) -> Option<Self::Item> {
        self.listener.read()
    }
}

impl<'a, M> IntoIterator for &'a mut (dyn Listener<M> + 'a) {
    type Item = M;
    type IntoIter = ListenerIterator<'a, dyn Listener<M> + 'a, M>;

    fn into_iter(self) -> Self::IntoIter {
        ListenerIterator::new(self)
    }
}

/// The `DefaultListener` struct exists to read messages from a `trotcast` receiver.
#[derive(Clone)]
pub struct DefaultListener<M>(pub(crate) Receiver<M>);

impl<M: Clone> DefaultListener<M> {
    pub fn filtered<F>(&self, filter: F) -> FilteredListener<DefaultListener<M>, M, F>
    where
        F: Fn(&M) -> bool,
    {
        FilteredListener(self.clone(), filter, PhantomData)
    }
}

impl<M: Clone> Listener<M> for DefaultListener<M> {
    fn read(&mut self) -> Option<M> {
        self.0.try_recv().ok()
    }
}

/// The `FilteredListener` struct exists to compose message predicates over listeners.
pub struct FilteredListener<L, M: Clone, F>(L, F, PhantomData<M>)
where
    L: Listener<M>,
    F: Fn(&M) -> bool;

impl<R, M: Clone, F> FilteredListener<R, M, F>
where
    R: Listener<M>,
    F: Fn(&M) -> bool,
{
}

impl<L, M: Clone, F> Listener<M> for FilteredListener<L, M, F>
where
    L: Listener<M>,
    F: Fn(&M) -> bool,
{
    fn read(&mut self) -> Option<M> {
        // Drain pending messages until one satisfies the filter predicate.
        while let Some(message) = self.0.read() {
            if (self.1)(&message) {
                return Some(message);
            }
        }

        None
    }
}
