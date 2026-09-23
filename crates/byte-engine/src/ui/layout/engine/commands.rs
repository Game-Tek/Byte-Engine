//! Structural changes components make to their engine, and how the engine applies them.
//!
//! A component makes one by awaiting a change such as [`EvaluationContext::remove`], which applies it while the
//! component's task is polled, like an element declaration. A mounted component dropped before it ended, as the
//! losing branch of a `select!` is, cannot reach the engine from its destructor, so it sends the removal of its scope
//! through the engine's channel instead. [`UiPoll::apply_commands`] applies those before every direct write and after
//! every task poll, so every change lands in the order it happened.

use super::*;

/// The `UiCommand` enum is one structural change to the retained tree or runtime, such as a removal.
///
/// Components make these through futures such as [`EvaluationContext::remove`], and dropped mounts send them; see
/// the module documentation. Next, see [`apply`], which applies one.
pub(super) enum UiCommand {
	/// Removes everything declared under `path` with the tasks declared there, or, with `owner`, the tasks that
	/// mounted scope owns instead. Two live mounts can share a path, so a mounted scope ends by its owner.
	Remove { path: u64, owner: Option<ScopeId> },
	/// Moves `child` under `parent` as its last child.
	Reparent { child: Id, parent: Id },
	/// Puts `target` on top of the focus stack, or takes it off.
	Focus { target: Id, focused: bool },
}

/// Applies one command. Invalid requests, such as moving a removed element, are logged and skipped.
///
/// Removing a scope drops the futures of the tasks declared in it. A dropped mounted component sends the command that
/// ends its own scope, so the caller drains the channel afterwards.
pub(super) fn apply(command: UiCommand, runtime: &mut Runtime, tree: &mut RetainedTree) {
	match command {
		UiCommand::Remove { path, owner } => {
			let removed = tree.remove_scope(path);
			if !removed.is_empty() {
				runtime.remove_targets(removed);
			}
			runtime.end_tasks(|task| match owner {
				Some(owner) => task.owner == owner,
				None => tree.path_is_under(task.path, path),
			});
		}
		UiCommand::Reparent { child, parent } => tree.reparent(child, parent),
		UiCommand::Focus { target, focused: true } => runtime.request_focus(target),
		UiCommand::Focus { target, focused: false } => runtime.release_focus(target),
	}
}
