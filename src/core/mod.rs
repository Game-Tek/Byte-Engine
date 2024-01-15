pub mod orchestrator;

pub mod entity;
pub mod domain;
pub mod property;
pub mod event;
pub mod listener;

pub use entity::Entity;
pub use entity::EntityHandle;

pub use orchestrator::Orchestrator;

use crate::core::listener::Listener;

pub fn spawn<E>(entity: impl IntoHandler<E>) -> EntityHandle<E> {
	static mut COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

	struct NoneListener {}
	impl Listener for NoneListener {
		fn invoke_for<T: 'static>(&self, handle: EntityHandle<T>) {
			
		}

		fn add_listener<L, T: 'static>(&self, listener: EntityHandle<L>) where L: orchestrator::EntitySubscriber<T> + 'static {
		}
	}

	let e = entity.call(Option::<&NoneListener>::None, unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) }).unwrap();
	e
}

pub fn spawn_in_domain<E>(listener: &impl Listener, entity: impl IntoHandler<E>) -> EntityHandle<E> {
	static mut COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
	let e = entity.call(Some(listener), unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) }).unwrap();
	e
}

/// Handles extractor pattern for most functions passed to the orchestrator.
pub trait IntoHandler<R> {
	fn call(self, domain: Option<&impl Listener>, cid: u32) -> Option<EntityHandle<R>>;
}

impl <R: 'static> IntoHandler<R> for R {
    fn call(self, listener: Option<&impl Listener>, cid: u32) -> Option<EntityHandle<R>> {
		let internal_id = cid;

		let obj = std::sync::Arc::new(smol::lock::RwLock::new(self));

		let handle = EntityHandle::<R>::new(obj, internal_id,);

		if let Some(listener) = listener {
			listener.invoke_for(handle.clone());
		}

		Some(handle)
    }
}

impl <R: 'static> IntoHandler<R> for orchestrator::EntityReturn<'_, R> {
    fn call(self, domain: Option<&impl Listener>, cid: u32) -> Option<EntityHandle<R>> {
		let internal_id = cid;

		let entity = (self.create)();

		let obj = std::sync::Arc::new(smol::lock::RwLock::new(entity));

		let mut handle = EntityHandle::<R>::new(obj, internal_id,);

		for f in self.post_creation_functions {
			f(&mut handle,);
		}

		for f in self.listens_to {
			f(handle.clone())
		}

		Some(handle)
    }
}