use besl::vm::{Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Value, VmError};
use besl::{BindingTypes, Expressions, Node, Nodes, Operators, Program, compile_to_besl};

#[test]
fn dynamic_buffer_index_is_evaluated_once_during_expression_lowering() {
	let builtins = besl::Builtins::new();
	let mut program = Program::new(&builtins);
	let u32_type = program.get_child(program.root(), "u32").expect("Expected u32 type");
	let atomicu32_name = program.intern("atomicu32");
	let atomic_u32_type = program.add_to_root(Node::r#struct(atomicu32_name, Vec::new()));

	let values_name = program.intern("values");
	let values = program.add(Node::array(values_name, u32_type, 2));
	program.add_to_root(Node::binding(
		values_name,
		BindingTypes::Buffer { members: vec![values] },
		0,
		true,
		false,
	));
	let count_name = program.intern("count");
	let count = program.add(Node::member(count_name, atomic_u32_type));
	let counter_name = program.intern("counter");
	program.add_to_root(Node::binding(
		counter_name,
		BindingTypes::Buffer { members: vec![count] },
		1,
		true,
		true,
	));
	let value_name = program.intern("value");
	let value = program.add(Node::member(value_name, u32_type));
	let result_name = program.intern("result");
	program.add_to_root(Node::binding(
		result_name,
		BindingTypes::Buffer { members: vec![value] },
		2,
		false,
		true,
	));

	// The atomic increment makes an accidental analysis-time lowering visible in
	// both the selected array element and the final counter value.
	let atomic_add_name = program.intern("atomic_add");
	let atomic_add = program.add_to_root(Node::intrinsic(atomic_add_name, Vec::new(), u32_type));
	let value_parameter = program.add(Node::new(Nodes::Parameter {
		name: value_name,
		r#type: atomic_u32_type,
	}));
	let increment_name = program.intern("increment");
	let increment_parameter = program.add(Node::new(Nodes::Parameter {
		name: increment_name,
		r#type: u32_type,
	}));
	program[atomic_add].add_children(vec![value_parameter, increment_parameter]);

	let source = r#"
		main: fn () -> void {
			result.value = values.values[atomic_add(counter.count, 1)] + 0;
		}
	"#;
	let program = compile_to_besl(source, program).expect("Expected lexed program");
	let executable = ExecutableProgram::compile(&program).expect("Expected executable program");

	let values_slot = ResourceSlot::new(0);
	let counter_slot = ResourceSlot::new(1);
	let result_slot = ResourceSlot::new(2);
	let mut values = Buffer::new(executable.buffer_layout(values_slot).expect("Expected values layout").clone());
	let mut counter = Buffer::new(
		executable
			.buffer_layout(counter_slot)
			.expect("Expected counter layout")
			.clone(),
	);
	let mut result = Buffer::new(executable.buffer_layout(result_slot).expect("Expected result layout").clone());
	values
		.write_array_element(0, Value::U32(10))
		.expect("Expected first value write");
	values
		.write_array_element(1, Value::U32(20))
		.expect("Expected second value write");
	counter.write("count", Value::U32(0)).expect("Expected counter write");

	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(values_slot, &mut values);
	descriptors.bind_buffer(counter_slot, &mut counter);
	descriptors.bind_buffer(result_slot, &mut result);
	executable.run_main(&mut descriptors).expect("Expected execution to succeed");

	assert_eq!(result.read("value").expect("Expected result value"), Value::U32(10));
	assert_eq!(counter.read("count").expect("Expected counter value"), Value::U32(1));
}

#[test]
fn non_void_function_fallthrough_reports_the_missing_return() {
	let builtins = besl::Builtins::new();
	let mut program = Program::new(&builtins);
	let u32_type = program.get_child(program.root(), "u32").expect("Expected u32 type");
	let value_name = program.intern("value");
	let value = program.add(Node::member(value_name, u32_type));
	let result_name = program.intern("result");
	program.add_to_root(Node::binding(
		result_name,
		BindingTypes::Buffer { members: vec![value] },
		0,
		false,
		true,
	));

	let source = r#"
		missing_return: fn () -> u32 {
			let value: u32 = 1;
		}

		main: fn () -> void {
			result.value = missing_return();
		}
	"#;
	let program = compile_to_besl(source, program).expect("Expected lexed program");
	let executable = ExecutableProgram::compile(&program).expect("Expected executable program");
	let result_slot = ResourceSlot::new(0);
	let mut result = Buffer::new(executable.buffer_layout(result_slot).expect("Expected result layout").clone());
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(result_slot, &mut result);

	let error = executable
		.run_main(&mut descriptors)
		.expect_err("Expected missing return error");

	assert_eq!(
		error,
		VmError::UnsupportedStatement {
			message: "Function with return type `u32` ended without returning a value".to_string(),
		}
	);
}

#[test]
fn function_arity_is_validated_before_arguments_are_lowered() {
	let builtins = besl::Builtins::new();
	let mut program = Program::new(&builtins);
	let u32_type = program.get_child(program.root(), "u32").expect("Expected u32 type");
	let void_type = program.get_child(program.root(), "void").expect("Expected void type");
	let value_name = program.intern("value");
	let parameter = program.add(Node::new(Nodes::Parameter {
		name: value_name,
		r#type: u32_type,
	}));
	let called_name = program.intern("called");
	let called_function = program.add_to_root(Node::function(called_name, vec![parameter], void_type, Vec::new()));

	// `continue` would produce a different compiler error if lowering started
	// before the call's two-to-one arity mismatch was validated.
	let invalid_argument = program.add(Node::expression(Expressions::Continue));
	let extra_argument = program.add(Node::expression(Expressions::Literal { value: "1".to_string() }));
	let call = program.add(Node::expression(Expressions::FunctionCall {
		function: called_function,
		parameters: vec![invalid_argument, extra_argument],
	}));
	let main_name = program.intern("main");
	program.add_to_root(Node::function(main_name, Vec::new(), void_type, vec![call]));

	let error = match ExecutableProgram::compile(&program) {
		Ok(_) => panic!("Expected argument mismatch"),
		Err(error) => error,
	};

	assert_eq!(error, VmError::CallArgumentMismatch { expected: 1, found: 2 });
}

#[test]
fn intrinsic_arity_is_validated_before_arguments_are_indexed_or_lowered() {
	let builtins = besl::Builtins::new();
	let expression_error = {
		let mut program = Program::new(&builtins);
		let f32_type = program.get_child(program.root(), "f32").expect("Expected f32 type");
		let void_type = program.get_child(program.root(), "void").expect("Expected void type");
		let f32_name = program.intern("f32");
		let intrinsic = program.add_to_root(Node::intrinsic(f32_name, Vec::new(), f32_type));
		let invalid_argument = program.add(Node::expression(Expressions::Continue));
		let extra_argument = program.add(Node::expression(Expressions::Literal { value: "1".to_string() }));
		let call = program.add(Node::expression(Expressions::IntrinsicCall {
			intrinsic,
			arguments: vec![invalid_argument, extra_argument],
			elements: Vec::new(),
		}));
		let value_name = program.intern("value");
		let declaration = program.add(Node::expression(Expressions::VariableDeclaration {
			name: value_name,
			r#type: f32_type,
		}));
		let assignment = program.add(Node::expression(Expressions::Operator {
			operator: Operators::Assignment,
			left: declaration,
			right: call,
		}));
		let main_name = program.intern("main");
		program.add_to_root(Node::function(main_name, Vec::new(), void_type, vec![assignment]));

		// Lowering the first argument would report `continue` as an unsupported value expression.
		compile_error(ExecutableProgram::compile(&program))
	};

	assert_eq!(expression_error, VmError::CallArgumentMismatch { expected: 0, found: 2 });

	let statement_error = {
		let mut program = Program::new(&builtins);
		let void_type = program.get_child(program.root(), "void").expect("Expected void type");
		let write_name = program.intern("write");
		let intrinsic = program.add_to_root(Node::intrinsic(write_name, Vec::new(), void_type));
		let argument = program.add(Node::expression(Expressions::Literal { value: "1".to_string() }));
		let call = program.add(Node::expression(Expressions::IntrinsicCall {
			intrinsic,
			arguments: vec![argument],
			elements: Vec::new(),
		}));
		let main_name = program.intern("main");
		program.add_to_root(Node::function(main_name, Vec::new(), void_type, vec![call]));

		// The declared intrinsic takes no arguments, so the call is rejected before its image argument is read.
		compile_error(ExecutableProgram::compile(&program))
	};

	assert_eq!(statement_error, VmError::CallArgumentMismatch { expected: 0, found: 1 });
}

#[test]
fn comparison_literals_inherit_the_typed_operand() {
	let builtins = besl::Builtins::new();
	let mut program = Program::new(&builtins);
	let bool_type = program.get_child(program.root(), "bool").expect("Expected bool type");
	let value_name = program.intern("value");
	let value = program.add(Node::member(value_name, bool_type));
	let result_name = program.intern("result");
	program.add_to_root(Node::binding(
		result_name,
		BindingTypes::Buffer { members: vec![value] },
		0,
		// Writing a packed `bool` reads its byte first.
		true,
		true,
	));

	let program = compile_to_besl(
		r#"
		main: fn () -> void {
			let signed: i32 = 2;
			result.value = signed < 3;
		}
		"#,
		program,
	)
	.expect("Expected lexed comparison");
	let executable = ExecutableProgram::compile(&program).expect("Expected typed comparison lowering");
	let slot = ResourceSlot::new(0);
	let mut result = Buffer::new(executable.buffer_layout(slot).expect("Expected result layout").clone());
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot, &mut result);
	executable.run_main(&mut descriptors).expect("Expected comparison execution");

	// `value` is bit 0 of the byte that holds the buffer's `bool` members.
	assert_eq!(result.read("_besl_bits0").expect("Expected comparison result"), Value::U8(1));
}

#[test]
fn incompatible_typed_comparison_operands_fail_during_compilation() {
	let builtins = besl::Builtins::new();
	let mut program = Program::new(&builtins);
	let bool_type = program.get_child(program.root(), "bool").expect("Expected bool type");
	let value_name = program.intern("value");
	let value = program.add(Node::member(value_name, bool_type));
	let result_name = program.intern("result");
	program.add_to_root(Node::binding(
		result_name,
		BindingTypes::Buffer { members: vec![value] },
		0,
		// Writing a packed `bool` reads its byte first.
		true,
		true,
	));
	let program = compile_to_besl(
		r#"
		main: fn () -> void {
			let signed: i32 = 1;
			let unsigned: u32 = 1;
			result.value = signed == unsigned;
		}
		"#,
		program,
	)
	.expect("Expected lexed comparison");

	assert_eq!(
		compile_error(ExecutableProgram::compile(&program)),
		VmError::TypeMismatch {
			expected: "i32".to_string(),
			found: "u32".to_string(),
		}
	);
}

fn compile_error(result: Result<ExecutableProgram, VmError>) -> VmError {
	match result {
		Ok(_) => panic!("Expected VM compilation to fail"),
		Err(error) => error,
	}
}
