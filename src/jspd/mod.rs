//! This module contains all code related to the parsing of the BESL language and the generation of the JSPD.

use std::rc::Rc;

mod tokenizer;
mod parser;

pub(crate) fn compile_to_jspd(source: &str) -> Result<Node, ()> {
	let tokens = tokenizer::tokenize(source)?;
	let jspd = parser::parse(tokens)?;

	return Ok(jspd);
}

pub(crate) enum Node {
	Root{
		children: Vec<Rc<Node>>
	},
	Struct {
		name: String,
		fields: Vec<Rc<Node>>
	},
	Member {
		name: String,
		ty: Option<Rc<Node>>
	},
	Function {
		name: String,
		params: Vec<Rc<Node>>,
		return_type: Rc<Node>,
		statements: Vec<Rc<Lexeme>>
	},
}

#[derive(Debug, Clone)]
pub(crate) struct Lexeme {
	lexeme: Lexemes,
	children: Vec<Rc<Lexeme>>
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Lexemes {
	Member,
	Literal,
	FunctionCall,
	VariableDeclaration,
	Assignment,
}

trait Precedence {
	fn precedence(&self) -> u8;
}

impl Precedence for Lexemes {
	fn precedence(&self) -> u8 {
		match self {
			Lexemes::Member => 0,
			Lexemes::Literal => 0,
			Lexemes::FunctionCall => 0,
			Lexemes::VariableDeclaration => 0,
			Lexemes::Assignment => 255,
		}
	}
}

use std::ops::Index;

impl Index<&str> for Node {
    type Output = Node;

    fn index(&self, index: &str) -> &Self::Output {
        let children = match self {
			Node::Root { children } => {
				children
			},
			Node::Struct { name, fields } => {
				fields
			},
			_ => panic!("Not implemented")
		};

		for child in children {
			match child.as_ref() {
				Node::Struct { name, fields: _ } => {
					if name == index {
						return child;
					}
				},
				Node::Member { name, ty: _ } => {
					if name == index {
						return child;
					}
				},
				Node::Function { name, params, return_type, statements } => {
					if name == index {
						return child;
					}
				},
				_ => {}
			}
		}

		panic!("Not found");
    }
}