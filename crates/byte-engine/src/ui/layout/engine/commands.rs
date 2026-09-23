//! Changes components send to their engine, and how the engine applies them.
//!
//! Every [`EvaluationContext`] holds a [`std::sync::mpsc::Sender`] of [`UiCommand`]s and the engine owns the
//! matching [`Receiver`]. A write made by a component, such as declaring an element or updating one, is sent right
//! away and applied by [`apply_commands`] after the task poll that made it. A task sends its commands in program
//! order and the channel is first in, first out, so siblings keep the order they were declared in and an update
//! never overtakes the creation of its element.

use std::sync::mpsc::Receiver;

use super::*;

/// The `UiCommand` enum is one change a component asked its engine to make to the retained tree or runtime.
///
/// Contexts send these through the engine's channel instead of borrowing the tree, so a component never holds the
/// engine's state. Next, see [`apply_commands`], which applies them between task polls.
pub(super) enum UiCommand {
	/// Declares an element with its initial properties. An element that already exists keeps its properties.
	Create {
		parent: Option<Id>,
		/// The path of the context that declared the element, which owns it for removal.
		declared_in: u64,
		id: Id,
		/// Boxed so every command stays small: the channel stores commands in blocks sized by the largest one.
		element: Box<ConcreteElement>,
	},
	/// Edits an element's primitive. The edit returns false when the element is of another kind.
	Update {
		id: Id,
		update: Box<dyn FnOnce(&mut Primitives) -> bool>,
	},
	/// Removes everything declared under `path` with the tasks declared there, or, with `owner`, the tasks that
	/// mounted scope owns instead. Two live mounts can share a path, so a mounted scope ends by its owner.
	Remove {
		path: u64,
		owner: Option<ScopeId>,
	},
	/// Moves `child` under `parent` as its last child.
	Reparent {
		child: Id,
		parent: Id,
	},
	/// Puts `target` on top of the focus stack, or takes it off.
	Focus {
		target: Id,
		focused: bool,
	},
	/// Records that the component scope `path` was declared in `declared_in`, so removing an ancestor ends it, and
	/// starts the scope's task when it is a spawned component.
	DeclareScope {
		path: u64,
		declared_in: u64,
		task: Option<(ScopeId, BoxedUiFuture)>,
	},
}

/// Applies every command received so far, including the ones that applying them sends.
///
/// Removing a scope drops the futures of the tasks declared in it, and a dropped mounted component sends the command
/// that ends its own scope. Those commands land in the same channel and are applied before this returns.
pub(super) fn apply_commands(commands: &Receiver<UiCommand>, runtime: &mut Runtime, tree: &mut RetainedTree) {
	while let Ok(command) = commands.try_recv() {
		apply(command, runtime, tree);
	}
}

/// Applies one command. Invalid requests, such as an edit of a removed element, are logged and skipped.
fn apply(command: UiCommand, runtime: &mut Runtime, tree: &mut RetainedTree) {
	match command {
		UiCommand::Create {
			parent,
			declared_in,
			id,
			element,
		} => tree.add_element(parent, declared_in, id, *element),
		UiCommand::Update { id, update } => match tree.update_element(id, update) {
			Some(true) => {}
			Some(false) => log::error!(
				"A UI element update was skipped because the element is of another kind. The most likely cause is calling an `update_*` method that does not match the element the context declared."
			),
			// A component awaited from outside a removed element keeps running and may still edit its elements.
			None => log::debug!("A UI element update was skipped because the element was removed."),
		},
		UiCommand::Remove { path, owner } => {
			let removed = tree.remove_scope(path);
			if !removed.is_empty() {
				runtime.remove_targets(removed);
			}
			// Dropped futures send their own cleanup through the channel, which the caller keeps draining.
			drop(runtime.detach_tasks(|task| match owner {
				Some(owner) => task.owner == owner,
				None => tree.path_is_under(task.path, path),
			}));
		}
		UiCommand::Reparent { child, parent } => tree.reparent(child, parent),
		UiCommand::Focus { target, focused: true } => runtime.request_focus(target),
		UiCommand::Focus { target, focused: false } => runtime.release_focus(target),
		UiCommand::DeclareScope {
			path,
			declared_in,
			task,
		} => {
			tree.declare_scope(path, declared_in);
			if let Some((owner, future)) = task {
				runtime.spawn(owner, path, future);
			}
		}
	}
}
