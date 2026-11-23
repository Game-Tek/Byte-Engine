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
pub(super) fn layer_path(program: &Program, chain: &[NodeId], name: &str) -> String {
	let mut path = String::new();
	for scope in chain {
		let scope = &program[scope];
		let Nodes::Scope { name: module, .. } = scope.node() else {
			continue;
		};
		match program.text(*module) {
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
pub(super) fn is_layer_parameter(program: &Program, parameter: &NodeId) -> bool {
	match program[parameter].node() {
		Nodes::Parameter { r#type, .. } => matches!(program[r#type].node(), Nodes::Layer { .. }),
		_ => false,
	}
}

/// Returns the layer `expression` is a value of, when it names a layer parameter.
pub(super) fn layer_of(program: &Program, expression: &NodeId) -> Option<NodeId> {
	let expression = &program[expression];
	let Nodes::Expression(Expressions::Member { source, .. }) = expression.node() else {
		return None;
	};
	let source = &program[source];
	match source.node() {
		Nodes::Parameter { r#type, .. } if matches!(program[r#type].node(), Nodes::Layer { .. }) => Some(*r#type),
		_ => None,
	}
}

/// Links `layer.name`: a read of the layer's member `name`, under the member's own name, as backends emit it.
pub(super) fn member_access(program: &mut Program, layer: &NodeId, name: &str) -> Result<NodeId, LexError> {
	let layer = &program[layer];
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
			.find(|member| matches!(program[*member].node(), Nodes::PushConstant { .. }))
			.cloned()
	} else {
		// A name no node carries names no member.
		program.find_name(name).and_then(|name| resolution::find_named_child(program, members, name))
	};
	let member = member.ok_or_else(|| {
		let layer_name = program.text(*layer_name);
		LexError::invalid(format!(
			"`{name}` isn't a member of layer `{layer_name}`. The most likely cause is a misspelled member name. Use one of the descriptors, the push constant block, or the tile targets that `{layer_name}` declares."
		))
	})?;

	let emitted = match program[member].get_name() {
		Some(emitted) => emitted,
		None => program.intern(name),
	};
	Ok(program.add(Node::expression(Expressions::Member {
		source: member,
		name: emitted,
	})))
}

/// Notes on each layer which of `main`'s layer parameters takes it, and checks that the layers `main` takes and the
/// program's own declarations hold at most one push constant block between them.
pub(super) fn record_entry_layers(program: &mut Program, root: &NodeId) -> Result<(), LexError> {
	let Some(main) = program.get_main() else {
		return Ok(());
	};
	let Nodes::Function { params, .. } = program[main].node() else {
		return Ok(());
	};

	// The program's own block is a direct child of the root.
	let own = match program[root].node() {
		Nodes::Scope { children, .. } => children
			.iter()
			.any(|child| matches!(program[child].node(), Nodes::PushConstant { .. })),
		_ => false,
	};
	let mut push_constant_owner = own.then(|| "the shader".to_string());

	// Read which layers `main` takes before recording the entry order on each of them.
	let layers = params
		.iter()
		.filter_map(|parameter| match program[parameter].node() {
			Nodes::Parameter { r#type, .. } if matches!(program[r#type].node(), Nodes::Layer { .. }) => Some(*r#type),
			_ => None,
		})
		.collect::<Vec<_>>();
	for (index, layer) in layers.into_iter().enumerate() {
		let Nodes::Layer {
			path,
			push_constant,
			entry,
			..
		} = program[layer].node_mut()
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
pub(super) fn erase_layer_parameters(program: &mut Program, node: &NodeId) {
	// Decide what to keep while only reading the program, and write a list back only when it loses an entry.
	match program[node].node() {
		Nodes::Function { params, .. } if params.iter().any(|parameter| is_layer_parameter(program, parameter)) => {
			let kept = params
				.iter()
				.copied()
				.filter(|parameter| !is_layer_parameter(program, parameter))
				.collect();
			if let Nodes::Function { params, .. } = program[node].node_mut() {
				*params = kept;
			}
		}
		Nodes::Expression(Expressions::FunctionCall { parameters, .. })
			if parameters.iter().any(|argument| layer_of(program, argument).is_some()) =>
		{
			let kept = parameters
				.iter()
				.copied()
				.filter(|argument| layer_of(program, argument).is_none())
				.collect();
			if let Nodes::Expression(Expressions::FunctionCall { parameters, .. }) = program[node].node_mut() {
				*parameters = kept;
			}
		}
		_ => {}
	}
	let children = match program[node].node() {
		Nodes::Scope { children, .. } => children.clone(),
		other => other.children().copied().collect(),
	};
	for child in &children {
		erase_layer_parameters(program, child);
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
pub fn place_descriptors(program: &mut Program) -> Result<(), LexError> {
	/// The order key of a set, its layout, and its bindings.
	type Set = ((u8, usize), Option<String>, Vec<NodeId>);

	/// Adds the bindings under `node` to `sets`, and the bindings that form no set to `unplaced`. `layer` is the
	/// enclosing layer's path and its order among the program's sets, or `None` when that layer forms no set of the
	/// program.
	fn visit(
		program: &Program,
		node: &NodeId,
		layer: Option<(&str, Option<usize>)>,
		has_main: bool,
		layers: &mut usize,
		sets: &mut Vec<Set>,
		unplaced: &mut Vec<NodeId>,
	) {
		match program[node].node() {
			Nodes::Scope { children, .. } => {
				for child in children {
					visit(program, child, None, has_main, layers, sets, unplaced);
				}
			}
			Nodes::Layer {
				path, members, entry, ..
			} => {
				// Without a `main`, the layers form their sets in declaration order.
				let order = if has_main { *entry } else { Some(*layers) };
				*layers += 1;
				for member in members {
					visit(program, member, Some((path, order)), has_main, layers, sets, unplaced);
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
				let Some(key) = key else {
					unplaced.push(*node);
					return;
				};
				match sets.iter_mut().find(|(_, existing, _)| existing == layout) {
					Some((_, _, bindings)) => bindings.push(*node),
					None => sets.push((key, layout.clone(), vec![*node])),
				}
			}
			_ => {}
		}
	}

	let root = program.root();
	let mut sets = Vec::<Set>::new();
	let mut unplaced = Vec::new();
	let has_main = program.get_main().is_some();
	visit(program, &root, None, has_main, &mut 0, &mut sets, &mut unplaced);
	for binding in unplaced {
		if let Nodes::Binding { placement, .. } = program[binding].node_mut() {
			*placement = None;
		}
	}
	sets.sort_by_key(|(key, ..)| *key);

	for (set, (_, layout, bindings)) in sets.iter_mut().enumerate() {
		let set_name = layout.as_deref().unwrap_or("the shader's own descriptors");
		bindings.sort_by_key(|binding| match program[binding].node() {
			Nodes::Binding { slot, .. } => *slot,
			_ => unreachable!("Only bindings are grouped into sets"),
		});

		let mut placement = Placement {
			set: set as u32,
			..Placement::default()
		};
		for (index, binding) in bindings.iter().enumerate() {
			let Nodes::Binding {
				name,
				slot,
				r#type,
				count,
				..
			} = program[binding].node()
			else {
				unreachable!("Only bindings are grouped into sets");
			};
			let (name, slot, count) = (*name, *slot, count.map_or(1, |count| count.get()));
			let takes_sampler = matches!(r#type, BindingTypes::CombinedImageSampler { sampler: None, .. });
			if slot != index as u32 {
				let name = program.text(name);
				return Err(LexError::invalid(format!(
					"Descriptor `{name}` of descriptor set `{set_name}` takes binding {slot}, where the set's next free number is {index}. The most likely cause is a binding number that repeats another one or skips a number. Number the bindings of each set from 0 without gaps or repeats."
				)));
			}
			if let Nodes::Binding { placement: target, .. } = program[binding].node_mut() {
				*target = Some(placement);
			}

			let name = program.text(name);
			let overflow = || {
				LexError::invalid(format!(
					"Descriptor set `{set_name}` holds more descriptors than fit in 32 bits at `{name}`. The most likely cause is a descriptor `count` far larger than intended. Lower the counts of the set's arrays."
				))
			};
			placement.resource = placement.resource.checked_add(count).ok_or_else(overflow)?;
			if takes_sampler {
				placement.sampler = placement.sampler.checked_add(count).ok_or_else(overflow)?;
			}
		}
	}

	Ok(())
}
