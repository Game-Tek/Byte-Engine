//! Grammar-aware input generation for the BESL fuzz targets.

use std::fmt::Write;

const MAX_STATEMENTS: usize = 12;
const MAX_EXPRESSION_DEPTH: usize = 3;

/// The `ByteCursor` struct exists to turn arbitrary fuzzer bytes into bounded grammar decisions.
struct ByteCursor<'a> {
	bytes: &'a [u8],
	position: usize,
}

impl<'a> ByteCursor<'a> {
	fn new(bytes: &'a [u8]) -> Self {
		Self { bytes, position: 0 }
	}

	fn next(&mut self) -> u8 {
		let value = self.bytes.get(self.position).copied().unwrap_or(0);
		self.position = self.position.saturating_add(1);
		value
	}

	fn choose(&mut self, choices: usize) -> usize {
		usize::from(self.next()) % choices
	}

	fn chance(&mut self, numerator: u8, denominator: u8) -> bool {
		self.next() % denominator < numerator
	}
}

/// Generates a bounded, type-correct BESL program from arbitrary bytes.
///
/// The generated subset deliberately combines declarations, scalar and vector expressions,
/// function calls, conditionals, and loops so mutations continue beyond syntax parsing into
/// semantic resolution and VM compilation.
pub fn generate_program(data: &[u8]) -> String {
	let mut cursor = ByteCursor::new(data);
	let mut source = String::with_capacity(2_048);

	// These declarations exercise top-level grammar without affecting VM execution semantics.
	source.push_str("Config: struct {\n\tvalue: f32,\n\tindices: u32[4],\n}\n\nLIMIT: const u32 = 4;\n\n");
	source.push_str("combine_u32: fn (left: u32, right: u32) -> u32 {\n\treturn left + right;\n}\n\n");
	source.push_str("combine_f32: fn (left: f32, right: f32) -> f32 {\n\treturn max(left, right);\n}\n\n");
	source.push_str("main: fn () -> void {\n");

	let first_u32 = cursor.next();
	let first_f32 = cursor.next();
	let _ = writeln!(source, "\tlet u0: u32 = {first_u32};");
	let _ = writeln!(source, "\tlet f0: f32 = {}.{};", first_f32 / 10, first_f32 % 10);
	source.push_str("\tlet v0: vec3f = vec3f(f0, 1.0, 2.0);\n");

	let mut u32_locals = 1;
	let mut f32_locals = 1;
	let mut vec3_locals = 1;
	let statement_count = 1 + cursor.choose(MAX_STATEMENTS);
	for statement_index in 0..statement_count {
		emit_statement(
			&mut source,
			&mut cursor,
			statement_index,
			&mut u32_locals,
			&mut f32_locals,
			&mut vec3_locals,
		);
	}

	source.push_str("\treturn;\n}\n");
	source
}

/// Emits one statement while keeping every generated reference in scope and type-correct.
fn emit_statement(
	source: &mut String,
	cursor: &mut ByteCursor<'_>,
	statement_index: usize,
	u32_locals: &mut usize,
	f32_locals: &mut usize,
	vec3_locals: &mut usize,
) {
	match cursor.choose(8) {
		0 => {
			let name = *u32_locals;
			let _ = write!(source, "\tlet u{name}: u32 = ");
			emit_u32_expression(source, cursor, *u32_locals, *f32_locals, MAX_EXPRESSION_DEPTH);
			source.push_str(";\n");
			*u32_locals += 1;
		}
		1 => {
			let name = *f32_locals;
			let _ = write!(source, "\tlet f{name}: f32 = ");
			emit_f32_expression(source, cursor, *f32_locals, *u32_locals, MAX_EXPRESSION_DEPTH);
			source.push_str(";\n");
			*f32_locals += 1;
		}
		2 => {
			let name = *vec3_locals;
			let _ = write!(source, "\tlet v{name}: vec3f = ");
			emit_vec3_expression(source, cursor, *vec3_locals, *f32_locals, MAX_EXPRESSION_DEPTH);
			source.push_str(";\n");
			*vec3_locals += 1;
		}
		3 => {
			let target = cursor.choose(*u32_locals);
			let _ = write!(source, "\tu{target} = ");
			emit_u32_expression(source, cursor, *u32_locals, *f32_locals, MAX_EXPRESSION_DEPTH);
			source.push_str(";\n");
		}
		4 => {
			let target = cursor.choose(*f32_locals);
			let _ = write!(source, "\tf{target} = ");
			emit_f32_expression(source, cursor, *f32_locals, *u32_locals, MAX_EXPRESSION_DEPTH);
			source.push_str(";\n");
		}
		5 => {
			let target = cursor.choose(*vec3_locals);
			let _ = write!(source, "\tv{target} = ");
			emit_vec3_expression(source, cursor, *vec3_locals, *f32_locals, MAX_EXPRESSION_DEPTH);
			source.push_str(";\n");
		}
		6 => emit_conditional(source, cursor, *u32_locals, *f32_locals),
		_ => emit_loop(source, cursor, statement_index, *u32_locals),
	}
}

/// Emits a condition whose body only mutates locals declared in the surrounding function scope.
fn emit_conditional(source: &mut String, cursor: &mut ByteCursor<'_>, u32_locals: usize, f32_locals: usize) {
	let left = cursor.choose(u32_locals);
	let right = cursor.next();
	let operator = ["<", "<=", ">", ">=", "==", "!="][cursor.choose(6)];
	let _ = writeln!(source, "\tif (u{left} {operator} {right}) {{");

	if cursor.chance(1, 2) {
		let target = cursor.choose(u32_locals);
		let value = cursor.next();
		let _ = writeln!(source, "\t\tu{target} = u{target} + {value};");
	} else {
		let target = cursor.choose(f32_locals);
		let value = cursor.next();
		let _ = writeln!(source, "\t\tf{target} = max(f{target}, {}.0);", value % 16);
	}

	source.push_str("\t}\n");
}

/// Emits a finite-shaped loop for compiler coverage; structured targets compile but do not execute it.
fn emit_loop(source: &mut String, cursor: &mut ByteCursor<'_>, statement_index: usize, u32_locals: usize) {
	let limit = 1 + cursor.next() % 8;
	let target = cursor.choose(u32_locals);
	let _ = writeln!(
		source,
		"\tfor (let i{statement_index}: u32 = 0; i{statement_index} < {limit}; i{statement_index} = i{statement_index} + 1) {{"
	);
	let _ = writeln!(source, "\t\tu{target} = u{target} + i{statement_index};");
	if cursor.chance(1, 3) {
		source.push_str("\t\tcontinue;\n");
	}
	source.push_str("\t}\n");
}

/// Writes a bounded `u32` expression using only available locals and supported operations.
fn emit_u32_expression(source: &mut String, cursor: &mut ByteCursor<'_>, u32_locals: usize, f32_locals: usize, depth: usize) {
	if depth == 0 {
		emit_u32_leaf(source, cursor, u32_locals);
		return;
	}

	match cursor.choose(6) {
		0 => emit_u32_leaf(source, cursor, u32_locals),
		1 => {
			let operator = ["+", "-", "*", "/", "%"][cursor.choose(5)];
			source.push('(');
			emit_u32_expression(source, cursor, u32_locals, f32_locals, depth - 1);
			let _ = write!(source, " {operator} ");
			emit_u32_expression(source, cursor, u32_locals, f32_locals, depth - 1);
			source.push(')');
		}
		2 => {
			source.push_str("combine_u32(");
			emit_u32_expression(source, cursor, u32_locals, f32_locals, depth - 1);
			source.push_str(", ");
			emit_u32_expression(source, cursor, u32_locals, f32_locals, depth - 1);
			source.push(')');
		}
		3 => {
			// Keep casts on leaves because the current lexer does not type grouped arguments.
			source.push_str("u32(");
			emit_f32_leaf(source, cursor, f32_locals);
			source.push(')');
		}
		4 => source.push_str("thread_idx()"),
		_ => {
			let operator = ["<", "<=", ">", ">=", "==", "!="][cursor.choose(6)];
			source.push('(');
			emit_u32_leaf(source, cursor, u32_locals);
			let _ = write!(source, " {operator} ");
			emit_u32_leaf(source, cursor, u32_locals);
			source.push(')');
		}
	}
}

fn emit_u32_leaf(source: &mut String, cursor: &mut ByteCursor<'_>, u32_locals: usize) {
	if cursor.chance(1, 2) {
		let _ = write!(source, "u{}", cursor.choose(u32_locals));
	} else {
		let _ = write!(source, "{}", cursor.next());
	}
}

/// Writes a bounded `f32` expression using scalar intrinsics supported by the BESL VM compiler.
fn emit_f32_expression(source: &mut String, cursor: &mut ByteCursor<'_>, f32_locals: usize, u32_locals: usize, depth: usize) {
	if depth == 0 {
		emit_f32_leaf(source, cursor, f32_locals);
		return;
	}

	match cursor.choose(7) {
		0 => emit_f32_leaf(source, cursor, f32_locals),
		1 => {
			let operator = ["+", "-", "*", "/", "%"][cursor.choose(5)];
			source.push('(');
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			let _ = write!(source, " {operator} ");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push(')');
		}
		2 => {
			source.push_str("combine_f32(");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push_str(", ");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push(')');
		}
		3 => {
			source.push_str("max(");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push_str(", ");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push(')');
		}
		4 => {
			let intrinsic = ["abs", "sqrt", "exp", "sin", "cos", "tan", "fract", "radians", "inversesqrt"][cursor.choose(9)];
			let _ = write!(source, "{intrinsic}(");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push(')');
		}
		5 => {
			// Keep casts on leaves because the current lexer does not type grouped arguments.
			source.push_str("f32(");
			emit_u32_leaf(source, cursor, u32_locals);
			source.push(')');
		}
		_ => {
			let intrinsic = ["clamp", "smoothstep", "mix"][cursor.choose(3)];
			let _ = write!(source, "{intrinsic}(");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push_str(", ");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push_str(", ");
			emit_f32_expression(source, cursor, f32_locals, u32_locals, depth - 1);
			source.push(')');
		}
	}
}

fn emit_f32_leaf(source: &mut String, cursor: &mut ByteCursor<'_>, f32_locals: usize) {
	if cursor.chance(1, 2) {
		let _ = write!(source, "f{}", cursor.choose(f32_locals));
	} else {
		let value = cursor.next();
		let _ = write!(source, "{}.{}", value / 10, value % 10);
	}
}

/// Writes a bounded `vec3f` expression to exercise constructors and vector intrinsics.
fn emit_vec3_expression(source: &mut String, cursor: &mut ByteCursor<'_>, vec3_locals: usize, f32_locals: usize, depth: usize) {
	if depth == 0 || cursor.chance(1, 3) {
		let _ = write!(source, "v{}", cursor.choose(vec3_locals));
		return;
	}

	match cursor.choose(4) {
		0 => {
			source.push_str("vec3f(");
			emit_f32_expression(source, cursor, f32_locals, 1, depth - 1);
			source.push_str(", ");
			emit_f32_expression(source, cursor, f32_locals, 1, depth - 1);
			source.push_str(", ");
			emit_f32_expression(source, cursor, f32_locals, 1, depth - 1);
			source.push(')');
		}
		1 => {
			// Keep overloaded vector intrinsics on typed leaves until expression type inference is available.
			let _ = write!(source, "normalize(v{})", cursor.choose(vec3_locals));
		}
		2 => {
			let _ = write!(
				source,
				"cross(v{}, v{})",
				cursor.choose(vec3_locals),
				cursor.choose(vec3_locals)
			);
		}
		_ => {
			let operator = ["+", "-", "*"][cursor.choose(3)];
			source.push('(');
			emit_vec3_expression(source, cursor, vec3_locals, f32_locals, depth - 1);
			let _ = write!(source, " {operator} ");
			emit_vec3_expression(source, cursor, vec3_locals, f32_locals, depth - 1);
			source.push(')');
		}
	}
}

#[cfg(test)]
mod tests {
	use super::generate_program;

	#[test]
	fn generated_programs_compile_through_the_vm() {
		let mut state = 0xA5A5_5A5A_u32;
		for length in 0..=512 {
			let mut data = vec![0; length];
			for byte in &mut data {
				// A deterministic generator covers varied inputs without adding a test-only dependency.
				state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
				*byte = state.to_le_bytes()[3];
			}

			let source = generate_program(&data);
			let program = besl::compile_to_besl(&source, None)
				.unwrap_or_else(|error| panic!("Generated source failed compilation: {error:?}\n\n{source}"));
			besl::vm::ExecutableProgram::compile(program)
				.unwrap_or_else(|error| panic!("Generated source failed VM compilation: {error}\n\n{source}"));
		}
	}

	#[test]
	fn generated_program_size_is_bounded() {
		let source = generate_program(&[u8::MAX; 4_096]);
		assert!(source.len() <= 32 * 1_024, "Generated source exceeded its size bound");
	}
}
