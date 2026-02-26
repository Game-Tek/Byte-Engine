use std::marker::PhantomData;

use super::{
    element::{ElementHandle, Id},
    flow::Size,
    layout_elements,
    query::Fetcher,
    ConcreteElement, Element, LayoutElement,
};

pub struct Engine {
    viewports: Vec<VirtualViewport>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            viewports: Vec::new(),
        }
    }

    pub fn add_viewport(&mut self, viewport: VirtualViewport) {
        self.viewports.push(viewport);
    }

    pub fn render<'a>(&'a mut self, root: &impl Component) -> Render<'a> {
        struct State<'a> {
            id: Id,
            counter: &'a mut u32,
            elements: &'a mut Vec<ConcreteElement>,
            relations: &'a mut Vec<(Id, Id)>,
        }

        impl<'state> Context for State<'state> {
            type Child<'a>
                = State<'a>
            where
                Self: 'a;

            fn element<'a>(&'a mut self, element: &dyn Element) -> Self::Child<'a> {
                let id = Id::new(*self.counter).unwrap();

                *self.counter += 1;

                self.elements.push(ConcreteElement::new(id, element));

                if id != self.id {
                    self.relations.push((self.id, id));
                }

                State {
                    id,
                    counter: &mut *self.counter,
                    elements: &mut *self.elements,
                    relations: &mut *self.relations,
                }
            }

            fn component(&mut self, component: &impl Component) {
                component.render(self);
            }
        }

        let mut elements = Vec::new();
        let mut relations = Vec::new();

        let mut counter = 1;

        let mut state = State {
            id: Id::new(counter).unwrap(),
            counter: &mut counter,
            elements: &mut elements,
            relations: &mut relations,
        };

        root.render(&mut state);

        let elements = layout_elements(&elements, &relations, Size::new(1024, 1024));

        Render {
            elements,
            relations,
            _phantom: PhantomData,
        }
    }
}

pub struct Render<'a> {
    elements: Vec<LayoutElement>,
    relations: Vec<(Id, Id)>,
    _phantom: PhantomData<&'a ()>,
}

impl<'a> Render<'a> {
    pub fn root(&self) -> &LayoutElement {
        self.elements.iter().find(|e| e.id == 1).unwrap()
    }

    pub fn query(&self) -> Fetcher<'_, LayoutElement> {
        Fetcher {
            elements: &self.elements,
            relation_map: &self.relations,
        }
    }

    pub fn size(&self) -> usize {
        self.elements.len()
    }
}

pub trait Context: Sized {
    type Child<'a>: Context
    where
        Self: 'a;

    fn element<'a>(&'a mut self, element: &dyn Element) -> Self::Child<'a>;
    fn component(&mut self, component: &impl Component);
}

struct VirtualViewport;

impl ElementHandle for VirtualViewport {
    fn id(&self) -> Id {
        unimplemented!()
    }
}

pub trait Component {
    fn render(&self, ctx: &mut impl Context);
}

#[cfg(test)]
mod tests {
    use super::super::super::{
        components::container::{BaseContainer, ContainerSettings},
        element::Id,
        flow::Size,
        layout::engine::{Context, Engine, VirtualViewport},
        Component,
    };

    struct Bar {
        options: Vec<BarOption>,
    }

    impl Bar {
        pub fn new(options: Vec<BarOption>) -> Self {
            Self { options }
        }
    }

    impl Component for Bar {
        fn render(&self, ctx: &mut impl Context) {
            let mut ctx = ctx.element(&BaseContainer::new(
                ContainerSettings::default().height(32.into()),
            ));

            for option in &self.options {
                option.render(&mut ctx);
            }
        }
    }

    struct BarOption {
        label: String,
    }

    impl BarOption {
        pub fn new(label: String) -> Self {
            Self { label }
        }
    }

    impl Component for BarOption {
        fn render(&self, ctx: &mut impl Context) {
            ctx.element(&BaseContainer::new(ContainerSettings::default()));
        }
    }

    struct BarList {}

    impl BarList {
        pub fn new() -> Self {
            Self {}
        }
    }

    struct BarListOption {}

    impl BarListOption {
        pub fn new() -> Self {
            Self {}
        }
    }

    struct Application {
        bar: Bar,
    }

    impl Application {
        pub fn new() -> Self {
            let options = vec![
                BarOption::new("File".to_string()),
                BarOption::new("Edit".to_string()),
                BarOption::new("View".to_string()),
            ];

            Self {
                bar: Bar::new(options),
            }
        }
    }

    impl Component for Application {
        fn render(&self, ctx: &mut impl Context) {
            let mut ctx = ctx.element(&BaseContainer::new(ContainerSettings::default()));
            self.bar.render(&mut ctx);
        }
    }

    #[test]
    fn it_works() {
        let viewport = VirtualViewport;

        let mut engine = Engine::new();

        engine.add_viewport(viewport);

        let application = Application::new();

        let render = engine.render(&application);

        assert_eq!(render.size(), 5);

        let query = render.query();

        let root = query.get(Id::new(1).unwrap()).unwrap();

        {
            let root = root.element();

            assert_eq!(root.size, Size::new(1024, 1024));
        }

        let children = root.children();

        {
            let children = children.elements();

            assert_eq!(children.size_hint().1.unwrap(), 1);
        }
    }
}
