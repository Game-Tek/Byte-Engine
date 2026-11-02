//! Links layers: values of a layer type, access to their members, and the erasure that leaves generated code without
//! them.
//!
//! A layer value exists only while a program links. `main` and helpers take layers as parameters and reach members
//! through them, such as `scene.views`, which links to the member itself. Once the program is linked,
//! [`record_entry_layers`] notes which layers `main` takes, for reflection, and [`erase_layer_parameters`] removes every
//! layer parameter and argument, so backends see the members as plain declarations.

use super::*;

/// Returns the full path of the layer `name` declared in the innermost module of `chain`, such as
/// `::byte_engine::layouts::scene::Scene` for a layer in a module from the asset root, or `Scene` at the program root.
pub(super) fn layer_path(chain: &[NodeReference], name: &str) -> String {
	let mut path = String::new();
	for scope in chain {
		let scope = scope.borrow();
		let Nodes::Scope { name: module, .. } = scope.node() else {
			continue;
		};
		match module.as_str() {
			"root" => {}
			// The modules from the asset root start their paths with `::`.
			"::" => path.push_str("::"),
			module => {
				if !path.is_empty() && !path.ends_with("::") {
					path.push_str("::");
				}
				path.push_str(module);
			}
		}
	}
	if !path.is_empty() && !path.ends_with("::") {
		path.push_str("::");
	}
	path.push_str(name);
	path
}

/// Reports whether `parameter` is a function parameter of a layer type.
pub(super) fn is_layer_parameter(parameter: &NodeReference) -> bool {
	match parameter.borrow().node() {
		Nodes::Parameter { r#type, .. } => matches!(r#type.borrow().node(), Nodes::Layer { .. }),
		_ => false,
	}
}

/// Returns the layer `expression` is a value of, when it names a layer parameter.
pub(super) fn layer_of(expression: &NodeReference) -> Option<NodeReference> {
	let expression = expression.borrow();
	let Nodes::Expression(Expressions::Member { source, .. }) = expression.node() else {
		return None;
	};
	let source = source.borrow();
	match source.node() {
		Nodes::Parameter { r#type, .. } if matches!(r#type.borrow().node(), Nodes::Layer { .. }) => Some(r#type.clone()),
		_ => None,
	}
}

/// Links `layer.name`: a read of the layer's member `name`, under the member's own name, as backends emit it.
pub(super) fn member_access(layer: &NodeReference, name: &str) -> Result<NodeReference, LexError> {
	let layer = layer.borrow();
	let Nodes::Layer {
		name: layer_name,
		members,
		push_constant,
		..
	} = layer.node()
	else {
		unreachable!("`layer_of` returns only layers");
	};

	let member = if push_constant.as_deref() == Some(name) {
		members
			.iter()
			.find(|member| matches!(member.borrow().node(), Nodes::PushConstant { .. }))
			.cloned()
	} else {
		resolution::find_named_child(members, name)
	};
	let member = member.ok_or_else(|| {
		LexError::invalid(format!(
			"`{name}` isn't a member of layer `{layer_name}`. The most likely cause is a misspelled member name. Use one of the descriptors, the push constant block, or the tile targets that `{layer_name}` declares."
		))
	})?;

	let emitted = member.borrow().get_name().unwrap_or(name).to_string();
	Ok(Node::expression(Expressions::Member {
		source: member,
		name: emitted,
	})
	.into())
}

/// Notes on each layer which of `main`'s layer parameters takes it, and checks that the layers `main` takes and the
/// program's own declarations hold at most one push constant block between them.
pub(super) fn record_entry_layers(root: &NodeReference) -> Result<(), LexError> {
	let Some(main) = root.get_main() else {
		return Ok(());
	};
	let main = main.borrow();
	let Nodes::Function { params, .. } = main.node() else {
		return Ok(());
	};

	// The program's own block is a direct child of the root.
	let own = match root.borrow().node() {
		Nodes::Scope { children, .. } => children
			.iter()
			.any(|child| matches!(child.borrow().node(), Nodes::PushConstant { .. })),
		_ => false,
	};
	let mut push_constant_owner = own.then(|| "the shader".to_string());

	let layers = params.iter().filter_map(|parameter| match parameter.borrow().node() {
		Nodes::Parameter { r#type, .. } if matches!(r#type.borrow().node(), Nodes::Layer { .. }) => Some(r#type.clone()),
		_ => None,
	});
	for (index, layer) in layers.enumerate() {
		let mut layer = layer.borrow_mut();
		let Nodes::Layer {
			path,
			push_constant,
			entry,
			..
		} = layer.node_mut()
		else {
			continue;
		};
		*entry = Some(index);

		if push_constant.is_some() {
			if let Some(owner) = &push_constant_owner {
				return Err(LexError::invalid(format!(
					"`main` takes two push constant blocks, from {owner} and from layer `{path}`. The most likely cause is two layers that each declare a push constant block. A shader binds one block: keep it in one of them."
				)));
			}
			push_constant_owner = Some(format!("layer `{path}`"));
		}
	}

	Ok(())
}

/// Removes every layer parameter from the functions under `node`, and every layer argument from the calls in them, so
/// generated code reads layer members as plain declarations.
pub(super) fn erase_layer_parameters(node: &NodeReference) {
	let children = {
		let mut borrowed = node.borrow_mut();
		match borrowed.node_mut() {
			Nodes::Function { params, .. } => params.retain(|parameter| !is_layer_parameter(parameter)),
			Nodes::Expression(Expressions::FunctionCall { parameters, .. }) => {
				parameters.retain(|argument| layer_of(argument).is_none());
			}
			_ => {}
		}
		match borrowed.node() {
			Nodes::Scope { children, .. } => children.clone(),
			other => other.children().cloned().collect(),
		}
	};
	for child in &children {
		erase_layer_parameters(child);
	}
}
