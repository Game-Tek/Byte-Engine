//! Stores `u8` members in 2-byte windows, because HLSL has no 1-byte member type.
//!
//! [`crate::shader::besl::evaluation::byte_windows`] plans the windows from the scalar layout every backend shares.
//! Here the emitter declares each window as a `uint16_t` in place of the members it holds, and reads and writes each
//! member's byte with shifts, so DX12 reads the same bytes as Metal and Vulkan.

use super::*;
use crate::shader::generator::NodeEmitter;

impl Generator {
	/// Plans the windows of every struct, buffer block, and push-constant block in `order`.
	pub(crate) fn plan_byte_windows(&mut self, order: &[besl::NodeReference]) {
		self.byte_windows.clear();
		// Window members only need their type's name to be declared.
		let u16_type: besl::NodeReference = besl::Node::r#struct("u16", Vec::new()).into();
		for node in order {
			let members = match node.borrow().node() {
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
			let windows = crate::shader::besl::evaluation::byte_windows(&members).unwrap_or_else(|_| {
				members
					.iter()
					.filter(|member| {
						matches!(member.borrow().node(), besl::Nodes::Member { r#type, count: None, .. } if r#type.borrow().get_name() == Some("u8"))
					})
					.enumerate()
					.map(|(index, member)| crate::shader::besl::evaluation::ByteWindow {
						offset: index * 2,
						bytes: vec![(member.clone(), 0)],
					})
					.collect()
			});
			for window in windows {
				let member: besl::NodeReference =
					besl::Node::member(&format!("_besl_bytes{}", window.offset / 2), u16_type.clone()).into();
				for (byte, shift) in window.bytes {
					self.byte_windows.insert(byte, (member.clone(), shift));
				}
			}
		}
	}

	/// Replaces each windowed `u8` member of `members` with the window that holds it, declaring each window once.
	pub(crate) fn windowed_members(&self, members: &[besl::NodeReference]) -> Vec<besl::NodeReference> {
		let mut declared: Vec<&besl::NodeReference> = Vec::new();
		members
			.iter()
			.filter_map(|member| match self.byte_windows.get(member) {
				Some((window, _)) if declared.contains(&window) => None,
				Some((window, _)) => {
					declared.push(window);
					Some(window.clone())
				}
				None => Some(member.clone()),
			})
			.collect()
	}

	/// Returns the window and bit shift of the `u8` member that `right`, the member side of an accessor, reads.
	fn windowed_member(&self, right: &besl::NodeReference) -> Option<(besl::NodeReference, u32)> {
		match right.borrow().node() {
			besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => self.byte_windows.get(source).cloned(),
			_ => None,
		}
	}

	/// Emits the window place `left.window`, through the same accessor lowering as any other member.
	fn emit_window_place(&mut self, string: &mut String, left: &besl::NodeReference, window: &besl::NodeReference) {
		let member: besl::NodeReference = besl::Node::expression(besl::Expressions::Member {
			name: window.borrow().get_name().unwrap_or_default().to_string(),
			source: window.clone(),
		})
		.into();
		NodeEmitter::emit_accessor_expression(self, string, left, &member);
	}

	/// Emits a read or a write of a windowed `u8` member, and returns whether `expression` was one.
	///
	/// A read is `((uint(window) >> shift) & 0xFFu)`. A write keeps the window's other byte:
	/// `window = uint16_t((uint(window) & keep) | ((uint(value) & 0xFFu) << shift))`. It is not atomic, like a
	/// byte store on the other backends.
	pub(crate) fn emit_windowed_byte(&mut self, string: &mut String, expression: &besl::Expressions) -> bool {
		match expression {
			besl::Expressions::Accessor { left, right } => {
				let Some((window, shift)) = self.windowed_member(right) else {
					return false;
				};
				string.push_str("((uint(");
				self.emit_window_place(string, left, &window);
				let _ = write!(string, ")>>{shift}u)&0xFFu)");
				true
			}
			besl::Expressions::Operator {
				operator: besl::Operators::Assignment,
				left: place,
				right: value,
			} => {
				let target = match place.borrow().node() {
					besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => self
						.windowed_member(right)
						.map(|(window, shift)| (left.clone(), window, shift)),
					_ => None,
				};
				let Some((left, window, shift)) = target else {
					return false;
				};
				let keep = 0xFFFFu32 ^ (0xFF << shift);
				self.emit_window_place(string, &left, &window);
				string.push_str("=uint16_t((uint(");
				self.emit_window_place(string, &left, &window);
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
		field: &besl::NodeReference,
		fields: &[besl::NodeReference],
	) -> bool {
		let Some((window, _)) = self.byte_windows.get(field) else {
			return false;
		};
		let first = fields
			.iter()
			.find(|candidate| self.byte_windows.get(*candidate).is_some_and(|(other, _)| other == window));
		if first != Some(field) {
			return true;
		}
		let window_name = window.borrow().get_name().unwrap_or_default().to_string();
		let _ = write!(string, "besl_value.{window_name}=uint16_t(");
		let mut separator = "";
		for candidate in fields {
			if let Some((other, shift)) = self.byte_windows.get(candidate)
				&& other == window
			{
				let name = candidate.borrow().get_name().unwrap_or_default().to_string();
				let _ = write!(string, "{separator}((besl_argument_{name}&0xFFu)<<{shift}u)");
				separator = "|";
			}
		}
		string.push(')');
		true
	}
}
