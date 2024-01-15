use crate::core::{domain::Domain, listener::{Listener, BasicListener}, EntityHandle, orchestrator::EntitySubscriber};

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
	fn invoke_for<T: 'static>(&self, handle: EntityHandle<T>) {
		self.listener.invoke_for(handle);
	}

	fn add_listener<L, T: 'static>(&self, listener: EntityHandle<L>) where L: EntitySubscriber<T> + 'static {
		self.listener.add_listener::<L, T>(listener);
	}
}