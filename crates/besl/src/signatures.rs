//! Expands signatures: every `name: Signature { ... }` becomes a function with the signature's parameters and return
//! type before the program links.
//!
//! Entry normalization reads `main`'s parameters as parsed, so signatures expand on the parsed program, where a path
//! resolves through the program's modules and `use` declarations the same way linking resolves it.

use crate::lexer::LexError;
use crate::parser::{Node, Nodes, SignaturePart, TypeName};

/// The documentation page of the signature rules that errors point to.
const SIGNATURES_DOCS_PATH: &str = "reference/besl/interface#layers-and-signatures";

/// How many signatures one signature can reach through its parts, which stops a signature that contains itself.
const MAX_SIGNATURE_DEPTH: usize = 64;

/// Replaces every implementation in `program` with a function that has its signature's parameters and return type.
pub(crate) fn expand_signatures(program: &mut Node<'_>) -> Result<(), LexError> {
	// Resolve every implementation first, while the program is only read, then rewrite them.
	let mut expansions = Vec::new();
	collect_expansions(program, program, &mut Vec::new(), &mut expansions)?;

	for (path, index, params, return_type, types) in expansions {
		let scope = scope_at_mut(program, &path).expect("An expanded implementation's scope still exists");
		let Nodes::Scope { children, .. } = scope.node_mut() else {
			unreachable!("`scope_at_mut` returns only scopes");
		};
		let Nodes::Implementation { name, statements, .. } = children[index].node_mut() else {
			unreachable!("The expansion was collected for an implementation");
		};
		let statements = std::mem::take(statements);
		let mut function = Node::function(name, params, return_type, statements);
		// The signature's types name declarations of the module that declares it.
		if let Nodes::Function {
			types: function_types, ..
		} = function.node_mut()
		{
			*function_types = Some(types);
		}
		children[index] = function;
	}

	Ok(())
}

/// One implementation to rewrite: its scope's path, its index in that scope, and its signature's flattened parameters
/// and return type.
type Expansion<'a> = (Vec<&'a str>, usize, Vec<Node<'a>>, TypeName<'a>, Vec<&'a str>);

/// Collects the expansion of every implementation under `scope`, whose path is `path`.
fn collect_expansions<'a>(
	program: &Node<'a>,
	scope: &Node<'a>,
	path: &mut Vec<&'a str>,
	expansions: &mut Vec<Expansion<'a>>,
) -> Result<(), LexError> {
	let Nodes::Scope { children, .. } = scope.node() else {
		return Ok(());
	};
	for (index, child) in children.iter().enumerate() {
		match child.node() {
			Nodes::Scope { name, .. } => {
				path.push(name);
				collect_expansions(program, child, path, expansions)?;
				path.pop();
			}
			Nodes::Implementation { name, signature, .. } => {
				let (params, return_type, types) = flatten(program, path, signature, 0)
					.map_err(|error| LexError::invalid(format!("Function `{name}` can't implement `{signature}`: {error}")))?;
				expansions.push((path.clone(), index, params, return_type, types));
			}
			_ => {}
		}
	}
	Ok(())
}

/// Returns the parameters and return type of the signature `path` names from the scope at `scope`, and the path of the
/// module that declares the signature, whose declarations the types name.
fn flatten<'a>(
	program: &Node<'a>,
	scope: &[&'a str],
	path: &'a str,
	depth: usize,
) -> Result<(Vec<Node<'a>>, TypeName<'a>, Vec<&'a str>), String> {
	if depth > MAX_SIGNATURE_DEPTH {
		return Err(format!(
			"`{path}` contains itself. The most likely cause is a signature that names itself through its parts. Remove the cycle. See {}.",
			crate::online_docs_url(SIGNATURES_DOCS_PATH)
		));
	}

	let (owner, declaration) = resolve(program, scope, path, 0).ok_or_else(|| {
		format!(
			"`{path}` names no signature. The most likely cause is a misspelled path, or a signature its module doesn't `use`. Check the path. See {}.",
			crate::online_docs_url(SIGNATURES_DOCS_PATH)
		)
	})?;
	let Nodes::Signature { parts, .. } = declaration.node() else {
		return Err(format!(
			"`{path}` isn't a signature. The most likely cause is a path that names a struct, layer, or function. Name a declaration of the form `Name: fn (...) -> Return;` or `Name: A + B;`."
		));
	};

	let mut params: Vec<Node<'a>> = Vec::new();
	let mut return_type = TypeName::Named("void");
	for part in parts {
		let (part_params, part_return) = match part {
			SignaturePart::Named(part) => {
				let (params, return_type, part_owner) = flatten(program, &owner, part, depth + 1)?;
				// Types resolve in one module, so a part from another module must name its types the same way there.
				if part_owner != owner
					&& params
						.iter()
						.any(|parameter| !names_same_type(program, &owner, &part_owner, parameter))
				{
					return Err(format!(
						"its part `{part}` names types of another module. The most likely cause is a signature composed from signatures of two modules. Declare the combined signature in the module whose types its parameters name, or `use` those types where it is declared. See {}.",
						crate::online_docs_url(SIGNATURES_DOCS_PATH)
					));
				}
				(params, return_type)
			}
			SignaturePart::Inline { params, return_type } => (params.clone(), return_type.clone()),
		};

		for parameter in part_params {
			let Nodes::Parameter { name, r#type } = parameter.node() else {
				continue;
			};
			match params.iter().find_map(|existing| match existing.node() {
				Nodes::Parameter {
					name: existing_name,
					r#type: existing_type,
				} if existing_name == name => Some(existing_type),
				_ => None,
			}) {
				// Two parts that take the same parameter take it once.
				Some(existing) if existing == r#type => {}
				Some(existing) => {
					return Err(format!(
						"its parts take parameter `{name}` as both `{existing}` and `{type}`. The most likely cause is two signatures that use one parameter name for different things. Rename one of them. See {}.",
						crate::online_docs_url(SIGNATURES_DOCS_PATH)
					));
				}
				None => params.push(parameter),
			}
		}

		if part_return != TypeName::Named("void") {
			if return_type != TypeName::Named("void") && return_type != part_return {
				return Err(format!(
					"its parts return both `{return_type}` and `{part_return}`. The most likely cause is two signatures that each declare a return type. Return `void` from all but one of them. See {}.",
					crate::online_docs_url(SIGNATURES_DOCS_PATH)
				));
			}
			return_type = part_return;
		}
	}

	Ok((params, return_type, owner))
}

/// Reports whether `parameter`'s type names the same declaration from the modules at `a` and `b`. A built-in type,
/// which no module declares, names the same type everywhere.
fn names_same_type<'a>(program: &Node<'a>, a: &[&'a str], b: &[&'a str], parameter: &Node<'a>) -> bool {
	let Nodes::Parameter {
		r#type: TypeName::Named(type_name),
		..
	} = parameter.node()
	else {
		return true;
	};
	let declaration = |module: &[&'a str]| resolve(program, module, type_name, 0).map(|(_, node)| node as *const Node<'a>);
	declaration(a) == declaration(b)
}

/// Returns the declaration `path` names from the scope at `scope`, and the path of the scope that declares it.
///
/// It follows the rules linking uses: a path starts at `::` (the modules from the asset root), `crate`, `self`, one
/// or more `super`, or a name that the scope or one around it declares or brings in with `use`. Each later segment
/// names a declaration of the module before it.
pub(crate) fn resolve<'p, 'a>(
	program: &'p Node<'a>,
	scope: &[&'a str],
	path: &'a str,
	depth: usize,
) -> Option<(Vec<&'a str>, &'p Node<'a>)> {
	// A `use` that names itself through others would never end.
	if depth > MAX_SIGNATURE_DEPTH {
		return None;
	}

	// The module the path starts from, its own path, and the segments after it.
	let (module_path, module, rest): (Vec<&'a str>, &'p Node<'a>, &'a str) = if let Some(rest) = path.strip_prefix("::") {
		let Nodes::Scope { children, .. } = scope_at(program, &["::"])?.node() else {
			return None;
		};
		children.iter().find_map(|child| {
			let name = child.name()?;
			let rest = match rest.strip_prefix(name)? {
				"" => "",
				rest => rest.strip_prefix("::")?,
			};
			Some((vec!["::", name], child, rest))
		})?
	} else {
		let (first, rest) = path.split_once("::").unwrap_or((path, ""));
		match first {
			"crate" => (Vec::new(), program, rest),
			"self" => (scope.to_vec(), scope_at(program, scope)?, rest),
			"super" => {
				let mut module = scope.len().checked_sub(1)?;
				let mut rest = rest;
				while let Some(next) = rest.strip_prefix("super") {
					module = module.checked_sub(1)?;
					rest = next.strip_prefix("::").unwrap_or(next);
				}
				(scope[..module].to_vec(), scope_at(program, &scope[..module])?, rest)
			}
			first => {
				let (owner, declaration) = (0..=scope.len())
					.rev()
					.find_map(|outer| lookup(program, &scope[..outer], first, depth))?;
				if rest.is_empty() {
					return Some((owner, declaration));
				}
				let mut module_path = owner;
				module_path.push(declaration.name()?);
				(module_path, declaration, rest)
			}
		}
	};

	if rest.is_empty() {
		let owner = module_path[..module_path.len().saturating_sub(1)].to_vec();
		return Some((owner, module));
	}
	walk(module_path, module, rest)
}

/// Walks `rest`, a path relative to `module` at `path`, through module declarations to the declaration it names, and
/// returns it with the path of the module that declares it.
fn walk<'p, 'a>(mut path: Vec<&'a str>, mut module: &'p Node<'a>, rest: &'a str) -> Option<(Vec<&'a str>, &'p Node<'a>)> {
	let mut segments = rest.split("::").peekable();
	while let Some(segment) = segments.next() {
		let Nodes::Scope { children, .. } = module.node() else {
			return None;
		};
		let child = children.iter().find(|child| child.name() == Some(segment))?;
		if segments.peek().is_none() {
			return Some((path, child));
		}
		path.push(segment);
		module = child;
	}
	None
}

/// Finds `name` among the declarations of the scope at `scope` as code inside it sees them: its own declarations,
/// then the names its `use` declarations bring in, explicit ones first.
fn lookup<'p, 'a>(program: &'p Node<'a>, scope: &[&'a str], name: &str, depth: usize) -> Option<(Vec<&'a str>, &'p Node<'a>)> {
	let Nodes::Scope { children, .. } = scope_at(program, scope)?.node() else {
		return None;
	};

	if let Some(declaration) = children
		.iter()
		.find(|child| child.name() == Some(name) && !matches!(child.node(), Nodes::Use { .. }))
	{
		return Some((scope.to_vec(), declaration));
	}

	let uses = || {
		children.iter().filter_map(|child| match child.node() {
			Nodes::Use { path, glob } => Some((*path, *glob)),
			_ => None,
		})
	};
	if let Some((path, _)) = uses().find(|(path, glob)| !glob && path.rsplit("::").next() == Some(name)) {
		return resolve(program, scope, path, depth + 1);
	}
	uses().filter(|(_, glob)| *glob).find_map(|(path, _)| {
		let (owner, module) = resolve(program, scope, path, depth + 1)?;
		let Nodes::Scope {
			name: module_name,
			children,
		} = module.node()
		else {
			return None;
		};
		let declaration = children.iter().find(|child| child.name() == Some(name))?;
		let mut owner = owner;
		owner.push(module_name);
		Some((owner, declaration))
	})
}

/// Returns the scope at `path`, a list of module names from the program root.
fn scope_at<'p, 'a>(program: &'p Node<'a>, path: &[&str]) -> Option<&'p Node<'a>> {
	path.iter().try_fold(program, |scope, segment| match scope.node() {
		Nodes::Scope { children, .. } => children
			.iter()
			.find(|child| matches!(child.node(), Nodes::Scope { .. }) && child.name() == Some(segment)),
		_ => None,
	})
}

/// Returns the scope at `path` for changing it. See [`scope_at`].
fn scope_at_mut<'p, 'a>(program: &'p mut Node<'a>, path: &[&str]) -> Option<&'p mut Node<'a>> {
	let mut scope = program;
	for segment in path {
		scope = scope
			.get_mut(segment)
			.filter(|child| matches!(child.node(), Nodes::Scope { .. }))?;
	}
	Some(scope)
}
