use crate::core::{domain::Domain, entity::EntityTrait, listener::{BasicListener, EntitySubscriber, Listener}, Entity, EntityHandle};

pub struct Space {
	listener: BasicListener,
}

impl Space {
	pub fn new() -> Self {
		Space {
			listener: BasicListener::new(),
		}
	}
}

impl Domain for Space {

}

impl Listener for Space {
	fn invoke_for<T: Entity  +?Sized + 'static>(&self, handle: EntityHandle<T>, reference: &T) {
		self.listener.invoke_for(handle, reference);
	}

	fn add_listener<T: Entity + ?Sized + 'static>(&self, listener: EntityHandle<dyn EntitySubscriber<T>>) {
		self.listener.add_listener::<T>(listener);
	}
}

impl Entity for Space {
	fn get_listener(&self) -> Option<&BasicListener> {
		Some(&self.listener)
	}
}