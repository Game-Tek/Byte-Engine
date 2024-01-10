use super::{Entity, EntityHandle, orchestrator::EventDescription};

struct PropertyState<T> {
	subscribers: Vec<std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>>,
}

pub trait PropertyLike<T> {
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>);
	fn get_value(&self) -> T;
}

/// A property is a piece of data that can be read and written, that signals when it is written to.
pub struct Property<T> {
	value: T,
	internal_state: std::rc::Rc<std::sync::RwLock<PropertyState<T>>>,
}

impl <T: Clone + 'static> Property<T> {
	/// Creates a new property with the given value.
	pub fn new(value: T) -> Self {
		Self {
			internal_state: std::rc::Rc::new(std::sync::RwLock::new(PropertyState { subscribers: Vec::new() })),
			value,
		}
	}

	pub fn link_to<S: Entity>(&mut self, handle: EntityHandle<S>, destination_property: fn() -> EventDescription<S, T>) {
		let mut internal_state = self.internal_state.write().unwrap();
		
		// internal_state.receivers.push(Box::new(SinkPropertyReceiver { handle, property: destination_property }));
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
}

pub struct DerivedProperty<F, T> {
	internal_state: std::rc::Rc<std::sync::RwLock<DerivedPropertyState<F, T>>>,
}

struct DerivedPropertyState<F, T> {
	deriver: fn(&F) -> T,
	value: T,
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
		let internal_state = std::rc::Rc::new(std::sync::RwLock::new(SinkPropertyState { value: source_property.get_value() }));

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

trait Subscriber<T> {
	fn update(&mut self, value: &T);
}

impl <T: Clone + 'static> PropertyLike<T> for Property<T> {
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(subscriber);
	}

	fn get_value(&self) -> T {
		self.value.clone()
	}
}

impl <F: Clone + 'static, T: Clone + 'static> PropertyLike<T> for DerivedProperty<F, T> {
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(subscriber);
	}

	fn get_value(&self) -> T {
		let internal_state = self.internal_state.read().unwrap();
		internal_state.value.clone()
	}
}