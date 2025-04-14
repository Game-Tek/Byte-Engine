use std::future::Future;

use super::{Entity, EntityHandle,};

/// A property-like object is an object that can be subscribed to and that has a value.
pub trait PropertyLike<T> {
	/// Adds a subscriber to the property object.
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>);

	fn get<'a>(&'a self) -> T where T: Clone;
}

struct PropertyState<T> {
	subscribers: Vec<std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>>,
}

/// A property is a piece of data that can be read and written, that signals when it is written to.
pub struct Property<T> {
	value: T,
	internal_state: std::rc::Rc<std::sync::RwLock<PropertyState<T>>>,
}

impl <T: Clone + 'static> Default for Property<T> where T: Default {
	fn default() -> Self {
		Self::new(T::default())
	}
}

impl <T: Clone + 'static> Property<T> {
	/// Creates a new property with the given value.
	pub fn new(value: T) -> Self {
		Self {
			internal_state: std::rc::Rc::new(std::sync::RwLock::new(PropertyState { subscribers: Vec::new() })),
			value,
		}
	}

	pub fn link_to<E: Entity>(&mut self, handle: EntityHandle<E>, destination_property: fn(&mut E, &T)) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(std::rc::Rc::new(std::sync::RwLock::new((handle, destination_property))));
	}

	pub fn get(&self) -> T where T: Copy {
		self.value
	}

	/// Sets the value of the property.
	pub fn set(&mut self, setter: impl FnOnce(&T) -> T) {
		self.value = setter(&self.value);

		let mut internal_state = self.internal_state.write().unwrap();

		for subscriber in &mut internal_state.subscribers {
			let mut subscriber = subscriber.write().unwrap();
			subscriber.update(&self.value);
		}
	}

	/// Adds a subscriber to the property.
	pub fn add<F>(&self, get: F) where F: FnMut(&T) + 'static {
		self.internal_state.write().unwrap().subscribers.push(std::rc::Rc::new(std::sync::RwLock::new(get)));
		// self.internal_state.write().unwrap().subscribers.push(std::rc::Rc::new(std::sync::RwLock::new(async move |e| { get(e) })));
	}

	// pub fn add_async<R>(&self, get: impl FnMut(&T) -> R + 'static) where R: Future<Output = ()> {
	// 	// self.internal_state.write().unwrap().subscribers.push(std::rc::Rc::new(std::sync::RwLock::new(Box::new(get))));
	// 	self.internal_state.write().unwrap().subscribers.push(std::rc::Rc::new(std::sync::RwLock::new(get)));
	// }
}

impl <T: 'static> Entity for Property<T> {}

/// A derived property is a property that has no value of its own, but is derived from another property.
pub struct DerivedProperty<F, T> {
	internal_state: std::rc::Rc<std::sync::RwLock<DerivedPropertyState<F, T>>>,
}

struct DerivedPropertyState<F, T> {
	value: T,
	deriver: fn(&F) -> T,
	subscribers: Vec<std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>>,
}

impl <F, T> Subscriber<F> for DerivedPropertyState<F, T> {
	fn update(&mut self, value: &F) {
		self.value = (self.deriver)(value);

		for receiver in &mut self.subscribers {
			let mut receiver = receiver.write().unwrap();
			receiver.update(&self.value);
		}
	}
}

impl <F: Clone + 'static, T: Clone + 'static> DerivedProperty<F, T> {
	pub fn new(source_property: &mut Property<F>, deriver: fn(&F) -> T) -> Self {
		let h = std::rc::Rc::new(std::sync::RwLock::new(DerivedPropertyState { subscribers: Vec::new(), value: deriver(&source_property.value), deriver }));

		source_property.add_subscriber(h.clone());

		Self {
			internal_state: h,
		}
	}

	pub fn link_to<S: Entity>(&mut self, subscriber: &SinkProperty<T>) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(subscriber.internal_state.clone());
	}

	pub fn get(&self) -> T {
		let internal_state = self.internal_state.read().unwrap();
		internal_state.value.clone()
	}
}

/// A sink property is a property that has no value of its own, but just consumes/copies the value of another property.
pub struct SinkProperty<T> {
	internal_state: std::rc::Rc<std::sync::RwLock<SinkPropertyState<T>>>,
}

pub struct SinkPropertyState<T> {
	value: T,
}

impl <T: Clone + 'static> Subscriber<T> for SinkPropertyState<T> {
	fn update(&mut self, value: &T) {
		self.value = value.clone();
	}
}

impl <T: Clone + 'static> SinkProperty<T> {
	pub fn new(source_property: &mut impl PropertyLike<T>) -> Self {
		let internal_state = std::rc::Rc::new(std::sync::RwLock::new(SinkPropertyState { value: source_property.get() }));

		source_property.add_subscriber(internal_state.clone());

		Self {
			internal_state: internal_state.clone(),
		}
	}

	pub fn from_derived<F: Clone + 'static>(source_property: &mut DerivedProperty<F, T>) -> Self {
		let internal_state = std::rc::Rc::new(std::sync::RwLock::new(SinkPropertyState { value: source_property.get() }));

		let mut source_property_internal_state = source_property.internal_state.write().unwrap();
		source_property_internal_state.subscribers.push(internal_state.clone());

		Self {
			internal_state: internal_state.clone(),
		}
	}

	pub fn get(&self) -> T {
		let internal_state = self.internal_state.read().unwrap();
		internal_state.value.clone()
	}
}

/// A subscriber is an object that can be notified of changes.
pub trait Subscriber<T> {
	fn update<'a>(&mut self, value: &'a T);
}

impl <T: Clone + 'static> PropertyLike<T> for Property<T> {
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(subscriber);
	}

	fn get<'a>(&'a self) -> T where T: Clone { self.value.clone() }
}

impl <F: Clone + 'static, T: Clone + 'static> PropertyLike<T> for DerivedProperty<F, T> {
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(subscriber);
	}

	fn get<'a>(&'a self) -> T where T: Clone {
		let internal_state = self.internal_state.read().unwrap();
		internal_state.value.clone()
	}
}

impl <E, T, F> Subscriber<T> for (EntityHandle<E>, F) where F: FnMut(&mut E, &T) {
	fn update(&mut self, value: &T) {
		let mut entity = self.0.write();
		(self.1)(&mut entity, value);
	}
}

impl <T, F> Subscriber<T> for F where F: FnMut(&T) + 'static {
	fn update(&mut self, value: &T) {
		(self)(value);
	}
}

// impl <T, F, R> Subscriber<T> for F where F: FnMut(&T) -> R + 'static, R: Future<Output = ()> {
// 	fn update(&mut self, value: &T) {
// 		(self)(value);
// 	}
// }

// impl <T, R> Subscriber<T> for Box<dyn FnMut(&T) -> R> where R: Future<Output = ()> + 'static {
// 	fn update(&mut self, value: &T) {
// 		(self)(value);
// 	}
// }

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use crate::core::spawn;

	use super::*;

	#[test]
	fn reactivity() {
		struct SourceComponent {
			value: Property<u32>,
			derived: DerivedProperty<u32, String>,
		}

		struct ReceiverComponent {
			value: SinkProperty<u32>,
			derived: SinkProperty<String>,
		}

		impl Entity for SourceComponent {}

		impl Entity for ReceiverComponent {}

		let mut value = Property::new(1);
		let derived = DerivedProperty::new(&mut value, |value| value.to_string());

		let source_component_handle: EntityHandle<SourceComponent> = spawn(SourceComponent { value, derived });
		let receiver_component_handle: EntityHandle<ReceiverComponent> = spawn(ReceiverComponent { value: source_component_handle.map(|c| { let mut c = c.write(); SinkProperty::new(&mut c.value) }), derived: source_component_handle.map(|c| { let mut c = c.write(); SinkProperty::from_derived(&mut c.derived) })});

		assert_eq!(source_component_handle.map(|c| { let c = c.read(); c.value.get() }), 1);
		assert_eq!(source_component_handle.map(|c| { let c = c.read(); c.derived.get() }), "1");
		assert_eq!(receiver_component_handle.map(|c| { let c = c.read(); c.value.get() }), 1);
		assert_eq!(receiver_component_handle.map(|c| { let c = c.read(); c.derived.get() }), "1");

		source_component_handle.map(|c| { let mut c = c.write(); c.value.set(|_| 2) });

		assert_eq!(source_component_handle.map(|c| { let c = c.read(); c.value.get() }), 2);
		assert_eq!(source_component_handle.map(|c| { let c = c.read(); c.derived.get() }), "2");
		assert_eq!(receiver_component_handle.map(|c| { let c = c.read(); c.value.get() }), 2);
		assert_eq!(receiver_component_handle.map(|c| { let c = c.read(); c.derived.get() }), "2");
	}
}
