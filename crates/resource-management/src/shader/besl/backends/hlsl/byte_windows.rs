//! Stores `u8` members in 2-byte windows, because HLSL has no 1-byte member type.
//!
//! [`crate::shader::besl::evaluation::byte_windows`] plans the windows from the scalar layout every backend shares.
//! Here the emitter declares each window as a `uint16_t` in place of the members it holds, and reads and writes each
//! member's byte with shifts, so DX12 reads the same bytes as Metal and Vulkan.

use utils::hash::HashMap;

use super::facade::ByteWindowSlot;
use super::*;
use crate::shader::generator::NodeEmitter;

impl<'p> Generator<'p> {
	/// Declares the windows of every struct, buffer block, and push-constant block that `main` reaches, and returns the
	/// window of each `u8` member. It adds each window's `u16` member and the member expression that names it to
	/// `program`, so emission only reads the program.
	pub(crate) fn plan_byte_windows(program: &mut besl::Program) -> HashMap<besl::NodeId, ByteWindowSlot> {
		let mut slots = HashMap::default();
		let Some(main) = program.get_main() else {
			return slots;
		};
		let order = ordered_shader_nodes(program, &main, "HLSL");
		// Window members only need their type's name to be declared, which the built-in `u16` gives.
		let Some(u16_type) = program.get_child(program.root(), "u16") else {
			return slots;
		};
		for node in &order {
			let members = match program[node].node() {
				besl::Nodes::Struct {
					template: None, fields, ..
				} => fields.clone(),
				besl::Nodes::PushConstant { members }
				| besl::Nodes::Binding {
					r#type: besl::BindingTypes::Buffer { members },
					..
				} => members.clone(),
				_ => continue,
			};
			// Reflection rejects a stored layout whose windows don't fit, so a plan fails only for a struct with no
			// storage layout, which only locals use. There each `u8` gets a window of its own.
			let windows = crate::shader::besl::evaluation::byte_windows(program, &members).unwrap_or_else(|_| {
				members
					.iter()
					.filter(|member| {
						matches!(program[*member].node(), besl::Nodes::Member { r#type, count: None, .. } if program.name(*r#type) == Some("u8"))
					})
					.enumerate()
					.map(|(index, member)| crate::shader::besl::evaluation::ByteWindow {
						offset: index * 2,
						bytes: vec![(*member, 0)],
					})
					.collect()
			});
			for window in windows {
				let name = program.intern(&format!("_besl_bytes{}", window.offset / 2));
				let declaration = program.add(besl::Node::member(name, u16_type));
				let place = program.add(besl::Node::expression(besl::Expressions::Member {
					name,
					source: declaration,
				}));
				for (byte, shift) in window.bytes {
					slots.insert(
						byte,
						ByteWindowSlot {
							window: declaration,
							place,
							shift,
						},
					);
				}
			}
		}
		slots
	}

	/// Replaces each windowed `u8` member of `members` with the window that holds it, declaring each window once.
	pub(crate) fn windowed_members(&self, members: &[besl::NodeId]) -> Vec<besl::NodeId> {
		let mut declared: Vec<besl::NodeId> = Vec::new();
		members
			.iter()
			.filter_map(|member| match self.byte_windows.get(member) {
				Some(slot) if declared.contains(&slot.window) => None,
				Some(slot) => {
					declared.push(slot.window);
					Some(slot.window)
				}
				None => Some(*member),
			})
			.collect()
	}

	/// Returns the window of the `u8` member that `right`, the member side of an accessor, reads.
	fn windowed_member(&self, right: &besl::NodeId) -> Option<ByteWindowSlot> {
		match self.program[right].node() {
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => self.byte_windows.get(source).copied(),
			_ => None,
		}
	}

	/// Emits the window place `left.window`, through the same accessor lowering as any other member.
	fn emit_window_place(&mut self, string: &mut String, left: &besl::NodeId, slot: &ByteWindowSlot) {
		NodeEmitter::emit_accessor_expression(self, string, left, &slot.place);
	}

	/// Emits a read or a write of a windowed `u8` member, and returns whether `expression` was one.
	///
	/// A read is `((uint(window) >> shift) & 0xFFu)`. A write keeps the window's other byte:
	/// `window = uint16_t((uint(window) & keep) | ((uint(value) & 0xFFu) << shift))`. It is not atomic, like a
	/// byte store on the other backends.
	pub(crate) fn emit_windowed_byte(&mut self, string: &mut String, expression: &besl::Expressions) -> bool {
		let program = self.program;
		match expression {
			besl::Expressions::Accessor { left, right } => {
				let Some(slot) = self.windowed_member(right) else {
					return false;
				};
				string.push_str("((uint(");
				self.emit_window_place(string, left, &slot);
				let _ = write!(string, ")>>{}u)&0xFFu)", slot.shift);
				true
			}
			besl::Expressions::Operator {
				operator: besl::Operators::Assignment,
				left: place,
				right: value,
			} => {
				let target = match program[place].node() {
					besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => {
						self.windowed_member(right).map(|slot| (*left, slot))
					}
					_ => None,
				};
				let Some((left, slot)) = target else {
					return false;
				};
				let shift = slot.shift;
				let keep = 0xFFFFu32 ^ (0xFF << shift);
				self.emit_window_place(string, &left, &slot);
				string.push_str("=uint16_t((uint(");
				self.emit_window_place(string, &left, &slot);
				let _ = write!(string, ")&{keep}u)|((uint(");
				self.emit_node_string(string, value);
				let _ = write!(string, ")&0xFFu)<<{shift}u))");
				true
			}
			_ => false,
		}
	}

	/// Emits the factory statements that assemble each window of a constructed struct from its `u8` arguments, on the
	/// window's first member, and returns whether `field` is windowed.
	pub(crate) fn emit_windowed_factory_field(
		&self,
		string: &mut String,
		field: &besl::NodeId,
		fields: &[besl::NodeId],
	) -> bool {
		let program = self.program;
		let Some(window) = self.byte_windows.get(field).map(|slot| slot.window) else {
			return false;
		};
		let first = fields
			.iter()
			.find(|candidate| self.byte_windows.get(*candidate).is_some_and(|other| other.window == window));
		if first != Some(field) {
			return true;
		}
		let window_name = program.name(window).unwrap_or_default().to_string();
		let _ = write!(string, "besl_value.{window_name}=uint16_t(");
		let mut separator = "";
		for candidate in fields {
			if let Some(&ByteWindowSlot {
				window: other, shift, ..
			}) = self.byte_windows.get(candidate)
				&& other == window
			{
				let name = program.name(*candidate).unwrap_or_default().to_string();
				let _ = write!(string, "{separator}((besl_argument_{name}&0xFFu)<<{shift}u)");
				separator = "|";
			}
		}
		string.push(')');
		true
	}
}
