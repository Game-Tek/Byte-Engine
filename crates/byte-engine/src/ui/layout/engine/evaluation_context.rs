//! Evaluation context and component-construction implementations.

use std::fmt::Display;

use super::{
	properties::{Setup, Spares, assert_rgba_len, declare, update},
	*,
};
use crate::ui::{
	animation::{Animatable, Animation, Motion, Values},
	components::{image::ImageSource, path::Path},
	layout::context::slot_path,
};

/// The `EvaluationContext` struct lets a mounted asynchronous UI component declare and edit its part of the tree.
///
/// It holds no engine state, only the identifiers that place its declarations. Every declaration, edit, and change is
/// a future that completes on its first poll, writing straight into the engine the task is polled by; reads such as
/// [`Context::geometry`] and [`Context::with`] complete the same way. Get one from [`Engine::mount`] or from an
/// [`ElementSlot`] of another context.
pub struct EvaluationContext<C = ()> {
	/// The element this context edits, or for a component scope the element it was declared under.
	pub(super) id: Id,
	/// The element declarations from this context attach under.
	pub(super) parent: Option<Id>,
	/// The path slots declared from this context are keyed under; see [`slot_path`].
	pub(super) path: u64,
	/// The mounted scope that owns tasks spawned from this context.
	pub(super) owner: ScopeId,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C: 'static> EvaluationContext<C> {
	pub(super) fn new(id: Id, parent: Option<Id>, path: u64, owner: ScopeId) -> Self {
		Self {
			id,
			parent,
			path,
			owner,
			ctx: PhantomData,
		}
	}

	/// Makes the context of the root component, which declares top-level elements.
	pub(super) fn new_root() -> Self {
		// The root context edits no element; its id only names the root for focus and events.
		Self::new(Id::MIN, None, ROOT_PATH, ScopeId::ROOT)
	}

	/// Returns a future that applies `command` on its first poll, so what the component does next already sees it.
	fn change(command: UiCommand) -> impl Future<Output = ()> {
		direct(move |poll: &mut UiPoll<C>| poll.apply(command))
	}

	/// Moves this element under another parent as its last child, keeping its
	/// id, path, and properties. Await it. The new parent's flow lays it out from
	/// the next frame. An unknown parent, or a parent that is this element or one
	/// of its descendants, is logged and changes nothing.
	pub fn reparent(&mut self, parent: Id) -> impl Future<Output = ()> {
		Self::change(UiCommand::Reparent { child: self.id, parent })
	}

	/// Moves another retained element under this element as its last child, so a
	/// drop target can take in the source it was given. Await it. An unknown
	/// element, or one that is this element or one of its ancestors, is logged and
	/// changes nothing.
	pub fn adopt(&mut self, child: Id) -> impl Future<Output = ()> {
		Self::change(UiCommand::Reparent { child, parent: self.id })
	}

	/// Removes this element, everything declared under it, and the components started
	/// there. Await it.
	///
	/// Awaited in a component's own context, it removes that component's elements
	/// and ends the component, so return right after. The context stays usable only
	/// to declare again: a later declaration of the same key under the same parent
	/// gets the same id. A mounted component awaited from outside the removed element
	/// keeps running until its own future ends; its elements are gone, so its updates
	/// change nothing.
	pub fn remove(&mut self) -> impl Future<Output = ()> {
		Self::change(UiCommand::Remove {
			path: self.path,
			owner: None,
		})
	}

	/// Edits this container with `setup`. Await it. The engine compares the container before and after the whole
	/// edit, so writing values it already has changes nothing.
	pub fn update_container(&mut self, setup: impl Setup<Container>) -> impl Future<Output = ()> {
		update::<C, Container>(self.id, setup)
	}

	/// Runs `animation` on this container: chain the callbacks that write its values, such as
	/// [`Animate::on_every_frame`], then await the run. It resolves once the animation settles; drop it to interrupt
	/// the run, and the animation keeps where it stands and how it moves for the next one.
	///
	/// Next, set where the animation is headed with [`Animation::set_target`] and run it again.
	pub fn animate<'a, V: Animatable + 'static, M: Motion>(&self, animation: &'a mut Animation<V, M>) -> Animate<'a, C, V, M> {
		Animate {
			target: self.id,
			animation,
			start: None,
			frame: None,
			render: None,
			started: false,
			complete: false,
			end: EndGuard::new(self.id, None),
		}
	}

	/// Hands `animation` to the engine, which moves it on every frame until it settles and calls `frame` with each
	/// frame's values, to edit the elements it drives through [`AnimationFrame::update_container`]. Await it for the
	/// handle that heads it somewhere; dropping the handle stops it.
	///
	/// Prefer it over [`Self::animate`] for motion a loop retargets as things change, such as a drawer that follows
	/// whether it's open: the handle needs no branch in the loop's `select!`. Next, call
	/// [`AnimationHandle::set_target`], and await [`AnimationHandle::settled`] where what follows needs it at rest.
	pub fn animation<V: Animatable + 'static, M: Motion + 'static>(
		&self,
		animation: Animation<V, M>,
		frame: impl FnMut(&mut AnimationFrame<'_>, Values<V>) + 'static,
	) -> impl Future<Output = AnimationHandle<C, V, M>> {
		super::animations::register::<C, V, M>(animation, frame)
	}

	/// Edits this text element or text field. See [`Self::update_container`].
	pub fn update_text(&mut self, setup: impl Setup<Text>) -> impl Future<Output = ()> {
		update::<C, Text>(self.id, setup)
	}

	/// Edits a retained curve in place, such as re-routing a wire while its ends move.
	///
	/// Changing only the segments keeps the layout placement, and re-routing with
	/// [`Properties::clear_segments`] reuses the segment storage. Changing its size or
	/// transform replays placement like any other element edit.
	pub fn update_curve(&mut self, setup: impl Setup<Curve>) -> impl Future<Output = ()> {
		update::<C, Curve>(self.id, setup)
	}

	/// Edits a retained path in place. Replacing its outline repacks it; a style edit repaints only.
	pub fn update_path(&mut self, setup: impl Setup<Path>) -> impl Future<Output = ()> {
		update::<C, Path>(self.id, setup)
	}

	/// Edits this image. See [`Self::update_container`].
	pub fn update_image(&mut self, setup: impl Setup<Image>) -> impl Future<Output = ()> {
		update::<C, Image>(self.id, setup)
	}

	/// Declares one `component` per item of the list `items` borrows from the application context, keyed by each item's
	/// own hash, such as a row per entity handle in a list of handles.
	///
	/// The list is read again whenever the application context is written, without allocating; an item is cloned only
	/// when its key is new. An item with a new key gets its component,
	/// given the item; one whose key left the list loses its component and every element it declared; one that stays
	/// keeps its component untouched. The elements each component declares directly under this element follow the
	/// list's order. Equal items are one item. Use [`Self::each_by`] for items that don't hash, or whose identity isn't
	/// their whole value. See [Collections](/docs/reference/gui#collections).
	///
	/// Every item's component owns a copy of `component`, and the list's read owns copies of `items` and the key, so the
	/// closures must be [`Clone`]: capture handles and plain values rather than state.
	pub fn each<T>(
		&mut self,
		items: impl Fn(&C) -> &[T] + Clone + 'static,
		component: impl AsyncFn(&mut EvaluationContext<C>, T) + Clone + 'static,
	) -> impl Future<Output = ()>
	where
		T: std::hash::Hash + Clone + 'static,
	{
		self.keyed(items, hash_of::<T>, component)
	}

	/// Declares one `component` per item of the list `items` borrows from the application context, keyed by what `key`
	/// returns for the item, such as a toast's id; see [`Self::each`].
	///
	/// `key` is called for every item each time the list is read. An item keeps its component while its key stays in
	/// the list, so a component that shows data that changes under one key reads it by that key.
	pub fn each_by<T, K>(
		&mut self,
		items: impl Fn(&C) -> &[T] + Clone + 'static,
		key: impl Fn(&T) -> K + Clone + 'static,
		component: impl AsyncFn(&mut EvaluationContext<C>, T) + Clone + 'static,
	) -> impl Future<Output = ()>
	where
		T: Clone + 'static,
		K: std::hash::Hash,
	{
		self.keyed(items, move |item: &T| hash_of(&key(item)), component)
	}

	/// Starts the task that keeps one component per key of `items`, keyed by `key`, under this element.
	fn keyed<T: Clone + 'static>(
		&mut self,
		items: impl Fn(&C) -> &[T] + Clone + 'static,
		key: impl Fn(&T) -> u64 + Clone + 'static,
		component: impl AsyncFn(&mut EvaluationContext<C>, T) + Clone + 'static,
	) -> impl Future<Output = ()> {
		self.element(ElementKey::hashed(COLLECTION_KEY))
			.component(async move |ctx| follow_collection(ctx, items, key, component).await)
	}

	fn read<T>(&self, read: fn(&Runtime, Id) -> T) -> Read<C, T> {
		Read {
			target: self.id,
			read,
			ctx: PhantomData,
		}
	}
}

impl<C: 'static> Context<C> for EvaluationContext<C> {
	fn id(&self) -> Id {
		self.id
	}

	fn with<F, T>(&self, read: F) -> With<C, F>
	where
		F: FnOnce(&mut C) -> T,
	{
		With {
			read: Some(read),
			ctx: PhantomData,
		}
	}

	fn element<'a>(&'a mut self, key: impl Into<ElementKey>) -> ElementSlot<'a, C> {
		ElementSlot {
			parent: self,
			key: key.into(),
		}
	}

	fn render(&mut self) -> RenderFuture<C> {
		RenderFuture {
			wait: None,
			complete: false,
			ctx: PhantomData,
		}
	}

	fn geometry(&self) -> Read<C, Option<ElementGeometry>> {
		self.read(|runtime, id| runtime.geometry.get(&id).map(|published| published.geometry))
	}

	fn extent_change(&self, known: LayoutExtent) -> ExtentChangeFuture<C> {
		ChangeFuture::new(self.id, known)
	}

	fn pointer(&self) -> Read<C, PointerState> {
		self.read(|runtime, _| runtime.pointer)
	}

	fn drag(&self) -> Read<C, Option<DragCapture>> {
		self.read(|runtime, _| runtime.drag.capture())
	}

	fn request_focus(&mut self) -> impl Future<Output = ()> {
		Self::change(UiCommand::Focus {
			target: self.id,
			focused: true,
		})
	}

	fn release_focus(&mut self) -> impl Future<Output = ()> {
		Self::change(UiCommand::Focus {
			target: self.id,
			focused: false,
		})
	}

	fn focus_change(&self, known: bool) -> FocusChangeFuture<C> {
		ChangeFuture::new(self.id, known)
	}

	fn remove(&mut self) -> impl Future<Output = ()> {
		EvaluationContext::remove(self)
	}
}

impl<C: 'static> ElementContext<C> for ElementSlot<'_, C> {
	fn container(self, setup: impl Setup<Container>) -> impl Future<Output = EvaluationContext<C>> {
		declare(self, |_, _| Primitives::Container(Container::default()), setup)
	}

	fn text(self, content: impl Display, setup: impl Setup<Text>) -> impl Future<Output = EvaluationContext<C>> {
		declare(
			self,
			move |_, spares| Primitives::Text(Text::new(spares.format(content), false)),
			setup,
		)
	}

	fn text_field(self, content: impl Display, setup: impl Setup<Text>) -> impl Future<Output = EvaluationContext<C>> {
		declare(
			self,
			move |_, spares| Primitives::Text(Text::new(spares.format(content), true)),
			setup,
		)
	}

	fn curve(self, setup: impl Setup<Curve>) -> impl Future<Output = EvaluationContext<C>> {
		let create = |_, spares: &mut Spares| {
			let mut curve = Curve::new();
			curve.path.segments = spares.segments();
			Primitives::Curve(curve)
		};
		declare(self, create, setup)
	}

	fn image(
		self,
		width: u32,
		height: u32,
		source: impl Into<ImageSource>,
		setup: impl Setup<Image>,
	) -> impl Future<Output = EvaluationContext<C>> {
		let create = move |content_id, _: &mut Spares| {
			let source = source.into();
			assert_rgba_len(width, height, &source);
			Primitives::Image(Image::new(content_id, width, height, source))
		};
		declare(self, create, setup)
	}

	fn path(self, setup: impl Setup<Path>) -> impl Future<Output = EvaluationContext<C>> {
		let create = |content_id, spares: &mut Spares| {
			let mut path = Path::new(content_id);
			path.path.segments = spares.segments();
			Primitives::Path(path)
		};
		declare(self, create, setup)
	}

	fn component(self, component: impl AsyncFnOnce(&mut EvaluationContext<C>) + 'static) -> impl Future<Output = ()> {
		let EvaluationContext {
			id,
			parent,
			path: declared_in,
			owner,
			..
		} = *self.parent;
		let path = slot_path(declared_in, self.key).get();
		// The task belongs to the enclosing mounted scope and ends when that scope is removed,
		// or earlier when the element it was declared under is removed.
		direct(move |poll: &mut UiPoll<C>| {
			poll.tree.declare_scope(path, declared_in);
			let mut ctx = EvaluationContext::new(id, parent, path, owner);
			// The runtime owns the context through this future; the component's borrow of it never escapes it.
			poll.runtime
				.spawn(owner, path, Box::pin(async move { component(&mut ctx).await }));
		})
	}

	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T, C>
	where
		F: AsyncFnOnce(&mut EvaluationContext<C>) -> T + 'static,
	{
		MountedComponentFuture {
			component: Some(component),
			future: None,
			id: self.parent.id,
			parent: self.parent.parent,
			parent_path: self.parent.path,
			path: slot_path(self.parent.path, self.key).get(),
			scope: None,
			complete: false,
			output: PhantomData,
		}
	}
}

impl<C: 'static> super::super::context::ContainerContext<C> for EvaluationContext<C> {
	fn on(&mut self, event: Events) -> EventFuture<C> {
		InputFuture::new(self.id, event)
	}

	fn on_key(&mut self, key: Key) -> KeyFuture<C> {
		InputFuture::new(self.id, key)
	}

	fn on_text_edit(&mut self) -> TextEditFuture<C> {
		InputFuture::new(self.id, ())
	}
}

/// The slot a collection's task takes under its element, which no name hashes to in practice.
const COLLECTION_KEY: u64 = 0xC011_EC71_0000_0001;

/// Hashes `value` the same way in every run, for a collection item's key.
fn hash_of<T: std::hash::Hash + ?Sized>(value: &T) -> u64 {
	use std::hash::Hasher;
	let mut hasher = utils::hash::FxHasher::default();
	value.hash(&mut hasher);
	hasher.finish()
}

/// Keeps one `component` per key of `items` under `ctx`'s element, for as long as the collection's task runs.
///
/// It wakes when the sequence of keys changes, then removes the scopes of keys that left, ranks every item's scope in
/// list order, which the tree keeps its elements in, and declares a component for each new key.
async fn follow_collection<C: 'static, T: Clone + 'static>(
	ctx: &mut EvaluationContext<C>,
	items: impl Fn(&C) -> &[T] + Clone + 'static,
	key: impl Fn(&T) -> u64 + Clone + 'static,
	component: impl AsyncFn(&mut EvaluationContext<C>, T) + Clone + 'static,
) {
	use std::hash::Hasher;
	// One hash of every key in order: the collection wakes only when that sequence changes, and reading it allocates
	// nothing.
	// The read owns its own copies of the closures, since it outlives every borrow of this task's locals.
	let mut sequence = {
		let (items, key) = (items.clone(), key.clone());
		ctx.proactive(move |app: &C| {
			let mut hasher = utils::hash::FxHasher::default();
			for item in items(app) {
				hasher.write_u64(key(item));
			}
			hasher.finish()
		})
	};
	let (collection, list) = (ctx.path, ctx.id);
	let item_path = move |key: u64| slot_path(collection, ElementKey::hashed(key)).get();
	// The keys shown now, and buffers reused by every change.
	let mut shown: utils::hash::HashSet<u64> = Default::default();
	let mut listed: utils::hash::HashSet<u64> = Default::default();
	let mut order: Vec<u64> = Vec::new();
	let mut fresh: Vec<(u64, T)> = Vec::new();
	loop {
		sequence.next().await;
		// The keys in list order, and the items whose keys are new.
		(order, fresh, listed) = {
			let (items, key, shown) = (&items, &key, &shown);
			ctx.with(move |app: &mut C| {
				listed.clear();
				for item in items(app) {
					let key = key(item);
					if !listed.insert(key) {
						log::error!(
							"A UI collection skipped an item whose key another item already has. The most likely cause is equal items in a list given to `each`, or a key function for `each_by` that isn't unique. Give each item a distinct key, such as an id."
						);
						continue;
					}
					order.push(key);
					if !shown.contains(&key) {
						fresh.push((key, item.clone()));
					}
				}
				(order, fresh, listed)
			})
			.await
		};
		for &gone in shown.iter().filter(|key| !listed.contains(key)) {
			EvaluationContext::<C>::change(UiCommand::Remove {
				path: item_path(gone),
				owner: None,
			})
			.await;
		}
		shown.retain(|key| listed.contains(key));
		// Ranks come before the new items' declarations, so each one lands in its place as it's declared.
		direct(|poll: &mut UiPoll<C>| {
			let ranks = order.iter().enumerate().map(|(rank, &key)| (item_path(key), rank as u32));
			poll.tree.set_ranks(list, ranks);
		})
		.await;
		order.clear();
		for (key, item) in fresh.drain(..) {
			// Each item's component is its own task, so it owns a copy of the closure.
			let component = component.clone();
			ctx.element(ElementKey::hashed(key))
				.component(async move |ctx| component(ctx, item).await)
				.await;
			shown.insert(key);
		}
	}
}
