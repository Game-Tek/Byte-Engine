use super::element::{ElementHandle, Id};

pub struct Fetcher<'a, T> {
	pub elements: Vec<T>,
	pub relation_map: &'a [(Id, Id)],
}

pub struct ElementResult<'a, T> {
	element: T,
	fetcher: &'a Fetcher<'a, T>,
}

impl<'a, T: ElementHandle> ElementResult<'a, T> {
	pub fn into_element(self) -> T {
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
		let children = self
			.fetcher
			.relation_map
			.iter()
			.filter_map(|&(p, c)| if p == self.element.id() { Some(c) } else { None })
			.collect();

		ChildrenResult {
			children,
			fetcher: self.fetcher,
		}
	}
}

impl<T: ElementHandle> ElementHandle for ElementResult<'_, T> {
	fn id(&self) -> Id {
		self.element.id()
	}
}

pub struct ParentResult<'a, T> {
	id: Id,
	fetcher: &'a Fetcher<'a, T>,
}

impl<'a, T: ElementHandle> ParentResult<'a, T> {
	fn element(&self) -> ElementResult<'_, T> {
		self.fetcher.get(self.id).unwrap()
	}

	fn id(&self) -> Id {
		self.id
	}
}

pub struct ChildrenResult<'a, T> {
	children: Vec<Id>,
	fetcher: &'a Fetcher<'a, T>,
}

impl<'a, T: ElementHandle> ChildrenResult<'a, T> {
	pub fn elements(&self) -> impl Iterator<Item = ElementResult<'_, T>> + '_ {
		self.children.iter().filter_map(|id| self.fetcher.get(*id))
	}
	pub fn ids(&self) -> impl Iterator<Item = Id> + '_ {
		self.children.iter().map(|id| *id)
	}
}

impl<'a, T: ElementHandle> Fetcher<'a, T> {
	pub fn get(&mut self, id: Id) -> Option<ElementResult<'a, T>> {
		self.elements
			.pop_if(|e| e.id() == id)
			.map(|element| ElementResult { element, fetcher: self })
	}
}
