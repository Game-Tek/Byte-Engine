use std::ops::Index;

use super::expressions::is_identifier;
use super::*;

impl<'a> Index<&str> for Node<'a> {
	type Output = Node<'a>;

	fn index(&self, index: &str) -> &Self::Output {
		let child = match &self.node {
			Nodes::Scope { children, .. } => children.iter().find(|child| {
				matches!(
					child.node(),
					Nodes::Scope { .. }
						| Nodes::Struct { .. }
						| Nodes::Member { .. }
						| Nodes::Function { .. }
						| Nodes::Descriptor { .. }
						| Nodes::Input { .. }
						| Nodes::Output { .. }
						| Nodes::TaskPayload { .. }
						| Nodes::Workgroup { .. }
						| Nodes::Const { .. }
				) && child.name() == Some(index)
			}),
			Nodes::Struct { fields, .. } => fields
				.iter()
				.find(|field| matches!(field.node(), Nodes::Member { .. }) && field.name() == Some(index)),
			_ => panic!("Cannot search  in these"),
		};

		child.unwrap_or_else(|| panic!("Not found"))
	}
}

pub(super) trait ParserIterator<'a> {
	fn next_is(&mut self, f: impl Fn(&'a str) -> bool) -> Result<&'a str, ParsingFailReasons>;
	fn next_str(&mut self, expected: &'a str) -> Result<&'a str, ParsingFailReasons>;
	fn next_identifier(&mut self) -> Result<&'a str, ParsingFailReasons>;
}

impl<'i, 'a> ParserIterator<'a> for std::slice::Iter<'i, &'a str> {
	fn next_is(&mut self, f: impl Fn(&'a str) -> bool) -> Result<&'a str, ParsingFailReasons> {
		let token = self.next().ok_or(ParsingFailReasons::StreamEndedPrematurely)?;
		if f(token) {
			Ok(token)
		} else {
			Err(ParsingFailReasons::NotMine)
		}
	}

	fn next_str(&mut self, expected: &'a str) -> Result<&'a str, ParsingFailReasons> {
		self.next_is(|v| v == expected)
	}

	fn next_identifier(&mut self) -> Result<&'a str, ParsingFailReasons> {
		self.next_is(is_identifier)
	}
}

#[derive(Clone)]
pub struct ProgramState {
	// pub(super) types: HashMap<String, NodeReference>,
}
