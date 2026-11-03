//! Writes a parsed BESL syntax tree back as BESL source.
//!
//! Use [`write`] to see the program a generator built or a transform produced, for example through a bake intercept.
//! A tree parsed from source writes as source that parses back to the same tree. Engine code can also build nodes that
//! have no source form; those write in the nearest form a reader understands:
//!
//! - A binding writes as a `descriptor`. A buffer binding names its members inline, which source can't.
//! - A specialization constant and an intrinsic declaration write as `//` comments.

use crate::lexer::BufferMemoryClass;
use crate::parser::{
	BindingResource, ElseBranch, Expressions, MatchArm, MatchPattern, Node, Nodes, RecordField, SignaturePart,
};
use crate::sampler::SamplerState;

/// Writes `node`, usually the root that [`crate::parse`] or a program generator returns, as BESL source.
///
/// A root writes its declarations separated by blank lines. Any other node writes as the declaration, statement, or
/// expression it is.
pub fn write(node: &Node<'_>) -> String {
	let mut writer = Writer {
		out: String::with_capacity(4096),
		depth: 0,
	};
	writer.declaration(node);
	writer.out
}

/// The `Writer` struct accumulates source text and tracks where the next line starts.
struct Writer {
	out: String,
	/// The number of tabs that start the next line.
	depth: usize,
}

/// The precedence of operands that never need parentheses: names, literals, calls, groups, and `if`/`match` values.
const OPERAND: u8 = 0;
/// The precedence of member access and indexing, the tightest binding.
const ACCESS: u8 = 1;

impl Writer {
	/// Starts a new line at the current depth.
	fn line(&mut self) {
		for _ in 0..self.depth {
			self.out.push('\t');
		}
	}

	/// Writes each of `items` with `write`, with `separator` between them.
	fn separated<T>(&mut self, items: &[T], separator: &str, mut write: impl FnMut(&mut Self, &T)) {
		for (index, item) in items.iter().enumerate() {
			if index > 0 {
				self.out.push_str(separator);
			}
			write(self, item);
		}
	}

	/// Writes the module `node`, a scope, inline under its name after `prefix`. A module whose file is attached writes
	/// inline too, so the result is one file.
	fn module(&mut self, node: &Node<'_>, prefix: &str) {
		let Nodes::Scope { name, children } = node.node() else {
			return self.declaration(node);
		};
		self.line();
		self.push_display(format_args!("{prefix}{name}: mod {{\n"));
		self.depth += 1;
		self.separated(children, "\n", Self::declaration);
		self.depth -= 1;
		self.line();
		self.out.push_str("}\n");
	}

	/// Writes a module-level declaration, or a whole scope of them.
	fn declaration(&mut self, node: &Node<'_>) {
		match node.node() {
			Nodes::Scope { name, children } => {
				// `#[name]` attributes parse to an empty scope that keeps no name, so there is nothing to write.
				if *name == "MACRO" && children.is_empty() {
					return;
				}
				if *name == "root" {
					self.separated(children, "\n", Self::declaration);
					return;
				}
				// The modules from the asset root write as `::path: mod { ... }`, which parses back to the same place.
				if *name == crate::modules::ROOT_MODULES {
					self.separated(children, "\n", |writer, module| writer.module(module, "::"));
					return;
				}
				self.module(node, "");
			}
			Nodes::FileModule { name } => {
				self.line();
				self.push_display(format_args!("{name}: mod;\n"));
			}
			Nodes::Use { path, glob } => {
				self.line();
				self.push_display(format_args!("use {path}{};\n", if *glob { "::*" } else { "" }));
			}
			Nodes::Layer { name, members } => {
				self.line();
				self.push_display(format_args!("{name}: layer {{\n"));
				self.depth += 1;
				for (member, node) in members {
					match node.node() {
						// A push constant block takes the member's name, where a top-level block is always `push_constant`.
						Nodes::PushConstant { members } => self.members(format_args!("{member}: push_constant"), members),
						_ => self.declaration(node),
					}
				}
				self.depth -= 1;
				self.line();
				self.out.push_str("}\n");
			}
			Nodes::Signature { name, parts } => {
				self.line();
				self.push_display(format_args!("{name}: "));
				self.separated(parts, " + ", |writer, part| match part {
					SignaturePart::Named(path) => writer.out.push_str(path),
					SignaturePart::Inline { params, return_type } => {
						writer.out.push_str("fn (");
						writer.separated(params, ", ", |writer, parameter| writer.member(parameter));
						writer.push_display(format_args!(") -> {return_type}"));
					}
				});
				self.out.push_str(";\n");
			}
			Nodes::Implementation {
				name,
				signature,
				statements,
			} => {
				self.line();
				self.push_display(format_args!("{name}: {signature} "));
				self.block(statements);
				self.out.push('\n');
			}
			Nodes::Struct { name, fields } => self.members(format_args!("{name}: struct"), fields),
			Nodes::PushConstant { members } => self.members(format_args!("push_constant: push_constant"), members),
			Nodes::Member { .. } | Nodes::Parameter { .. } => {
				self.line();
				self.member(node);
				self.out.push_str(";\n");
			}
			Nodes::Function {
				name,
				params,
				return_type,
				statements,
				..
			} => {
				self.line();
				self.push_display(format_args!("{name}: fn ("));
				self.separated(params, ", ", |writer, parameter| writer.member(parameter));
				self.push_display(format_args!(") -> {return_type} "));
				self.block(statements);
				self.out.push('\n');
			}
			Nodes::Binding {
				name,
				r#type,
				slot,
				read,
				write,
				memory_class,
				count,
			} => {
				self.descriptor_head(name);
				match r#type {
					BindingResource::Buffer { members } => {
						self.out.push_str("{ ");
						self.separated(members, ", ", |writer, member| writer.member(member));
						self.out.push_str(" }");
					}
					BindingResource::Image { format } => self.push_display(format_args!("StorageImage<{format}>")),
					BindingResource::CombinedImageSampler { format } => self.out.push_str(match *format {
						"" => "Texture2D",
						"ArrayTexture2D" => "Texture2DArray",
						format => format,
					}),
				}
				self.descriptor_tail(*slot, *read, *write, *memory_class, count.map(|count| count.get()), None);
			}
			Nodes::Descriptor {
				name,
				resource_type,
				runtime_array,
				format,
				slot,
				read,
				write,
				memory_class,
				count,
				sampler,
			} => {
				self.descriptor_head(name);
				self.out.push_str(resource_type);
				if *runtime_array {
					self.out.push_str("[]");
				}
				if let Some(format) = format {
					self.push_display(format_args!("<{format}>"));
				}
				self.descriptor_tail(*slot, *read, *write, *memory_class, count.map(|count| count.get()), *sampler);
			}
			Nodes::Sampler { name, state } => {
				let SamplerState {
					filter,
					mip_filter,
					reduction,
					address,
				} = state;
				// Every property is written, so the reader doesn't need to know the defaults.
				self.line();
				self.push_display(format_args!(
					"{name}: sampler<{{ filter: {}, mip_filter: {}, reduction: {}, address: {} }}>;\n",
					filter.token(),
					mip_filter.token(),
					reduction.token(),
					address.token()
				));
			}
			Nodes::Specialization { name, r#type, id } => {
				self.line();
				self.push_display(format_args!("// specialization {id}: {name}: {type}\n"));
			}
			Nodes::Intrinsic { name, r#return, .. } => {
				self.line();
				self.push_display(format_args!("// intrinsic {name} -> {}\n", r#return));
			}
			Nodes::Input { name, format, location } => {
				self.line();
				self.push_display(format_args!("{name}: input<{format}, {location}>;\n"));
			}
			Nodes::Output {
				name,
				format,
				location,
				count,
				per_vertex,
			} => {
				let declaration = if *per_vertex { "vertex_output" } else { "output" };
				self.line();
				self.push_display(format_args!("{name}: {declaration}<{format}, {location}"));
				self.count(count.map(|count| count.get()));
			}
			Nodes::TaskPayload { name, format, count } => {
				self.line();
				self.push_display(format_args!("{name}: task_payload<{format}, {count}>;\n"));
			}
			Nodes::Workgroup { name, format, count } => {
				self.line();
				self.push_display(format_args!("{name}: workgroup<{format}"));
				self.count(count.map(|count| count.get()));
			}
			Nodes::Const { name, r#type, value } => {
				self.line();
				self.push_display(format_args!("{name}: const {type} = "));
				self.expression(value, OPERAND);
				self.out.push_str(";\n");
			}
			Nodes::Expression(_) | Nodes::Conditional { .. } | Nodes::Match { .. } | Nodes::ForLoop { .. } => {
				self.statement(node);
			}
		}
	}

	/// Writes `header`, then `members` one per line in braces, as structs and the push constant block declare them.
	fn members(&mut self, header: std::fmt::Arguments<'_>, members: &[Node<'_>]) {
		self.line();
		self.push_display(header);
		self.out.push_str(" {\n");
		self.depth += 1;
		for member in members {
			self.line();
			self.member(member);
			self.out.push_str(",\n");
		}
		self.depth -= 1;
		self.line();
		self.out.push_str("}\n");
	}

	/// Writes a struct field, buffer member, or function parameter as `name: type`.
	fn member(&mut self, node: &Node<'_>) {
		match node.node() {
			Nodes::Member { name, r#type } => self.push_display(format_args!("{name}: {type}")),
			Nodes::Parameter { name, r#type } => self.push_display(format_args!("{name}: {type}")),
			_ => {}
		}
	}

	/// Writes the optional element count that ends a stage-interface declaration, and the end of the declaration.
	fn count(&mut self, count: Option<usize>) {
		if let Some(count) = count {
			self.push_display(format_args!(", {count}"));
		}
		self.out.push_str(">;\n");
	}

	/// Writes a descriptor up to its type.
	fn descriptor_head(&mut self, name: &str) {
		self.line();
		self.push_display(format_args!("{name}: descriptor<{{ type: "));
	}

	/// Writes the properties after a descriptor's type, and the end of its declaration.
	fn descriptor_tail(
		&mut self,
		slot: u32,
		read: bool,
		write: bool,
		memory_class: Option<BufferMemoryClass>,
		count: Option<u32>,
		sampler: Option<&str>,
	) {
		let access = match (read, write) {
			(true, true) => "read_write",
			(false, true) => "write",
			_ => "read",
		};
		self.push_display(format_args!(", binding: {slot}, access: {access}"));
		if let Some(memory_class) = memory_class {
			self.push_display(format_args!(", memory: {}", memory_class.token()));
		}
		if let Some(count) = count {
			self.push_display(format_args!(", count: {count}"));
		}
		if let Some(sampler) = sampler {
			self.push_display(format_args!(", sampler: {sampler}"));
		}
		self.out.push_str(" }>;\n");
	}

	/// Writes `{`, `statements` one per line one level deeper, and `}`, leaving the cursor after the `}`.
	fn block(&mut self, statements: &[Node<'_>]) {
		self.out.push_str("{\n");
		self.depth += 1;
		for statement in statements {
			self.statement(statement);
		}
		self.depth -= 1;
		self.line();
		self.out.push('}');
	}

	/// Writes one statement of a block on its own lines.
	fn statement(&mut self, node: &Node<'_>) {
		match node.node() {
			// Blocks end the statement themselves; a block's value has no `;`.
			Nodes::Conditional { .. }
			| Nodes::Match { .. }
			| Nodes::ForLoop { .. }
			| Nodes::Expression(Expressions::Yield { .. }) => {
				self.line();
				self.expression(node, OPERAND);
				self.out.push('\n');
			}
			Nodes::Expression(_) => {
				self.line();
				self.expression(node, OPERAND);
				self.out.push_str(";\n");
			}
			_ => self.declaration(node),
		}
	}

	/// Writes `node` as an expression that sits where an operand of precedence `limit` may go, adding parentheses when
	/// it binds more loosely. `OPERAND` accepts anything.
	fn expression(&mut self, node: &Node<'_>, limit: u8) {
		let parenthesize = limit != OPERAND && precedence(node) > limit;
		if parenthesize {
			self.out.push('(');
		}
		self.unparenthesized(node);
		if parenthesize {
			self.out.push(')');
		}
	}

	/// Writes `node` as an expression without parentheses around it.
	fn unparenthesized(&mut self, node: &Node<'_>) {
		match node.node() {
			Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => {
				self.out.push_str("if (");
				self.expression(condition, OPERAND);
				self.out.push_str(") ");
				self.block(statements);
				match else_branch {
					Some(ElseBranch::Block(statements)) => {
						self.out.push_str(" else ");
						self.block(statements);
					}
					Some(ElseBranch::If(conditional)) => {
						self.out.push_str(" else ");
						self.unparenthesized(conditional);
					}
					None => {}
				}
			}
			Nodes::Match { scrutinee, arms } => {
				self.out.push_str("match ");
				self.expression(scrutinee, OPERAND);
				self.out.push_str(" {\n");
				self.depth += 1;
				for MatchArm { patterns, statements } in arms {
					self.line();
					self.separated(patterns, " | ", |writer, pattern| match pattern {
						MatchPattern::Literal { value, negative } => {
							writer.push_display(format_args!("{}{value}", if *negative { "-" } else { "" }));
						}
						MatchPattern::Wildcard => writer.out.push('_'),
					});
					// A block body needs no comma after it, and means the same as the expression arm it may stand for.
					self.out.push_str(" => ");
					self.block(statements);
					self.out.push('\n');
				}
				self.depth -= 1;
				self.line();
				self.out.push('}');
			}
			Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				self.out.push_str("for (");
				self.expression(initializer, OPERAND);
				self.out.push_str("; ");
				self.expression(condition, OPERAND);
				self.out.push_str("; ");
				self.expression(update, OPERAND);
				self.out.push_str(") ");
				self.block(statements);
			}
			Nodes::Expression(expression) => self.expression_form(expression),
			_ => unreachable!(
				"A declaration was written as an expression. The most likely cause is a program generator that placed a declaration inside an expression."
			),
		}
	}

	/// Writes one [`Expressions`] form without parentheses around it.
	fn expression_form(&mut self, expression: &Expressions<'_>) {
		match expression {
			// A parsed group, `(a + b)`, or the index of an accessor; see `Expressions::Accessor` below.
			Expressions::Expression(elements) => {
				self.out.push('(');
				self.separated(elements, " ", |writer, element| writer.expression(element, OPERAND));
				self.out.push(')');
			}
			Expressions::Accessor { left, right } => {
				self.expression(left, ACCESS);
				match right.node() {
					// `a[i]` parses to an accessor whose right side is the group holding `i`.
					Nodes::Expression(Expressions::Expression(elements)) => {
						self.out.push('[');
						self.separated(elements, " ", |writer, element| writer.expression(element, OPERAND));
						self.out.push(']');
					}
					Nodes::Expression(Expressions::Literal { value }) => self.push_display(format_args!("[{value}]")),
					_ => {
						self.out.push('.');
						self.unparenthesized(right);
					}
				}
			}
			Expressions::Member { name } => self.out.push_str(name),
			Expressions::Literal { value } => self.out.push_str(value),
			Expressions::RecordLiteral { fields } => {
				self.out.push_str(if fields.is_empty() { "{" } else { "{ " });
				self.separated(fields, ", ", |writer, RecordField { name, value }| {
					writer.out.push_str(name);
					// `{ color }` is shorthand for `{ color: color }`.
					let shorthand =
						matches!(value.node(), Nodes::Expression(Expressions::Member { name: value }) if value == name);
					if !shorthand {
						writer.out.push_str(": ");
						writer.expression(value, OPERAND);
					}
				});
				self.out.push_str(if fields.is_empty() { "}" } else { " }" });
			}
			Expressions::Call { name, parameters } => {
				self.push_display(format_args!("{name}("));
				self.separated(parameters, ", ", |writer, parameter| writer.expression(parameter, OPERAND));
				self.out.push(')');
			}
			Expressions::Operator { operator, left, right } => {
				// Operators group left to right, so a right operand that binds as loosely needs parentheses.
				let precedence = operator.precedence();
				self.expression(left, precedence);
				self.push_display(format_args!(" {} ", operator.token()));
				self.expression(right, precedence - 1);
			}
			Expressions::Unary { operator, operand } => {
				self.out.push_str(operator.token());
				// `- -x` and `- -1` keep their space, so the two signs never read as one token.
				let signed = match operand.node() {
					Nodes::Expression(Expressions::Unary { .. }) => true,
					Nodes::Expression(Expressions::Literal { value }) => value.starts_with('-'),
					_ => false,
				};
				if signed {
					self.out.push(' ');
				}
				self.expression(operand, crate::lexer::UNARY_PRECEDENCE);
			}
			Expressions::Yield { value } => self.expression(value, OPERAND),
			Expressions::VariableDeclaration { name, r#type } => self.push_display(format_args!("let {name}: {type}")),
			Expressions::Macro { name, body } => {
				self.push_display(format_args!("#[{name}] "));
				self.expression(body, OPERAND);
			}
			Expressions::Return { value } => {
				self.out.push_str("return");
				if let Some(value) = value {
					self.out.push(' ');
					self.expression(value, OPERAND);
				}
			}
			Expressions::Continue => self.out.push_str("continue"),
			Expressions::Break => self.out.push_str("break"),
			Expressions::Discard => self.out.push_str("discard"),
		}
	}

	/// Appends formatted text without an intermediate allocation.
	fn push_display(&mut self, arguments: std::fmt::Arguments<'_>) {
		use std::fmt::Write as _;
		self.out.write_fmt(arguments).expect("Writing to a String never fails.");
	}
}

/// Returns how loosely `node` binds as an operand, on the scale of [`crate::Operators::precedence`].
fn precedence(node: &Node<'_>) -> u8 {
	match node.node() {
		Nodes::Expression(Expressions::Operator { operator, .. }) => operator.precedence(),
		Nodes::Expression(Expressions::Unary { .. }) => crate::lexer::UNARY_PRECEDENCE,
		Nodes::Expression(Expressions::Accessor { .. }) => ACCESS,
		_ => OPERAND,
	}
}

#[cfg(test)]
mod tests {
	use super::write;
	use crate::parser::Node;

	/// Asserts that `source` writes as source that parses back to the same tree, and returns what it wrote.
	fn assert_round_trips(source: &str) -> String {
		let tree = crate::parse(source).expect("The test source should parse.");
		let written = write(&tree);
		let reparsed =
			crate::parse(&written).unwrap_or_else(|error| panic!("The written source should parse ({error:?}):\n{written}"));
		assert_eq!(
			format!("{reparsed:?}"),
			format!("{tree:?}"),
			"The written source should parse to the tree it was written from:\n{written}"
		);
		written
	}

	#[test]
	fn declarations_and_resources_round_trip() {
		assert_round_trips(
			r#"
			Light: struct { position: vec3f, intensity: f32, samples: u32[4], }
			WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);
			linear_clamp: sampler<{ filter: linear, address: clamp }>;
			lights: descriptor<{ type: Light[], binding: 0, access: read, memory: device }>;
			albedo: descriptor<{ type: Texture2D, binding: 1, access: read, sampler: linear_clamp }>;
			gbuffers: descriptor<{ type: Texture2D, binding: 2, access: read, count: 4 }>;
			output_image: descriptor<{ type: StorageImage<rgba16f>, binding: 3, access: read_write }>;
			push_constant: push_constant { index: u32, }
			instance_index: input<u32, 0>;
			color: output<vec4f, 0>;
			uvs: vertex_output<vec2f, 3, 64>;
			visible: task_payload<u32, 32>;
			counter: workgroup<atomicu32>;
			scratch: workgroup<f32, 64>;
			main: fn () -> void { return; }
			"#,
		);
	}

	#[test]
	fn statements_and_operator_precedence_round_trip() {
		assert_round_trips(
			r#"
			shade: fn (intensity: f32, bias: f32, kind: u32) -> f32 {
				let scaled: f32 = (intensity + bias) * -2.0 - intensity / bias % 3.0;
				let grouped: f32 = intensity - (bias - scaled);
				let lobe: u32 = kind << 2 | kind & 3 ^ ~kind;
				match lobe {
					0 => lobe = 1,
					1 | 2 => { lobe = 2; }
					-1 => lobe = 4,
					_ => lobe = 3,
				}
				let weight: f32 = match lobe {
					0 => 0.1,
					_ => {
						let squared: f32 = scaled * scaled;
						squared * 0.5
					}
				};
				let shade: f32 = if (scaled > 0.0 && !(bias < 1.0) || kind != 2) { 1.0 } else if (bias == 0.0) { 0.5 } else { 0.25 };
				let total: f32 = 0.0;
				let values: f32[3] = f32[3](1.0, 2.0, 3.0);
				for (let i: u32 = 0; i < 3; i = i + 1) {
					if (i >= 2) {
						break;
					} else {
						continue;
					}
					values[i] = values[i] * - -weight;
				}
				return total - shade * values[kind].x + if (kind <= 1) { 1.0 } else { 2.0 };
			}
			"#,
		);
	}

	#[test]
	fn raster_entry_points_and_records_round_trip() {
		assert_round_trips(
			r#"
			main: fn (pipeline_input: interface { uv: vec2f, id: primitive<u32> }) -> output { color: vec4f, depth: f32 } {
				let color: vec4f = vec4f(pipeline_input.uv.x, pipeline_input.uv.y, 0.0, 1.0);
				if (color.x < 0.0) {
					discard;
				}
				return { color, depth: 1.0 };
			}
			"#,
		);
	}

	#[test]
	fn written_source_links() {
		let written = assert_round_trips(
			r#"
			WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);
			sum: fn (count: u32) -> f32 {
				let total: f32 = 0.0;
				for (let i: u32 = 0; i < count; i = i + 1) {
					total = total + WEIGHTS[i] * (1.0 - f32(i));
				}
				return total;
			}
			main: fn () -> void {
				let total: f32 = sum(3);
				return;
			}
			"#,
		);

		crate::lex(crate::parse(&written).unwrap())
			.unwrap_or_else(|error| panic!("The written source should link ({error:?}):\n{written}"));
	}

	#[test]
	fn engine_built_declarations_write_in_their_nearest_source_form() {
		let frame = Node::binding("frame", Node::buffer(vec![Node::member("exposure", "f32")]), 0, true, false);
		let root = Node::root_with_children(vec![
			frame,
			Node::specialization("transparent", "bool", 7),
			Node::binding("albedo", Node::combined_image_sampler(), 1, true, false),
		]);

		assert_eq!(
			write(&root),
			"frame: descriptor<{ type: { exposure: f32 }, binding: 0, access: read }>;\n\
			\n\
			// specialization 7: transparent: bool\n\
			\n\
			albedo: descriptor<{ type: Texture2D, binding: 1, access: read }>;\n"
		);
	}
}
