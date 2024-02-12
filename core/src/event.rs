use super::{property::Subscriber, Entity, EntityHandle};

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