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
	fn invoke_for<T: Entity + 'static>(&self, handle: EntityHandle<T>) {
		self.listener.invoke_for(handle);
	}

	fn invoke_for_trait<T: Entity + 'static>(&self, handle: EntityHandle<T>, r#type: EntityTrait) { self.listener.invoke_for_trait(handle, r#type); }

	fn add_listener<L, T: Entity + 'static>(&self, listener: EntityHandle<L>) where L: EntitySubscriber<T> + 'static {
		self.listener.add_listener::<L, T>(listener);
	}
}

impl Entity for Space {
	fn get_listener(&self) -> Option<&BasicListener> {
		Some(&self.listener)
	}
}