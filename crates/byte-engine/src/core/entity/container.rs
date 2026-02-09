use utils::Box;

#[derive(Clone)]
pub struct Container<T: ?Sized>(pub(crate) Box<T>);
