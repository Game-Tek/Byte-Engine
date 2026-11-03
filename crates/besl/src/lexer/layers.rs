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

/// Places every binding of the program at `root` in the program's descriptor sets, and records the result on each
/// binding as its [`Placement`].
///
/// The sets are the layers `main` takes, in parameter order, then the sets backends add (bindings whose layout names
/// no layer `main` takes, such as tile outputs), in the order the program first declares one of their bindings, and
/// last the program's own descriptors. A layer without descriptors forms no set. Without a `main`, every layer forms
/// its set, in declaration order. A binding of a layer `main` doesn't take gets no placement. Each set numbers its
/// bindings from 0 without gaps, and stores them densely in that order.
///
/// Linking calls it. Call it again after adding or replacing bindings in a linked program, as tile lowering does.
pub fn place_descriptors(root: &NodeReference) -> Result<(), LexError> {
	/// The order key of a set, its layout, and its bindings.
	type Set = ((u8, usize), Option<String>, Vec<NodeReference>);

	/// Adds the bindings under `node` to `sets`. `layer` is the enclosing layer's path and its order among the
	/// program's sets, or `None` when that layer forms no set of the program.
	fn visit(
		node: &NodeReference,
		layer: Option<(&str, Option<usize>)>,
		has_main: bool,
		layers: &mut usize,
		sets: &mut Vec<Set>,
	) {
		let borrowed = node.borrow();
		match borrowed.node() {
			Nodes::Scope { children, .. } => {
				for child in children {
					visit(child, None, has_main, layers, sets);
				}
			}
			Nodes::Layer {
				path, members, entry, ..
			} => {
				// Without a `main`, the layers form their sets in declaration order.
				let order = if has_main { *entry } else { Some(*layers) };
				*layers += 1;
				for member in members {
					visit(member, Some((path, order)), has_main, layers, sets);
				}
			}
			Nodes::Binding { layout, .. } => {
				let key = match (layout.as_deref(), layer) {
					(None, _) => Some((2, 0)),
					// A layer `main` doesn't take forms no set.
					(Some(layout), Some((path, order))) if layout == path => order.map(|order| (0, order)),
					// A layout no layer declares is one a backend added; the first binding declared orders it.
					(Some(_), _) => Some((1, sets.len())),
				};
				drop(borrowed);
				let Some(key) = key else {
					if let Nodes::Binding { placement, .. } = node.borrow_mut().node_mut() {
						*placement = None;
					}
					return;
				};
				let layout = match node.borrow().node() {
					Nodes::Binding { layout, .. } => layout.clone(),
					_ => unreachable!("The node was matched as a binding above"),
				};
				match sets.iter_mut().find(|(_, existing, _)| *existing == layout) {
					Some((_, _, bindings)) => bindings.push(node.clone()),
					None => sets.push((key, layout, vec![node.clone()])),
				}
			}
			_ => {}
		}
	}

	let mut sets = Vec::<Set>::new();
	visit(root, None, root.get_main().is_some(), &mut 0, &mut sets);
	sets.sort_by_key(|(key, ..)| *key);

	for (set, (_, layout, bindings)) in sets.iter_mut().enumerate() {
		let set_name = layout.as_deref().unwrap_or("the shader's own descriptors");
		let slot = |binding: &NodeReference| match binding.borrow().node() {
			Nodes::Binding { slot, .. } => *slot,
			_ => unreachable!("Only bindings are grouped into sets"),
		};
		bindings.sort_by_key(slot);

		let mut placement = Placement {
			set: set as u32,
			..Placement::default()
		};
		for (index, binding) in bindings.iter().enumerate() {
			let mut binding = binding.borrow_mut();
			let Nodes::Binding {
				name,
				slot,
				r#type,
				count,
				placement: target,
				..
			} = binding.node_mut()
			else {
				unreachable!("Only bindings are grouped into sets");
			};
			if *slot != index as u32 {
				return Err(LexError::invalid(format!(
					"Descriptor `{name}` of descriptor set `{set_name}` takes binding {slot}, where the set's next free number is {index}. The most likely cause is a binding number that repeats another one or skips a number. Number the bindings of each set from 0 without gaps or repeats."
				)));
			}
			*target = Some(placement);

			let count = count.map_or(1, |count| count.get());
			let overflow = || {
				LexError::invalid(format!(
					"Descriptor set `{set_name}` holds more descriptors than fit in 32 bits at `{name}`. The most likely cause is a descriptor `count` far larger than intended. Lower the counts of the set's arrays."
				))
			};
			placement.resource = placement.resource.checked_add(count).ok_or_else(overflow)?;
			if matches!(r#type, BindingTypes::CombinedImageSampler { sampler: None, .. }) {
				placement.sampler = placement.sampler.checked_add(count).ok_or_else(overflow)?;
			}
		}
	}

	Ok(())
}
