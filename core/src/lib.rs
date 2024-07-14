#![feature(coerce_unsized, unsize, async_closure, trivial_bounds, impl_trait_in_fn_trait_return, type_alias_impl_trait, lazy_type_alias)]

pub mod orchestrator;

pub mod entity;
pub mod domain;
pub mod property;
pub mod event;
pub mod listener;

use std::future::Future;
use std::ops::Deref;

pub use entity::Entity;
pub use entity::EntityHandle;

pub use orchestrator::Orchestrator;
use listener::EntitySubscriber;
use listener::Listener;

use entity::DomainType;
use entity::EntityBuilder;
use utils::r#async::RwLock;

struct NoneListener {}
impl Listener for NoneListener {
	fn invoke_for<'a, T: ?Sized + 'static>(&'a self, _: EntityHandle<T>, _: &'a T) -> utils::BoxedFuture<'a, ()> { Box::pin(async move {}) }
	fn add_listener<T: ?Sized + 'static>(&self, _: EntityHandle<dyn EntitySubscriber<T>>) {}
}

impl Entity for NoneListener {}

pub async fn spawn<E>(entity: impl SpawnHandler<E>) -> EntityHandle<E> {
	static mut COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

	let e = entity.call(Option::None, unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) }).await.unwrap();
	e
}

pub async fn spawn_as_child<'a, E>(parent: DomainType, entity: impl SpawnHandler<E>) -> EntityHandle<E> {
	static mut COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
	let e = entity.call(Some(parent), unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) }).await.unwrap();
	e
}

// TODO: alert when no one is listening to an specific entity

/// Handles extractor pattern for most functions passed to the orchestrator.
pub trait SpawnHandler<R> {
	fn call<'a>(self, domain: Option<DomainType>, cid: u32) -> impl Future<Output = Option<EntityHandle<R>>> where Self: Sized;
}

impl <R: 'static> SpawnHandler<R> for R {
    async fn call<'a>(self, domain: Option<DomainType>, cid: u32) -> Option<EntityHandle<R>> {
		let internal_id = cid;

		let obj = std::sync::Arc::new(RwLock::new(self));

		let handle = EntityHandle::<R>::new(obj, internal_id,);

		if let Some(domain) = domain {
			if let Some(listener) = domain.write_sync().deref().get_listener() {
				listener.invoke_for(handle.clone(), handle.read_sync().deref()).await;
			}
		}

		Some(handle)
    }
}

impl <R: Entity + 'static> SpawnHandler<R> for EntityBuilder<'_, R> {
    async fn call<'a>(self, domain: Option<DomainType>, cid: u32) -> Option<EntityHandle<R>> {
		let internal_id = cid;

		let entity = (self.create)(domain.clone()).await;

		let obj = std::sync::Arc::new(RwLock::new(entity));

		let mut handle = EntityHandle::<R>::new(obj, internal_id,);

		for f in self.post_creation_functions {
			f(&mut handle,);
		}

		if let Some(domain) = domain.clone() {
			for f in self.listens_to {
				f(domain.clone(), handle.clone())
			}
		}

		if let Some(domain) = domain {
			if let Some(listener) = domain.write_sync().deref().get_listener() {
				handle.read_sync().deref().call_listeners(listener, handle.clone()).await;
			}
		}

		Some(handle)
    }
}
