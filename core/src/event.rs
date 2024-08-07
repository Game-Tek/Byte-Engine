use super::{property::Subscriber, EntityHandle};

/// Trait for an event-like object.
/// Allows an event object to be subscribed to and to be triggered.
pub trait EventLike<T> {
	/// Subscribes a consumer to the event.
	///	
	/// # Arguments
	/// * `consumer` - The consumer to be subscribed.
	/// * `endpoint` - The function to be called when the event is triggered.
	fn subscribe<C: 'static>(&mut self, consumer: EntityHandle<C>, endpoint: fn(&mut C, &T));

	/// Triggers the event.
	/// Most implmentations will call the endpoint function for each of the consumers.
	/// 
	/// # Arguments
	/// * `value` - The value to be passed to the consumers.
	fn ocurred<'a>(&self, value: &'a T);
}

pub struct Event<T> {
	subscribers: Vec<std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>>,
}

impl <T: 'static> EventLike<T> for Event<T> {
	fn subscribe<C: 'static>(&mut self, consumer: EntityHandle<C>, endpoint: fn(&mut C, &T)) {
		self.subscribers.push(std::rc::Rc::new(std::sync::RwLock::new((consumer, endpoint))));
	}

	fn ocurred(&self, value: &T) {
		for subscriber in &self.subscribers {
			let mut subscriber = subscriber.write().unwrap();
			subscriber.update(value);
		}
	}
}

impl <T> Default for Event<T> {
	fn default() -> Self {
		Self {
			subscribers: Vec::new(),
		}
	}
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use utils::r#async::block_on;

use crate::{entity::EntityBuilder, spawn, Entity};

	use super::*;

	#[test]
	fn events() {
		struct MyComponent {
			name: String,
			value: u32,
			click: bool,

			event: Event<bool>,
		}

		impl MyComponent {
			pub fn set_click(&mut self, value: bool) {
				self.click = value;

				self.event.ocurred(&self.click);
			}

			pub fn click(&mut self) -> &mut Event<bool> { &mut self.event }
		}

		impl Entity for MyComponent {}

		struct MySystem {

		}

		impl Entity for MySystem {}

		static mut COUNTER: u32 = 0;

		impl MySystem {
			fn new<'c>(_: &EntityHandle<MyComponent>) -> EntityBuilder<'c, MySystem> {
				EntityBuilder::new(MySystem {})
			}

			fn on_event(&mut self, _: &bool) {
				unsafe {
					COUNTER += 1;
				}
			}
		}

		let component_handle: EntityHandle<MyComponent> = block_on(spawn(MyComponent { name: "test".to_string(), value: 1, click: false, event: Default::default() }));

		let system_handle: EntityHandle<MySystem> = block_on(spawn(MySystem::new(&component_handle)));

		component_handle.map(|c| {
			let mut c = c.write_sync();
			c.click().subscribe(system_handle.clone(), MySystem::on_event);
		});

		assert_eq!(unsafe { COUNTER }, 0);

		component_handle.map(|c| {
			let mut c = c.write_sync();
			c.set_click(true);
		});

		assert_eq!(unsafe { COUNTER }, 1);
	}
}