use super::element::{ElementHandle, Id};

/// The `Fetcher` struct provides relationship-aware lookup over a layout element snapshot.
pub struct Fetcher<'a, T> {
	pub elements: &'a [T],
	pub relation_map: &'a [(Id, Id)],
}

/// The `ElementResult` struct provides navigation from one resolved layout element.
pub struct ElementResult<'a, T> {
	element: &'a T,
	fetcher: &'a Fetcher<'a, T>,
}

impl<'a, T: ElementHandle> ElementResult<'a, T> {
	/// Returns the resolved element.
	pub fn element(self) -> &'a T {
		self.element
	}

	pub fn parent(&self) -> Option<ParentResult<'a, T>> {
		self.fetcher
			.relation_map
			.iter()
			.find(|(_, c)| *c == self.element.id())
			.map(|&(id, _)| ParentResult {
				id,
				fetcher: self.fetcher,
			})
	}

	pub fn children(&self) -> ChildrenResult<'a, T> {
		ChildrenResult {
			parent: self.element.id(),
			fetcher: self.fetcher,
		}
	}
}

impl<T: ElementHandle> ElementHandle for ElementResult<'_, T> {
	fn id(&self) -> Id {
		self.element.id()
	}
}

/// The `ParentResult` struct provides safe lookup of an element's recorded parent.
pub struct ParentResult<'a, T> {
	id: Id,
	fetcher: &'a Fetcher<'a, T>,
}

impl<'a, T: ElementHandle> ParentResult<'a, T> {
	/// Returns the parent element when it is present in the fetched snapshot.
	pub fn element(&self) -> Option<ElementResult<'a, T>> {
		self.fetcher.get(self.id)
	}

	/// Returns the recorded parent identity even when its element is absent.
	pub fn id(&self) -> Id {
		self.id
	}
}

/// The `ChildrenResult` struct provides allocation-free traversal of an element's recorded children.
pub struct ChildrenResult<'a, T> {
	parent: Id,
	fetcher: &'a Fetcher<'a, T>,
}

impl<'a, T: ElementHandle> ChildrenResult<'a, T> {
	pub fn elements(&self) -> impl Iterator<Item = ElementResult<'_, T>> + '_ {
		self.fetcher.relation_map.iter().filter_map(|&(parent, child)| {
			if parent == self.parent {
				self.fetcher.get(child)
			} else {
				None
			}
		})
	}

	pub fn ids(&self) -> impl Iterator<Item = Id> + '_ {
		self.fetcher
			.relation_map
			.iter()
			.filter_map(|&(parent, child)| (parent == self.parent).then_some(child))
	}
}

impl<'a, T: ElementHandle> Fetcher<'a, T> {
	pub fn get(&'a self, id: Id) -> Option<ElementResult<'a, T>> {
		self.elements
			.iter()
			.find(|e| e.id() == id)
			.map(|element| ElementResult { element, fetcher: self })
	}
}

#[cfg(test)]
mod tests {
	use std::num::NonZeroU32;

	use super::*;

	struct TestElement {
		id: Id,
		name: &'static str,
	}

	impl ElementHandle for TestElement {
		fn id(&self) -> Id {
			self.id
		}
	}

	fn id(value: u32) -> Id {
		NonZeroU32::new(value).expect("non-zero test element identity")
	}

	#[test]
	fn fetcher_resolves_elements_and_navigates_nested_relations() {
		let elements = [
			TestElement { id: id(1), name: "root" },
			TestElement { id: id(2), name: "left" },
			TestElement {
				id: id(3),
				name: "right",
			},
			TestElement { id: id(4), name: "leaf" },
		];
		let relations = [(id(1), id(3)), (id(1), id(2)), (id(2), id(4))];
		let fetcher = Fetcher {
			elements: &elements,
			relation_map: &relations,
		};

		assert_eq!(fetcher.get(id(1)).expect("root").element().name, "root");
		assert!(fetcher.get(id(1)).expect("root").parent().is_none());
		assert!(fetcher.get(id(99)).is_none());

		let root = fetcher.get(id(1)).expect("root");
		assert_eq!(root.children().ids().collect::<Vec<_>>(), [id(3), id(2)]);
		assert_eq!(
			root.children()
				.elements()
				.map(|element| element.element().name)
				.collect::<Vec<_>>(),
			["right", "left"]
		);

		let leaf_parent = fetcher.get(id(4)).expect("leaf").parent().expect("leaf parent");
		assert_eq!(leaf_parent.id(), id(2));
		assert_eq!(leaf_parent.element().expect("resolved parent").element().name, "left");
	}

	#[test]
	fn dangling_relations_preserve_ids_without_panicking_during_element_lookup() {
		let elements = [
			TestElement { id: id(1), name: "root" },
			TestElement {
				id: id(5),
				name: "orphan",
			},
		];
		let relations = [(id(1), id(9)), (id(8), id(5))];
		let fetcher = Fetcher {
			elements: &elements,
			relation_map: &relations,
		};

		let children = fetcher.get(id(1)).expect("root").children();
		assert_eq!(children.ids().collect::<Vec<_>>(), [id(9)]);
		assert_eq!(children.elements().count(), 0);

		let parent = fetcher.get(id(5)).expect("orphan").parent().expect("recorded parent");
		assert_eq!(parent.id(), id(8));
		assert!(parent.element().is_none());
	}
}
