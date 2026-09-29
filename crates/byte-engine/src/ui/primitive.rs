use crate::ui::{
	Container,
	components::{curve::Curve, image::Image, path::Path, text::Text},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Events {
	Actuated,
	Scrolled,
	/// The pointer was pressed on this element and the engine holds it until
	/// release or cancellation. Delivered to the surface under the press.
	Grabbed,
	/// The held element moved past the drag threshold. Delivered to the source
	/// once per evaluation while the pointer moves, with
	/// [`super::UiEvent::delta`] set to the offset from the press point in layout units.
	Dragged,
	/// A source was released over this element or one of its descendants.
	/// Delivered to the target under the release point, then to each ancestor,
	/// with [`super::UiEvent::source`] set. The source never receives its own drop.
	Dropped,
	/// A grab ended by release or cancellation. Delivered to the source after any
	/// [`Self::Dropped`] the release produced, with [`super::UiEvent::source`]
	/// set to the surface the drag was dropped on, if any.
	DragEnded,
	/// The pointer moved onto this surface or one of its descendants. Delivered
	/// once per evaluation in which the surface under the pointer changed, to
	/// the new surface and then to each ancestor that did not already contain
	/// the pointer. A held drag source is skipped, so a target under a dragged
	/// item still hears about the pointer.
	PointerEntered,
	/// The pointer left this surface and all of its descendants. Delivered to
	/// the previous surface and then to each ancestor that no longer contains
	/// the pointer, before any [`Self::PointerEntered`] of the same evaluation.
	PointerExited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
	Escape,
	Backspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEdit {
	Inserted(char),
	Deleted(char),
}

impl TextEdit {
	pub fn apply_to(self, content: &mut String) {
		match self {
			Self::Inserted(character) => content.push(character),
			Self::Deleted(character) => {
				if content.ends_with(character) {
					content.pop();
				}
			}
		}
	}
}

/// The `Primitives` enum holds the kind-specific state of one element the engine owns.
///
/// Layout, hit testing, and rendering match on it to reach what only that kind has, such as a container's flow or
/// a text's content. The state every kind shares, such as style and transform, lives on the tree node instead.
pub enum Primitives {
	Container(Container),
	Curve(Curve),
	Path(Path),
	Image(Image),
	Text(Text),
}

impl Primitives {
	/// Reports whether pointer hit testing can find this element.
	///
	/// Containers opt out with [`crate::ui::Properties::hit_testable`], editable text is always a target, and curves
	/// are one once they have a hit width.
	pub(crate) fn hit_testable(&self) -> bool {
		match self {
			Primitives::Container(container) => container.hit_testable,
			Primitives::Text(text) => text.editable,
			Primitives::Curve(curve) => curve.hit_width().is_some(),
			Primitives::Path(_) | Primitives::Image(_) => false,
		}
	}
}
