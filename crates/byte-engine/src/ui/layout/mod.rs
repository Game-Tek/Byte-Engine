pub mod engine;
pub mod query;

use super::{
    element::{self, Element, ElementHandle, Id},
    flow::{self, Location, Location3, Offset, Size},
    layout::query::{ElementResult, Fetcher},
    primitive::BasePrimitive,
    Primitive,
};

#[derive(Clone)]
struct ConcreteElement {
    id: Id,
    flow: flow::FlowFunction,
    primitive: BasePrimitive,
}

impl ConcreteElement {
    pub fn new(id: Id, element: &dyn Element) -> Self {
        Self {
            id,
            flow: element.flow(),
            primitive: element.primitive(),
        }
    }
}

impl Element for ConcreteElement {
    fn flow(&self) -> flow::FlowFunction {
        self.flow
    }

    fn primitive(&self) -> BasePrimitive {
        self.primitive.clone()
    }
}

impl ElementHandle for ConcreteElement {
    fn id(&self) -> Id {
        self.id
    }
}

#[derive(Clone, Copy)]
/// Describes an element layed out for an screen an ready to be rendered,
pub(crate) struct LayoutElement {
    pub(crate) id: u32,
    pub(crate) position: Location3,
    pub(crate) size: Size,
}

impl ElementHandle for LayoutElement {
    fn id(&self) -> Id {
        Id::new(self.id).unwrap()
    }
}

/// Lays out the given elements and returns a vector of layout elements with their calculated positions and sizes for a given viewport.
/// The relation map describes embedded elements.
fn layout_elements(
    elements: &[ConcreteElement],
    relation_map: &[(Id, Id)],
    available_space: Size,
) -> Vec<LayoutElement> {
    let mut lelements = Vec::with_capacity(elements.len());

    #[derive(Clone, Copy)]
    struct TraversalState {
        available_space: Size,
        offset: Offset,
    }

    #[derive(Clone, Copy)]
    struct Context<'a> {
        fetcher: &'a Fetcher<'a, ConcreteElement>,
        root_size: Size,
    }

    fn calculate_element(
        element: ElementResult<'_, ConcreteElement>,
        _: Context,
        ts: TraversalState,
    ) -> LayoutElement {
        let primitive = element.element().primitive();
        let shape = primitive.shape();

        let size = shape.bbox(ts.available_space);

        LayoutElement {
            id: element.id().into(),
            position: Location3::from((ts.offset.into(), 0)),
            size,
        }
    }

    fn layout_element(
        elements: &mut Vec<LayoutElement>,
        element: ElementResult<'_, ConcreteElement>,
        ctx: Context,
        ts: TraversalState,
    ) -> LayoutElement {
        let l = calculate_element(element.clone(), ctx, ts);

        let available_space = l.size;
        let mut offset: Offset = Into::<Location>::into(l.position).into();

        elements.push(l);

        for child in element.children().elements() {
            let l = layout_element(
                elements,
                child,
                ctx,
                TraversalState {
                    available_space,
                    offset,
                },
            );
            offset = element.element().flow()(offset, l.size);
        }

        l
    }

    let fetcher = Fetcher {
        elements,
        relation_map,
    };

    let root = elements
        .iter()
        .find_map(|container| {
            let res = fetcher.get(container.id())?;
            if res.parent().is_none() {
                Some(res)
            } else {
                None
            }
        })
        .expect("Root container not found");

    layout_element(
        &mut lelements,
        root,
        Context {
            fetcher: &fetcher,
            root_size: available_space,
        },
        TraversalState {
            available_space,
            offset: Offset::new(0, 0),
        },
    );

    lelements
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sizing {
    Relative(u16, u16),
    Absolute(u32),
}

impl Sizing {
    pub fn full() -> Self {
        Self::Relative(1, 1)
    }

    pub fn pixels(value: u32) -> Self {
        Self::Absolute(value)
    }

    pub fn calculate(&self, available: u32) -> u32 {
        match self {
            Sizing::Relative(num, denom) => (available * *num as u32) / *denom as u32,
            Sizing::Absolute(value) => *value,
        }
    }
}

impl Default for Sizing {
    fn default() -> Self {
        Self::full()
    }
}

impl Into<Sizing> for u32 {
    fn into(self) -> Sizing {
        Sizing::Absolute(self)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        components::container::{BaseContainer, ContainerSettings},
        element::{ElementHandle, Id},
        flow::{self, Location, Location3, Size},
        layout::{ConcreteElement, Sizing},
        Element,
    };

    use super::layout_elements;

    fn make_elements(elements: &[&dyn Element]) -> Vec<ConcreteElement> {
        let mut counter = Id::MIN;

        elements
            .iter()
            .map(|e| {
                let id = counter;

                counter = counter.checked_add(1).unwrap();

                ConcreteElement {
                    id,
                    flow: e.flow(),
                    primitive: e.primitive(),
                }
            })
            .collect()
    }

    #[test]
    fn layout_root() {
        let root = BaseContainer::new(Default::default());

        let elements = make_elements(&[&root as &dyn Element]);

        let elements = layout_elements(&elements, &[], Size::new(1024, 1024));

        assert_eq!(elements.len(), 1);

        let element = &elements[0];

        assert_eq!(element.size, Size::new(1024, 1024));
    }

    #[test]
    fn layout_root_half_size() {
        let root = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));

        let elements = make_elements(&[&root as &dyn Element]);

        let elements = layout_elements(&elements, &[], Size::new(1024, 1024));

        assert_eq!(elements.len(), 1);

        let element = &elements[0];

        assert_eq!(element.size, Size::new(512, 512));
    }

    #[test]
    fn layout_half_children() {
        let root = BaseContainer::new(Default::default());
        let a = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
        let b = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
        let c = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
        let d = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));

        let elements = make_elements(&[
            &root as &dyn Element,
            &a as &dyn Element,
            &b as &dyn Element,
            &c as &dyn Element,
            &d as &dyn Element,
        ]);

        let root = &elements[0];
        let a = &elements[1];
        let b = &elements[2];
        let c = &elements[3];
        let d = &elements[4];

        let elements = layout_elements(
            &elements,
            &[
                (root.id(), a.id()),
                (a.id(), b.id()),
                (b.id(), c.id()),
                (c.id(), d.id()),
            ],
            Size::new(1024, 1024),
        );

        assert_eq!(elements.len(), 5);

        let element = &elements[0];
        assert_eq!(element.size, Size::new(1024, 1024));

        let element = &elements[1];
        assert_eq!(element.size, Size::new(512, 512));

        let element = &elements[2];
        assert_eq!(element.size, Size::new(256, 256));

        let element = &elements[3];
        assert_eq!(element.size, Size::new(128, 128));

        let element = &elements[4];
        assert_eq!(element.size, Size::new(64, 64));
    }

    #[test]
    fn layout_column() {
        let root = BaseContainer::new(ContainerSettings::default().flow(flow::column));
        let a = BaseContainer::new(ContainerSettings::default().size(Sizing::Absolute(64)));
        let b = BaseContainer::new(ContainerSettings::default().size(Sizing::Absolute(64)));
        let c = BaseContainer::new(ContainerSettings::default().size(Sizing::Absolute(64)));
        let d = BaseContainer::new(ContainerSettings::default().size(Sizing::Absolute(64)));

        let elements = make_elements(&[
            &root as &dyn Element,
            &a as &dyn Element,
            &b as &dyn Element,
            &c as &dyn Element,
            &d as &dyn Element,
        ]);

        let root = &elements[0];
        let a = &elements[1];
        let b = &elements[2];
        let c = &elements[3];
        let d = &elements[4];

        let elements = layout_elements(
            &elements,
            &[
                (root.id(), a.id()),
                (root.id(), b.id()),
                (root.id(), c.id()),
                (root.id(), d.id()),
            ],
            Size::new(1024, 1024),
        );

        assert_eq!(elements.len(), 5);

        let element = &elements[0];
        assert_eq!(element.size, Size::new(1024, 1024));
        assert_eq!(element.position, Location3::new(0, 0, 0));

        let element = &elements[1];
        assert_eq!(element.size, Size::new(64, 64));
        assert_eq!(element.position, Location3::new(0, 0, 0));

        let element = &elements[2];
        assert_eq!(element.size, Size::new(64, 64));
        assert_eq!(element.position, Location3::new(0, 64, 0));

        let element = &elements[3];
        assert_eq!(element.size, Size::new(64, 64));
        assert_eq!(element.position, Location3::new(0, 128, 0));

        let element = &elements[4];
        assert_eq!(element.size, Size::new(64, 64));
        assert_eq!(element.position, Location3::new(0, 192, 0));
    }
}
