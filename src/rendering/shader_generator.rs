use std::rc::Rc;

use crate::jspd::lexer;

pub trait ShaderGenerator {
	fn process(&self, children: Vec<Rc<lexer::Node>>) -> (&'static str, lexer::Node);
}