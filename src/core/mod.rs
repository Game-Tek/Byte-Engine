pub mod orchestrator;

pub mod entity;
pub mod domain;
pub mod property;
pub mod event;
pub mod listener;

use std::ops::Deref;

pub use entity::Entity;
pub use entity::EntityHandle;

pub use orchestrator::Orchestrator;

use crate::core::listener::EntitySubscriber;
use crate::core::listener::Listener;

use self::entity::DomainType;
use self::entity::EntityBuilder;

struct NoneListener {}
impl Listener for NoneListener {
	fn invoke_for<T: 'static>(&self, _: EntityHandle<T>) {
		
	}

	fn add_listener<L, T: 'static>(&self, _: EntityHandle<L>) where L: EntitySubscriber<T> + 'static {
	}
}

impl Entity for NoneListener {}

pub fn spawn<E>(entity: impl SpawnHandler<E>) -> EntityHandle<E> {
	static mut COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

	let e = entity.call(Option::None, unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) }).unwrap();
	e
}

// pub fn spawn_as_child<E>(parent: &dyn Entity, entity: impl SpawnHandler<E>) -> EntityHandle<E> {
// 	static mut COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
// 	let e = entity.call(Some(parent), unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) }).unwrap();
// 	e
// }

pub fn spawn_as_child<E>(parent: DomainType, entity: impl SpawnHandler<E>) -> EntityHandle<E> {
	static mut COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
	let e = entity.call(Some(parent), unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) }).unwrap();
	e
}

/// Handles extractor pattern for most functions passed to the orchestrator.
pub trait SpawnHandler<R> {
	fn call(self, domain: Option<DomainType>, cid: u32) -> Option<EntityHandle<R>>;
}

impl <R: 'static> SpawnHandler<R> for R {
    fn call(self, listener: Option<DomainType>, cid: u32) -> Option<EntityHandle<R>> {
		let internal_id = cid;

		let obj = std::sync::Arc::new(smol::lock::RwLock::new(self));

		let handle = EntityHandle::<R>::new(obj, internal_id,);

		if let Some(listener) = listener {
			if let Some(listener) = listener.write_sync().deref().get_listener() {
				listener.invoke_for(handle.clone());
			}
		}

		Some(handle)
    }
}

impl <R: 'static> SpawnHandler<R> for EntityBuilder<'_, R> {
    fn call(self, domain: Option<DomainType>, cid: u32) -> Option<EntityHandle<R>> {
		let internal_id = cid;

		let entity = (self.create)(domain.clone());

		let obj = std::sync::Arc::new(smol::lock::RwLock::new(entity));

		let mut handle = EntityHandle::<R>::new(obj, internal_id,);

		for f in self.post_creation_functions {
			f(&mut handle,);
		}

		if let Some(domain) = domain.clone() {
			for f in self.listens_to {
				f(domain.clone(), handle.clone())
			}
		}

		Some(handle)
    }
}