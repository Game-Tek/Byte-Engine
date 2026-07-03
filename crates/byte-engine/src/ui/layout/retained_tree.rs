use std::collections::{HashMap, HashSet};

use super::{ConcreteElement, Id, IdedElement, PathSegment};

#[derive(Default)]
pub(super) struct RetainedTree {
	pub(super) elements: Vec<IdedElement>,
	pub(super) element_indices: HashMap<Id, usize>,
	pub(super) relations: Vec<(Id, Id)>,
	pub(super) children_by_parent: HashMap<Id, Vec<Id>>,
	pub(super) parent_by_child: HashMap<Id, Id>,
	path_counts: HashMap<(Option<Id>, &'static str), u32>,
	path_ids: HashMap<Vec<PathSegment>, Id>,
	next_id: u32,
}

impl RetainedTree {
	pub(super) fn new() -> Self {
		Self {
			next_id: 1,
			..Self::default()
		}
	}

	pub(super) fn begin_frame(&mut self) {
		self.path_counts.clear();
	}

	pub(super) fn element_path(
		&mut self,
		parent: Option<Id>,
		parent_path: &[PathSegment],
		name: &'static str,
	) -> Vec<PathSegment> {
		let count = self.path_counts.entry((parent, name)).or_insert(0);
		*count += 1;

		let mut path = Vec::with_capacity(parent_path.len() + 1);
		path.extend_from_slice(parent_path);
		path.push(PathSegment { name, ordinal: *count });
		path
	}

	pub(super) fn scope_path(
		&mut self,
		parent: Option<Id>,
		parent_path: &[PathSegment],
		name: &'static str,
	) -> Vec<PathSegment> {
		self.element_path(parent, parent_path, name)
	}

	pub(super) fn id_for_path(&mut self, path: &[PathSegment]) -> Id {
		if let Some(id) = self.path_ids.get(path) {
			return *id;
		}

		let id = Id::new(self.next_id).expect("UI id counter must stay non-zero");
		self.next_id += 1;
		self.path_ids.insert(path.to_vec(), id);
		id
	}

	pub(super) fn add_element(
		&mut self,
		parent: Option<Id>,
		parent_path: &[PathSegment],
		name: &'static str,
		element: ConcreteElement,
	) -> (Id, Vec<PathSegment>) {
		let path = self.element_path(parent, parent_path, name);
		let id = self.id_for_path(&path);

		if self.element_indices.contains_key(&id) {
			return (id, path);
		}

		self.element_indices.insert(id, self.elements.len());
		self.elements.push(IdedElement {
			id,
			element,
			path: path.clone(),
		});

		if let Some(parent) = parent {
			self.add_relation(parent, id);
		}

		(id, path)
	}

	fn add_relation(&mut self, parent: Id, child: Id) {
		if self.parent_by_child.get(&child) == Some(&parent) {
			return;
		}
		if self.parent_by_child.insert(child, parent).is_none() {
			self.relations.push((parent, child));
			self.children_by_parent.entry(parent).or_default().push(child);
		}
	}

	pub(super) fn element_mut(&mut self, id: Id) -> Option<&mut IdedElement> {
		let index = *self.element_indices.get(&id)?;
		self.elements.get_mut(index)
	}

	pub(super) fn element(&self, id: Id) -> Option<&IdedElement> {
		let index = *self.element_indices.get(&id)?;
		self.elements.get(index)
	}

	pub(super) fn remove_scope(&mut self, scope: &[PathSegment]) -> Vec<Id> {
		if scope.is_empty() {
			return Vec::new();
		}

		let mut removed = HashSet::new();
		self.elements.retain(|element| {
			let should_remove = element.path.starts_with(scope);
			if should_remove {
				removed.insert(element.id);
			}
			!should_remove
		});

		if removed.is_empty() {
			return Vec::new();
		}

		self.relations
			.retain(|(parent, child)| !removed.contains(parent) && !removed.contains(child));
		self.parent_by_child
			.retain(|child, parent| !removed.contains(child) && !removed.contains(parent));
		self.children_by_parent.retain(|parent, children| {
			if removed.contains(parent) {
				return false;
			}
			children.retain(|child| !removed.contains(child));
			!children.is_empty()
		});
		self.rebuild_element_indices();
		removed.into_iter().collect()
	}

	fn rebuild_element_indices(&mut self) {
		self.element_indices.clear();
		for (index, element) in self.elements.iter().enumerate() {
			self.element_indices.insert(element.id, index);
		}
	}
}
