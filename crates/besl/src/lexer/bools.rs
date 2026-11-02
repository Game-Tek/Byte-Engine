//! Packs `bool` members of structs, buffer blocks, and push-constant blocks into bits.
//!
//! A `bool` has no byte representation that every backend shares, so a linked program never stores one. Each run of
//! contiguous `bool` members becomes `ceil(N / 8)` hidden `u8` members, and every read, write, and construction of a
//! packed member becomes bit arithmetic on its byte. Backends, reflection, and the VM then see ordinary `u8` members and
//! agree on the bytes.

use std::collections::{HashMap, HashSet};

use super::ast::{CallTarget, Expressions, LexError, Node, NodeReference, Nodes, Operators};

const BOOL_MEMBERS_DOCS_PATH: &str = "reference/besl/language#bool-members";

/// The prefix of the hidden members that hold packed `bool` members.
const PACKED_MEMBER_PREFIX: &str = "_besl_bits";

/// The `PackedBit` struct locates one packed `bool` member: the byte member that holds it and its bit in that byte.
struct PackedBit {
	byte: NodeReference,
	mask: u32,
}

/// The `ConstructorArgument` enum says where one argument of a packed struct's constructor comes from.
enum ConstructorArgument {
	/// The argument at this index in the original constructor, unchanged.
	Keep(usize),
	/// A byte assembled from the `bool` arguments at these indices, each with the bit it sets.
	Byte(Vec<(usize, u32)>),
}

/// The `Packing` struct is everything the rewrite needs: where each packed member lives, how each packed struct's
/// constructor maps its arguments, and the conversions that move a byte to and from `u32`.
struct Packing {
	bits: HashMap<usize, PackedBit>,
	constructors: HashMap<usize, Vec<ConstructorArgument>>,
	/// The `u32(u8)` and `u8(u32)` conversions, looked up only when the program has `bool` members to pack.
	conversions: Option<(NodeReference, NodeReference)>,
}

impl Packing {
	fn to_u32(&self) -> &NodeReference {
		&self
			.conversions
			.as_ref()
			.expect("Packed members exist, so the conversions were found")
			.0
	}

	fn to_u8(&self) -> &NodeReference {
		&self
			.conversions
			.as_ref()
			.expect("Packed members exist, so the conversions were found")
			.1
	}
}

/// Packs every `bool` member under `root` into bits, after the program is linked, and rejects writes to byte-sized
/// members that would evaluate their place twice.
///
/// The pass runs once per linked graph: a second run finds no `bool` members left, so it changes nothing.
pub(super) fn pack_bool_members(root: &NodeReference) -> Result<(), LexError> {
	let mut containers = Vec::new();
	collect_containers(root, &mut containers);

	let mut packing = Packing {
		bits: HashMap::new(),
		constructors: HashMap::new(),
		conversions: None,
	};
	let mut packed_members = Vec::with_capacity(containers.len());
	if !containers.is_empty() {
		let u8_type = find_builtin(
			root,
			|node| matches!(node, Nodes::Struct { name, fields, .. } if name == "u8" && fields.is_empty()),
		)?;
		packing.conversions = Some((find_conversion(root, "u32", "u8")?, find_conversion(root, "u8", "u32")?));
		for container in &containers {
			packed_members.push(plan_container(container, &u8_type, &mut packing)?);
		}
	}

	let mut visited = HashSet::new();
	rewrite_declarations(root, &packing, &mut visited)?;

	// Swap the member lists last: the rewrite resolves accesses through the original `bool` members.
	for (container, members) in containers.iter().zip(packed_members) {
		match container.borrow_mut().node_mut() {
			Nodes::Struct { fields: list, .. } | Nodes::PushConstant { members: list } => *list = members,
			Nodes::Binding {
				r#type: super::ast::BindingTypes::Buffer { members: list },
				..
			} => *list = members,
			_ => unreachable!("Only member containers are collected"),
		}
	}
	Ok(())
}

/// Collects every struct, buffer block, and push-constant block under `scope` that declares a `bool` member, including
/// the members of layers.
fn collect_containers(scope: &NodeReference, containers: &mut Vec<NodeReference>) {
	let node = scope.borrow();
	let (Nodes::Scope { children, .. } | Nodes::Layer { members: children, .. }) = node.node() else {
		return;
	};
	for child in children {
		let has_bool = match child.borrow().node() {
			Nodes::Scope { .. } | Nodes::Layer { .. } => {
				collect_containers(child, containers);
				false
			}
			// Templated structs are arrays and generic handles; a `bool` array member is rejected where it is declared.
			Nodes::Struct {
				template: None, fields, ..
			}
			| Nodes::PushConstant { members: fields }
			| Nodes::Binding {
				r#type: super::ast::BindingTypes::Buffer { members: fields },
				..
			} => fields
				.iter()
				.any(|member| member_is_bool(member) || member_is_bool_array(member)),
			_ => false,
		};
		if has_bool {
			containers.push(child.clone());
		}
	}
}

/// Plans one container's packed members: groups its `bool` runs into bytes, records where each `bool` landed, and
/// returns the container's new member list.
fn plan_container(
	container: &NodeReference,
	u8_type: &NodeReference,
	packing: &mut Packing,
) -> Result<Vec<NodeReference>, LexError> {
	let node = container.borrow();
	let (container_name, members, is_struct) = match node.node() {
		Nodes::Struct { name, fields, .. } => (format!("Struct `{name}`"), fields, true),
		Nodes::PushConstant { members } => ("The push constant".to_string(), members, false),
		Nodes::Binding {
			name,
			r#type: super::ast::BindingTypes::Buffer { members },
			..
		} => (format!("Buffer `{name}`"), members, false),
		_ => unreachable!("Only member containers are collected"),
	};

	let mut packed = Vec::with_capacity(members.len());
	let mut constructor = Vec::with_capacity(members.len());
	// The bit the next `bool` of the current run takes, and the byte it goes into, or `None` between runs.
	let mut run: Option<(NodeReference, u32)> = None;
	for (index, member) in members.iter().enumerate() {
		let member_name = member.borrow().get_name().unwrap_or_default().to_string();
		if member_name.starts_with(PACKED_MEMBER_PREFIX) {
			return Err(LexError::invalid(format!(
				"{container_name} declares `{member_name}`. BESL reserves names starting with `{PACKED_MEMBER_PREFIX}` for the bytes that hold `bool` members. Rename the member. See {}.",
				crate::online_docs_url(BOOL_MEMBERS_DOCS_PATH)
			)));
		}
		if member_is_bool_array(member) {
			return Err(LexError::invalid(format!(
				"{container_name} member `{member_name}` is a `bool` array. BESL stores `bool` members as single bits and has no bit arrays. Declare separate `bool` members, or keep the flags in a `u32` and test them with `&`. See {}.",
				crate::online_docs_url(BOOL_MEMBERS_DOCS_PATH)
			)));
		}
		if !member_is_bool(member) {
			run = None;
			packed.push(member.clone());
			constructor.push(ConstructorArgument::Keep(index));
			continue;
		}

		// Start a new byte at the start of a run and after every eighth `bool` of it.
		let (byte, bit) = match run.take() {
			Some((byte, bit)) if bit < 8 => (byte, bit),
			_ => {
				let byte: NodeReference =
					Node::member(&format!("{PACKED_MEMBER_PREFIX}{}", packed.len()), u8_type.clone()).into();
				packed.push(byte.clone());
				constructor.push(ConstructorArgument::Byte(Vec::new()));
				(byte, 0)
			}
		};
		let mask = 1 << bit;
		if let Some(ConstructorArgument::Byte(bits)) = constructor.last_mut() {
			bits.push((index, mask));
		}
		packing.bits.insert(
			member.identity(),
			PackedBit {
				byte: byte.clone(),
				mask,
			},
		);
		run = Some((byte, bit + 1));
	}

	if is_struct {
		packing.constructors.insert(container.identity(), constructor);
	}
	Ok(packed)
}

/// Rewrites the bodies of every function and constant under `scope`.
fn rewrite_declarations(scope: &NodeReference, packing: &Packing, visited: &mut HashSet<usize>) -> Result<(), LexError> {
	let children = match scope.borrow().node() {
		Nodes::Scope { children, .. } => children.clone(),
		_ => return Ok(()),
	};
	for child in &children {
		let is_body = match child.borrow().node() {
			Nodes::Scope { .. } => {
				rewrite_declarations(child, packing, visited)?;
				false
			}
			Nodes::Function { .. } | Nodes::Const { .. } => true,
			_ => false,
		};
		if is_body {
			rewrite(child, packing, visited)?;
		}
	}
	Ok(())
}

/// Rewrites one node in place, then its parts. Each node is visited once, since call arguments and their expanded
/// elements share nodes.
fn rewrite(node: &NodeReference, packing: &Packing, visited: &mut HashSet<usize>) -> Result<(), LexError> {
	if !visited.insert(node.identity()) {
		return Ok(());
	}

	let replacement = match node.borrow().node() {
		// A write replaces the whole assignment, before its place would be rewritten as a read.
		Nodes::Expression(Expressions::Operator {
			operator: Operators::Assignment,
			left,
			right,
		}) => {
			reject_place_with_call(left)?;
			match packed_place(left, packing) {
				Some((base, bit)) => {
					reject_write_only_buffer(left, base.as_ref())?;
					Some(write_bit(base, bit, right, packing))
				}
				None => None,
			}
		}
		Nodes::Expression(Expressions::Accessor { .. } | Expressions::Member { .. }) => {
			packed_place(node, packing).map(|(base, bit)| read_bit(base, bit, packing))
		}
		Nodes::Expression(Expressions::FunctionCall { function, parameters }) => packing
			.constructors
			.get(&function.get().identity())
			.map(|plan| construct(function, parameters, plan, packing)),
		_ => None,
	};
	if let Some(replacement) = replacement {
		*node.borrow_mut() = replacement;
	}

	let parts = node.borrow().node().children().cloned().collect::<Vec<_>>();
	for part in &parts {
		rewrite(part, packing, visited)?;
	}
	Ok(())
}

/// Returns the base a packed `bool` is read through, such as `values[i]` in `values[i].flag`, or `None` for a bare
/// member, together with where the `bool` is packed. Returns `None` when `place` reads no packed `bool`.
fn packed_place<'a>(place: &NodeReference, packing: &'a Packing) -> Option<(Option<NodeReference>, &'a PackedBit)> {
	match place.borrow().node() {
		Nodes::Expression(Expressions::Accessor { left, right }) => match right.borrow().node() {
			Nodes::Expression(Expressions::Member { source, .. }) => {
				packing.bits.get(&source.identity()).map(|bit| (Some(left.clone()), bit))
			}
			_ => None,
		},
		Nodes::Expression(Expressions::Member { source, .. }) => packing.bits.get(&source.identity()).map(|bit| (None, bit)),
		_ => None,
	}
}

/// The byte that holds `bit`, read through `base` when there is one.
fn byte_place(base: Option<NodeReference>, bit: &PackedBit) -> NodeReference {
	let byte = expression(Expressions::Member {
		name: bit.byte.borrow().get_name().unwrap_or_default().to_string(),
		source: bit.byte.clone(),
	});
	match base {
		Some(left) => expression(Expressions::Accessor { left, right: byte }),
		None => byte,
	}
}

/// Builds `(u32(byte) & mask) != 0`.
fn read_bit(base: Option<NodeReference>, bit: &PackedBit, packing: &Packing) -> Node {
	let byte = convert(packing.to_u32(), byte_place(base, bit));
	let masked = operator(Operators::BitwiseAnd, byte, literal(bit.mask));
	Node::expression(Expressions::Operator {
		operator: Operators::Inequality,
		left: masked,
		right: literal(0),
	})
}

/// Builds `byte = u8((u32(byte) & ~mask) | (value ? mask : 0))`. The base is shared by both sides, so it must not
/// have side effects; [`reject_place_with_call`] guarantees that.
fn write_bit(base: Option<NodeReference>, bit: &PackedBit, value: &NodeReference, packing: &Packing) -> Node {
	let target = byte_place(base.clone(), bit);
	let current = convert(packing.to_u32(), byte_place(base, bit));
	let kept = operator(Operators::BitwiseAnd, current, literal(0xFF ^ bit.mask));
	let stored = operator(Operators::BitwiseOr, kept, bit_value(value, bit.mask));
	Node::expression(Expressions::Operator {
		operator: Operators::Assignment,
		left: target,
		right: convert(packing.to_u8(), stored),
	})
}

/// Builds a packed struct's constructor call: each byte of `bool` arguments becomes `u8(bit_a | bit_b | …)`, and every
/// other argument passes through. Arguments keep their order, so each is evaluated once, left to right.
fn construct(function: &CallTarget, parameters: &[NodeReference], plan: &[ConstructorArgument], packing: &Packing) -> Node {
	let parameters = plan
		.iter()
		.map(|argument| match argument {
			ConstructorArgument::Keep(index) => parameters[*index].clone(),
			ConstructorArgument::Byte(bits) => {
				let byte = bits
					.iter()
					.map(|(index, mask)| bit_value(&parameters[*index], *mask))
					.reduce(|byte, bit| operator(Operators::BitwiseOr, byte, bit))
					.expect("A packed byte holds at least one bool");
				convert(packing.to_u8(), byte)
			}
		})
		.collect();
	Node::expression(Expressions::FunctionCall {
		function: function.clone(),
		parameters,
	})
}

/// Builds `value ? mask : 0` as a `u32`, folding a literal `true` or `false`.
fn bit_value(value: &NodeReference, mask: u32) -> NodeReference {
	match value.borrow().node() {
		Nodes::Expression(Expressions::Literal { value }) if value == "true" => return literal(mask),
		Nodes::Expression(Expressions::Literal { value }) if value == "false" => return literal(0),
		_ => {}
	}
	expression(Expressions::Ternary {
		condition: value.clone(),
		if_true: literal(mask),
		if_false: literal(0),
	})
}

/// Rejects a write to a byte-sized member through a place whose index calls a function or an atomic.
///
/// Writing a byte-sized member reads and rewrites the byte, or on some backends the word, that holds it, so the place
/// is evaluated twice, and so would the call.
fn reject_place_with_call(place: &NodeReference) -> Result<(), LexError> {
	let (base, member) = match place.borrow().node() {
		Nodes::Expression(Expressions::Accessor { left, right }) => match right.borrow().node() {
			Nodes::Expression(Expressions::Member { source, name }) => (left.clone(), (source.clone(), name.clone())),
			_ => return Ok(()),
		},
		_ => return Ok(()),
	};
	let byte_sized = match member.0.borrow().node() {
		Nodes::Member { r#type, count: None, .. } => matches!(r#type.borrow().get_name(), Some("bool" | "u8")),
		_ => false,
	};
	if !byte_sized {
		return Ok(());
	}
	if let Some(call) = side_effecting_call(&base) {
		return Err(LexError::invalid(format!(
			"Member `{}` is written through a place that calls `{call}`. Writing a `bool` or `u8` member reads and rewrites the bytes around it, so the call would run twice. Store the index in a `let` first and index with that local. See {}.",
			member.1,
			crate::online_docs_url(BOOL_MEMBERS_DOCS_PATH)
		)));
	}
	Ok(())
}

/// Rejects a write to a packed `bool` in a buffer that shaders can't read, since the write reads its byte first.
fn reject_write_only_buffer(place: &NodeReference, base: Option<&NodeReference>) -> Result<(), LexError> {
	let Some(binding) = base.and_then(written_binding) else {
		return Ok(());
	};
	let binding = binding.borrow();
	let Nodes::Binding { name, read: false, .. } = binding.node() else {
		return Ok(());
	};
	let member = match place.borrow().node() {
		Nodes::Expression(Expressions::Accessor { right, .. }) => right.borrow().get_name().unwrap_or_default().to_string(),
		_ => String::new(),
	};
	Err(LexError::invalid(format!(
		"`bool` member `{member}` is written to buffer `{name}`, which is write-only. Writing a `bool` member reads and rewrites the byte that holds it. Declare the buffer `access: read_write`. See {}.",
		crate::online_docs_url(BOOL_MEMBERS_DOCS_PATH)
	)))
}

/// Returns the binding a place such as `values[i].record` is rooted in, or `None` when it is rooted in a local.
fn written_binding(place: &NodeReference) -> Option<NodeReference> {
	match place.borrow().node() {
		Nodes::Binding { .. } => Some(place.clone()),
		Nodes::Expression(Expressions::Member { source, .. }) => {
			matches!(source.borrow().node(), Nodes::Binding { .. }).then(|| source.clone())
		}
		Nodes::Expression(Expressions::Accessor { left, .. }) => written_binding(left),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => written_binding(&elements[0]),
		_ => None,
	}
}

/// Returns the name of the first function or atomic that `expression` calls.
fn side_effecting_call(expression: &NodeReference) -> Option<String> {
	let node = expression.borrow();
	match node.node() {
		Nodes::Expression(Expressions::FunctionCall { function, .. }) => {
			let target = function.get();
			let target = target.borrow();
			if let Nodes::Function { name, .. } = target.node() {
				return Some(name.clone());
			}
		}
		Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
			if let Some(name) = intrinsic.borrow().get_name().filter(|name| name.starts_with("atomic_")) {
				return Some(name.to_string());
			}
		}
		_ => {}
	}
	node.node().children().find_map(side_effecting_call)
}

/// Whether `member` is a single `bool` member.
fn member_is_bool(member: &NodeReference) -> bool {
	matches!(member.borrow().node(), Nodes::Member { r#type, count: None, .. } if r#type.borrow().get_name() == Some("bool"))
}

/// Whether `member` is an array of `bool`, declared with a count or as a fixed `bool[N]` type.
fn member_is_bool_array(member: &NodeReference) -> bool {
	match member.borrow().node() {
		Nodes::Member { r#type, count, .. } => {
			let r#type = r#type.borrow();
			match r#type.node() {
				Nodes::Struct { name, .. } if name == "bool" => count.is_some(),
				Nodes::Struct {
					template: Some(element), ..
				} => element.borrow().get_name() == Some("bool"),
				_ => false,
			}
		}
		_ => false,
	}
}

/// Finds the built-in declaration under `root` that `matches` accepts.
fn find_builtin(root: &NodeReference, matches: impl Fn(&Nodes) -> bool) -> Result<NodeReference, LexError> {
	let node = root.borrow();
	let Nodes::Scope { children, .. } = node.node() else {
		return Err(missing_builtins());
	};
	children
		.iter()
		.find(|child| matches(child.borrow().node()))
		.cloned()
		.ok_or_else(missing_builtins)
}

/// Finds the conversion intrinsic `name` whose one parameter has type `source`.
fn find_conversion(root: &NodeReference, name: &str, source: &str) -> Result<NodeReference, LexError> {
	find_builtin(root, |node| match node {
		Nodes::Intrinsic {
			name: intrinsic,
			elements,
			..
		} if intrinsic == name => elements.iter().any(
			|element| matches!(element.borrow().node(), Nodes::Parameter { r#type, .. } if r#type.borrow().get_name() == Some(source)),
		),
		_ => false,
	})
}

fn missing_builtins() -> LexError {
	LexError::invalid(
		"`bool` members can't be packed because the program has no built-in `u8` conversions. The most likely cause is a parent scope that was not built from `Node::root`; build it from `Node::root`.",
	)
}

fn expression(expression: Expressions) -> NodeReference {
	Node::expression(expression).into()
}

fn literal(value: u32) -> NodeReference {
	expression(Expressions::Literal {
		value: value.to_string(),
	})
}

fn operator(operator: Operators, left: NodeReference, right: NodeReference) -> NodeReference {
	expression(Expressions::Operator { operator, left, right })
}

/// Calls the conversion intrinsic `intrinsic` on `value`. Conversions have no body, so the call's elements are its
/// argument.
fn convert(intrinsic: &NodeReference, value: NodeReference) -> NodeReference {
	expression(Expressions::IntrinsicCall {
		intrinsic: intrinsic.clone(),
		arguments: vec![value.clone()],
		elements: vec![value],
	})
}
