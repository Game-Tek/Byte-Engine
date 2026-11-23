//! Packs `bool` members of structs, buffer blocks, and push-constant blocks into bits.
//!
//! A `bool` has no byte representation that every backend shares, so a linked program never stores one. Each run of
//! contiguous `bool` members becomes `ceil(N / 8)` hidden `u8` members, and every read, write, and construction of a
//! packed member becomes bit arithmetic on its byte. Backends, reflection, and the VM then see ordinary `u8` members and
//! agree on the bytes.

use std::collections::{HashMap, HashSet};

use super::ast::{Expressions, LexError, Node, NodeId, Nodes, Operators, Program};

const BOOL_MEMBERS_DOCS_PATH: &str = "reference/besl/language#bool-members";

/// The prefix of the hidden members that hold packed `bool` members.
const PACKED_MEMBER_PREFIX: &str = "_besl_bits";

/// The `PackedBit` struct locates one packed `bool` member: the byte member that holds it and its bit in that byte.
struct PackedBit {
	byte: NodeId,
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
	conversions: Option<(NodeId, NodeId)>,
}

impl Packing {
	fn to_u32(&self) -> NodeId {
		self
			.conversions
			.as_ref()
			.expect("Packed members exist, so the conversions were found")
			.0
	}

	fn to_u8(&self) -> NodeId {
		self
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
pub(super) fn pack_bool_members(program: &mut Program, root: &NodeId) -> Result<(), LexError> {
	let mut containers = Vec::new();
	collect_containers(program, root, &mut containers);

	let mut packing = Packing {
		bits: HashMap::new(),
		constructors: HashMap::new(),
		conversions: None,
	};
	let mut packed_members = Vec::with_capacity(containers.len());
	if !containers.is_empty() {
		let u8_type = find_builtin(
			program,
			root,
			|node| matches!(node, Nodes::Struct { name, fields, .. } if name == "u8" && fields.is_empty()),
		)?;
		packing.conversions = Some((find_conversion(program, root, "u32", "u8")?, find_conversion(program, root, "u8", "u32")?));
		for container in &containers {
			packed_members.push(plan_container(program, container, &u8_type, &mut packing)?);
		}
	}

	let mut visited = HashSet::new();
	rewrite_declarations(program, root, &packing, &mut visited)?;

	// Swap the member lists last: the rewrite resolves accesses through the original `bool` members.
	for (container, members) in containers.iter().zip(packed_members) {
		match program[container].node_mut() {
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
fn collect_containers(program: &Program, scope: &NodeId, containers: &mut Vec<NodeId>) {
	let node = &program[scope];
	let (Nodes::Scope { children, .. } | Nodes::Layer { members: children, .. }) = node.node() else {
		return;
	};
	for child in children {
		let has_bool = match program[child].node() {
			Nodes::Scope { .. } | Nodes::Layer { .. } => {
				collect_containers(program, child, containers);
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
				.any(|member| member_is_bool(program, member) || member_is_bool_array(program, member)),
			_ => false,
		};
		if has_bool {
			containers.push(*child);
		}
	}
}

/// Plans one container's packed members: groups its `bool` runs into bytes, records where each `bool` landed, and
/// returns the container's new member list.
fn plan_container(
	program: &mut Program,
	container: &NodeId,
	u8_type: &NodeId,
	packing: &mut Packing,
) -> Result<Vec<NodeId>, LexError> {
	// Copy the member list, so packed bytes can be added to the program while the members are walked. Only containers
	// that hold a `bool` get here.
	let (container_name, members, is_struct) = match program[container].node() {
		Nodes::Struct { name, fields, .. } => (format!("Struct `{name}`"), fields.clone(), true),
		Nodes::PushConstant { members } => ("The push constant".to_string(), members.clone(), false),
		Nodes::Binding {
			name,
			r#type: super::ast::BindingTypes::Buffer { members },
			..
		} => (format!("Buffer `{name}`"), members.clone(), false),
		_ => unreachable!("Only member containers are collected"),
	};

	let mut packed = Vec::with_capacity(members.len());
	let mut constructor = Vec::with_capacity(members.len());
	// The bit the next `bool` of the current run takes, and the byte it goes into, or `None` between runs.
	let mut run: Option<(NodeId, u32)> = None;
	for (index, member) in members.iter().enumerate() {
		let member_name = program[member].get_name().unwrap_or_default().to_string();
		if member_name.starts_with(PACKED_MEMBER_PREFIX) {
			return Err(LexError::invalid(format!(
				"{container_name} declares `{member_name}`. BESL reserves names starting with `{PACKED_MEMBER_PREFIX}` for the bytes that hold `bool` members. Rename the member. See {}.",
				crate::online_docs_url(BOOL_MEMBERS_DOCS_PATH)
			)));
		}
		if member_is_bool_array(program, member) {
			return Err(LexError::invalid(format!(
				"{container_name} member `{member_name}` is a `bool` array. BESL stores `bool` members as single bits and has no bit arrays. Declare separate `bool` members, or keep the flags in a `u32` and test them with `&`. See {}.",
				crate::online_docs_url(BOOL_MEMBERS_DOCS_PATH)
			)));
		}
		if !member_is_bool(program, member) {
			run = None;
			packed.push(*member);
			constructor.push(ConstructorArgument::Keep(index));
			continue;
		}

		// Start a new byte at the start of a run and after every eighth `bool` of it.
		let (byte, bit) = match run.take() {
			Some((byte, bit)) if bit < 8 => (byte, bit),
			_ => {
				let byte: NodeId =
					program.add(Node::member(&format!("{PACKED_MEMBER_PREFIX}{}", packed.len()), *u8_type));
				packed.push(byte);
				constructor.push(ConstructorArgument::Byte(Vec::new()));
				(byte, 0)
			}
		};
		let mask = 1 << bit;
		if let Some(ConstructorArgument::Byte(bits)) = constructor.last_mut() {
			bits.push((index, mask));
		}
		packing.bits.insert(
			member.index(),
			PackedBit {
				byte: byte,
				mask,
			},
		);
		run = Some((byte, bit + 1));
	}

	if is_struct {
		packing.constructors.insert(container.index(), constructor);
	}
	Ok(packed)
}

/// Rewrites the bodies of every function and constant under `scope`.
fn rewrite_declarations(program: &mut Program, scope: &NodeId, packing: &Packing, visited: &mut HashSet<usize>) -> Result<(), LexError> {
	let children = match program[scope].node() {
		Nodes::Scope { children, .. } => children.clone(),
		_ => return Ok(()),
	};
	for child in &children {
		let is_body = match program[child].node() {
			Nodes::Scope { .. } => {
				rewrite_declarations(program, child, packing, visited)?;
				false
			}
			Nodes::Function { .. } | Nodes::Const { .. } => true,
			_ => false,
		};
		if is_body {
			rewrite(program, child, packing, visited)?;
		}
	}
	Ok(())
}

/// Rewrites one node in place, then its parts. Each node is visited once, since call arguments and their expanded
/// elements share nodes.
fn rewrite(program: &mut Program, node: &NodeId, packing: &Packing, visited: &mut HashSet<usize>) -> Result<(), LexError> {
	if !visited.insert(node.index()) {
		return Ok(());
	}

	/// What the node needs, read out of it so the replacement can be built while the program grows.
	enum Plan<'p> {
		Write { left: NodeId, right: NodeId },
		Read,
		Construct {
			function: NodeId,
			parameters: Vec<NodeId>,
			plan: &'p [ConstructorArgument],
		},
	}
	let plan = match program[node].node() {
		// A write replaces the whole assignment, before its place would be rewritten as a read.
		Nodes::Expression(Expressions::Operator {
			operator: Operators::Assignment,
			left,
			right,
		}) => Some(Plan::Write { left: *left, right: *right }),
		Nodes::Expression(Expressions::Accessor { .. } | Expressions::Member { .. }) => Some(Plan::Read),
		Nodes::Expression(Expressions::FunctionCall { function, parameters }) => {
			packing.constructors.get(&function.index()).map(|plan| Plan::Construct {
				function: *function,
				parameters: parameters.clone(),
				plan,
			})
		}
		_ => None,
	};
	let replacement = match plan {
		Some(Plan::Write { left, right }) => {
			reject_place_with_call(program, &left)?;
			match packed_place(program, &left, packing) {
				Some((base, bit)) => {
					reject_write_only_buffer(program, &left, base.as_ref())?;
					Some(write_bit(program, base, bit, &right, packing))
				}
				None => None,
			}
		}
		Some(Plan::Read) => packed_place(program, node, packing).map(|(base, bit)| read_bit(program, base, bit, packing)),
		Some(Plan::Construct {
			function,
			parameters,
			plan,
		}) => Some(construct(program, &function, &parameters, plan, packing)),
		None => None,
	};
	if let Some(replacement) = replacement {
		program[node] = replacement;
	}

	let parts = program[node].node().children().cloned().collect::<Vec<_>>();
	for part in &parts {
		rewrite(program, part, packing, visited)?;
	}
	Ok(())
}

/// Returns the base a packed `bool` is read through, such as `values[i]` in `values[i].flag`, or `None` for a bare
/// member, together with where the `bool` is packed. Returns `None` when `place` reads no packed `bool`.
fn packed_place<'a>(program: &Program, place: &NodeId, packing: &'a Packing) -> Option<(Option<NodeId>, &'a PackedBit)> {
	match program[place].node() {
		Nodes::Expression(Expressions::Accessor { left, right }) => match program[right].node() {
			Nodes::Expression(Expressions::Member { source, .. }) => {
				packing.bits.get(&source.index()).map(|bit| (Some(*left), bit))
			}
			_ => None,
		},
		Nodes::Expression(Expressions::Member { source, .. }) => packing.bits.get(&source.index()).map(|bit| (None, bit)),
		_ => None,
	}
}

/// The byte that holds `bit`, read through `base` when there is one.
fn byte_place(program: &mut Program, base: Option<NodeId>, bit: &PackedBit) -> NodeId {
	let name = program[bit.byte].get_name().unwrap_or_default().to_string();
	let byte = expression(program, Expressions::Member { name, source: bit.byte });
	match base {
		Some(left) => expression(program, Expressions::Accessor { left, right: byte }),
		None => byte,
	}
}

/// Builds `(u32(byte) & mask) != 0`.
fn read_bit(program: &mut Program, base: Option<NodeId>, bit: &PackedBit, packing: &Packing) -> Node {
	let byte = byte_place(program, base, bit);
	let byte = convert(program, packing.to_u32(), byte);
	let mask = literal(program, bit.mask);
	let masked = operator(program, Operators::BitwiseAnd, byte, mask);
	let zero = literal(program, 0);
	Node::expression(Expressions::Operator {
		operator: Operators::Inequality,
		left: masked,
		right: zero,
	})
}

/// Builds `byte = u8((u32(byte) & ~mask) | (value ? mask : 0))`. The base is shared by both sides, so it must not
/// have side effects; [`reject_place_with_call`] guarantees that.
fn write_bit(program: &mut Program, base: Option<NodeId>, bit: &PackedBit, value: &NodeId, packing: &Packing) -> Node {
	let target = byte_place(program, base, bit);
	let current = byte_place(program, base, bit);
	let current = convert(program, packing.to_u32(), current);
	let keep_mask = literal(program, 0xFF ^ bit.mask);
	let kept = operator(program, Operators::BitwiseAnd, current, keep_mask);
	let value = bit_value(program, value, bit.mask);
	let stored = operator(program, Operators::BitwiseOr, kept, value);
	let stored = convert(program, packing.to_u8(), stored);
	Node::expression(Expressions::Operator {
		operator: Operators::Assignment,
		left: target,
		right: stored,
	})
}

/// Builds a packed struct's constructor call: each byte of `bool` arguments becomes `u8(bit_a | bit_b | …)`, and every
/// other argument passes through. Arguments keep their order, so each is evaluated once, left to right.
fn construct(program: &mut Program, function: &NodeId, parameters: &[NodeId], plan: &[ConstructorArgument], packing: &Packing) -> Node {
	let mut arguments = Vec::with_capacity(plan.len());
	for argument in plan {
		let argument = match argument {
			ConstructorArgument::Keep(index) => parameters[*index],
			ConstructorArgument::Byte(bits) => {
				let mut byte = None;
				for (index, mask) in bits {
					let bit = bit_value(program, &parameters[*index], *mask);
					byte = Some(match byte {
						Some(byte) => operator(program, Operators::BitwiseOr, byte, bit),
						None => bit,
					});
				}
				let byte = byte.expect("A packed byte holds at least one bool");
				convert(program, packing.to_u8(), byte)
			}
		};
		arguments.push(argument);
	}
	Node::expression(Expressions::FunctionCall {
		function: *function,
		parameters: arguments,
	})
}

/// Builds `value ? mask : 0` as a `u32`, folding a literal `true` or `false`.
fn bit_value(program: &mut Program, value: &NodeId, mask: u32) -> NodeId {
	match program[value].node() {
		Nodes::Expression(Expressions::Literal { value }) if value == "true" => return literal(program, mask),
		Nodes::Expression(Expressions::Literal { value }) if value == "false" => return literal(program, 0),
		_ => {}
	}
	let if_true = literal(program, mask);
	let if_false = literal(program, 0);
	expression(program, Expressions::Ternary {
		condition: *value,
		if_true,
		if_false,
	})
}

/// Rejects a write to a byte-sized member through a place whose index calls a function or an atomic.
///
/// Writing a byte-sized member reads and rewrites the byte, or on some backends the word, that holds it, so the place
/// is evaluated twice, and so would the call.
fn reject_place_with_call(program: &Program, place: &NodeId) -> Result<(), LexError> {
	let (base, member) = match program[place].node() {
		Nodes::Expression(Expressions::Accessor { left, right }) => match program[right].node() {
			Nodes::Expression(Expressions::Member { source, name }) => (*left, (*source, name.clone())),
			_ => return Ok(()),
		},
		_ => return Ok(()),
	};
	let byte_sized = match program[member.0].node() {
		Nodes::Member { r#type, count: None, .. } => matches!(program[r#type].get_name(), Some("bool" | "u8")),
		_ => false,
	};
	if !byte_sized {
		return Ok(());
	}
	if let Some(call) = side_effecting_call(program, &base) {
		return Err(LexError::invalid(format!(
			"Member `{}` is written through a place that calls `{call}`. Writing a `bool` or `u8` member reads and rewrites the bytes around it, so the call would run twice. Store the index in a `let` first and index with that local. See {}.",
			member.1,
			crate::online_docs_url(BOOL_MEMBERS_DOCS_PATH)
		)));
	}
	Ok(())
}

/// Rejects a write to a packed `bool` in a buffer that shaders can't read, since the write reads its byte first.
fn reject_write_only_buffer(program: &Program, place: &NodeId, base: Option<&NodeId>) -> Result<(), LexError> {
	let Some(binding) = base.and_then(|base| written_binding(program, base)) else {
		return Ok(());
	};
	let binding = &program[binding];
	let Nodes::Binding { name, read: false, .. } = binding.node() else {
		return Ok(());
	};
	let member = match program[place].node() {
		Nodes::Expression(Expressions::Accessor { right, .. }) => program[right].get_name().unwrap_or_default().to_string(),
		_ => String::new(),
	};
	Err(LexError::invalid(format!(
		"`bool` member `{member}` is written to buffer `{name}`, which is write-only. Writing a `bool` member reads and rewrites the byte that holds it. Declare the buffer `access: read_write`. See {}.",
		crate::online_docs_url(BOOL_MEMBERS_DOCS_PATH)
	)))
}

/// Returns the binding a place such as `values[i].record` is rooted in, or `None` when it is rooted in a local.
fn written_binding(program: &Program, place: &NodeId) -> Option<NodeId> {
	match program[place].node() {
		Nodes::Binding { .. } => Some(*place),
		Nodes::Expression(Expressions::Member { source, .. }) => {
			matches!(program[source].node(), Nodes::Binding { .. }).then(|| *source)
		}
		Nodes::Expression(Expressions::Accessor { left, .. }) => written_binding(program, left),
		Nodes::Expression(Expressions::Expression { elements }) if elements.len() == 1 => written_binding(program, &elements[0]),
		_ => None,
	}
}

/// Returns the name of the first function or atomic that `expression` calls.
fn side_effecting_call(program: &Program, expression: &NodeId) -> Option<String> {
	let node = &program[expression];
	match node.node() {
		Nodes::Expression(Expressions::FunctionCall { function, .. }) => {
			let target = &program[function];
			if let Nodes::Function { name, .. } = target.node() {
				return Some(name.clone());
			}
		}
		Nodes::Expression(Expressions::IntrinsicCall { intrinsic, .. }) => {
			if let Some(name) = program[intrinsic].get_name().filter(|name| name.starts_with("atomic_")) {
				return Some(name.to_string());
			}
		}
		_ => {}
	}
	node.node().children().find_map(|child| side_effecting_call(program, child))
}

/// Whether `member` is a single `bool` member.
fn member_is_bool(program: &Program, member: &NodeId) -> bool {
	matches!(program[member].node(), Nodes::Member { r#type, count: None, .. } if program[r#type].get_name() == Some("bool"))
}

/// Whether `member` is an array of `bool`, declared with a count or as a fixed `bool[N]` type.
fn member_is_bool_array(program: &Program, member: &NodeId) -> bool {
	match program[member].node() {
		Nodes::Member { r#type, count, .. } => {
			let r#type = &program[r#type];
			match r#type.node() {
				Nodes::Struct { name, .. } if name == "bool" => count.is_some(),
				Nodes::Struct {
					template: Some(element), ..
				} => program[element].get_name() == Some("bool"),
				_ => false,
			}
		}
		_ => false,
	}
}

/// Finds the built-in declaration under `root` that `matches` accepts.
fn find_builtin(program: &Program, root: &NodeId, matches: impl Fn(&Nodes) -> bool) -> Result<NodeId, LexError> {
	let node = &program[root];
	let Nodes::Scope { children, .. } = node.node() else {
		return Err(missing_builtins());
	};
	children
		.iter()
		.find(|child| matches(program[*child].node()))
		.cloned()
		.ok_or_else(missing_builtins)
}

/// Finds the conversion intrinsic `name` whose one parameter has type `source`.
fn find_conversion(program: &Program, root: &NodeId, name: &str, source: &str) -> Result<NodeId, LexError> {
	find_builtin(program, root, |node| match node {
		Nodes::Intrinsic {
			name: intrinsic,
			elements,
			..
		} if intrinsic == name => elements.iter().any(
			|element| matches!(program[element].node(), Nodes::Parameter { r#type, .. } if program[r#type].get_name() == Some(source)),
		),
		_ => false,
	})
}

fn missing_builtins() -> LexError {
	LexError::invalid(
		"`bool` members can't be packed because the program has no built-in `u8` conversions. The most likely cause is a program that was not created with `Program::new`; create it with `Program::new`.",
	)
}

fn expression(program: &mut Program, expression: Expressions) -> NodeId {
	program.add(Node::expression(expression))
}

fn literal(program: &mut Program, value: u32) -> NodeId {
	expression(program, Expressions::Literal {
		value: value.to_string(),
	})
}

fn operator(program: &mut Program, operator: Operators, left: NodeId, right: NodeId) -> NodeId {
	expression(program, Expressions::Operator { operator, left, right })
}

/// Calls the conversion intrinsic `intrinsic` on `value`. Conversions have no body, so the call's elements are its
/// argument.
fn convert(program: &mut Program, intrinsic: NodeId, value: NodeId) -> NodeId {
	expression(program, Expressions::IntrinsicCall {
		intrinsic,
		arguments: vec![value],
		elements: vec![value],
	})
}
