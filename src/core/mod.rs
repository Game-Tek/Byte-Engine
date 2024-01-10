pub mod orchestrator;

pub mod entity;
pub mod property;

pub use entity::Entity;
pub use entity::EntityHandle;

pub use orchestrator::Orchestrator;

pub fn spawn<E: Entity>(orchestrator_handle: orchestrator::OrchestratorHandle, entity: impl IntoHandler<(), E>) -> EntityHandle<E> {
	entity.call(orchestrator_handle,).unwrap()
}

/// Handles extractor pattern for most functions passed to the orchestrator.
pub trait IntoHandler<P, R: Entity> {
	fn call(self, orchestrator_handle: orchestrator::OrchestratorHandle,) -> Option<EntityHandle<R>>;
}

impl <R: Entity + 'static> IntoHandler<(), R> for R {
    fn call(self, orchestrator_handle: orchestrator::OrchestratorHandle,) -> Option<EntityHandle<R>> {
		let internal_id = {
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let obj = std::sync::Arc::new(smol::lock::RwLock::new(self));

		{
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj.clone());
			systems_data.systems_by_name.insert(std::any::type_name::<R>(), internal_id);
		}

		let handle = EntityHandle::<R>::new(obj, internal_id, 0);

		{
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut listeners = orchestrator.listeners_by_class.lock().unwrap();
			let listeners = listeners.entry(std::any::type_name::<R>()).or_insert(Vec::new());
			for (internal_id, f) in listeners {
				smol::block_on(f(&orchestrator, orchestrator_handle.clone(), *internal_id, handle.clone()));
			}
		}


		Some(handle)
    }
}

impl <R: Entity + 'static> IntoHandler<(), R> for orchestrator::EntityReturn<'_, R> {
    fn call(self, orchestrator_handle: orchestrator::OrchestratorHandle,) -> Option<EntityHandle<R>> {
		let internal_id = {
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let entity = (self.create)(orchestrator::OrchestratorReference { handle: orchestrator_handle.clone(), internal_id });

		let obj = std::sync::Arc::new(smol::lock::RwLock::new(entity));

		{
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj.clone());
			systems_data.systems_by_name.insert(std::any::type_name::<R>(), internal_id);
		}

		let mut handle = EntityHandle::<R>::new(obj, internal_id, 0);

		{
			

			for f in self.post_creation_functions {
				f(&mut handle, orchestrator::OrchestratorReference { handle: orchestrator_handle.clone(), internal_id });
			}
		}

		{
			for (type_id, f) in self.listens_to {
				let orchestrator = orchestrator_handle.as_ref().borrow();

				let mut listeners = orchestrator.listeners_by_class.lock().unwrap();
			
				let listeners = listeners.entry(type_id).or_insert(Vec::new());

				listeners.push((internal_id, f));
			}
		}

		Some(handle)
    }
}