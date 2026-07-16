use besl::vm::{Buffer, DescriptorBindings, DescriptorSlot, ExecutableProgram, Value, VmError};
use besl::{compile_to_besl, BindingTypes, Expressions, Node, Nodes, Operators};

#[test]
fn dynamic_buffer_index_is_evaluated_once_during_expression_lowering() {
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	let atomic_u32_type = root.add_child(Node::r#struct("atomicu32", Vec::new()).into());

	root.add_children(vec![
		Node::binding(
			"values",
			BindingTypes::Buffer {
				members: vec![Node::array("values", u32_type.clone(), 2)],
			},
			0,
			0,
			true,
			false,
		)
		.into(),
		Node::binding(
			"counter",
			BindingTypes::Buffer {
				members: vec![Node::member("count", atomic_u32_type.clone()).into()],
			},
			0,
			1,
			true,
			true,
		)
		.into(),
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type.clone()).into()],
			},
			0,
			2,
			false,
			true,
		)
		.into(),
	]);

	// The atomic increment makes an accidental analysis-time lowering visible in
	// both the selected array element and the final counter value.
	let atomic_add = root.add_child(Node::intrinsic("atomic_add", Vec::new(), u32_type.clone()).into());
	atomic_add.borrow_mut().add_children(vec![
		Node::new(Nodes::Parameter {
			name: "value".to_string(),
			r#type: atomic_u32_type,
		})
		.into(),
		Node::new(Nodes::Parameter {
			name: "increment".to_string(),
			r#type: u32_type,
		})
		.into(),
	]);

	let source = r#"
		main: fn () -> void {
			result.value = values.values[atomic_add(counter.count, 1)] + 0;
		}
	"#;
	let program = compile_to_besl(source, Some(root)).expect("Expected lexed program");
	let executable = ExecutableProgram::compile(program).expect("Expected executable program");

	let values_slot = DescriptorSlot::new(0, 0);
	let counter_slot = DescriptorSlot::new(0, 1);
	let result_slot = DescriptorSlot::new(0, 2);
	let mut values = Buffer::new(executable.buffer_layout(values_slot).expect("Expected values layout").clone());
	let mut counter = Buffer::new(
		executable
			.buffer_layout(counter_slot)
			.expect("Expected counter layout")
			.clone(),
	);
	let mut result = Buffer::new(executable.buffer_layout(result_slot).expect("Expected result layout").clone());
	values
		.write_indexed("values", 0, Value::U32(10))
		.expect("Expected first value write");
	values
		.write_indexed("values", 1, Value::U32(20))
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
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	root.add_child(
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
			0,
			0,
			false,
			true,
		)
		.into(),
	);

	let source = r#"
		missing_return: fn () -> u32 {
			let value: u32 = 1;
		}

		main: fn () -> void {
			result.value = missing_return();
		}
	"#;
	let program = compile_to_besl(source, Some(root)).expect("Expected lexed program");
	let executable = ExecutableProgram::compile(program).expect("Expected executable program");
	let result_slot = DescriptorSlot::new(0, 0);
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
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	let void_type = root.get_child("void").expect("Expected void type");
	let parameter = Node::new(Nodes::Parameter {
		name: "value".to_string(),
		r#type: u32_type,
	})
	.into();
	let called_function = root.add_child(Node::function("called", vec![parameter], void_type.clone(), Vec::new()).into());

	// `continue` would produce a different compiler error if lowering started
	// before the call's two-to-one arity mismatch was validated.
	let invalid_argument = Node::expression(Expressions::Continue).into();
	let extra_argument = Node::expression(Expressions::Literal { value: "1".to_string() }).into();
	let call = Node::expression(Expressions::FunctionCall {
		function: called_function,
		parameters: vec![invalid_argument, extra_argument],
	})
	.into();
	root.add_child(Node::function("main", Vec::new(), void_type, vec![call]).into());

	let error = match ExecutableProgram::compile(root.into()) {
		Ok(_) => panic!("Expected argument mismatch"),
		Err(error) => error,
	};
	assert_eq!(error, VmError::CallArgumentMismatch { expected: 1, found: 2 });
}

#[test]
fn intrinsic_arity_is_validated_before_arguments_are_indexed_or_lowered() {
	let expression_error = {
		let mut root = Node::root();
		let f32_type = root.get_child("f32").expect("Expected f32 type");
		let void_type = root.get_child("void").expect("Expected void type");
		let intrinsic = root.add_child(Node::intrinsic("f32", Vec::new(), f32_type.clone()).into());
		let invalid_argument = Node::expression(Expressions::Continue).into();
		let extra_argument = Node::expression(Expressions::Literal { value: "1".to_string() }).into();
		let call = Node::expression(Expressions::IntrinsicCall {
			intrinsic,
			arguments: vec![invalid_argument, extra_argument],
			elements: Vec::new(),
		})
		.into();
		let declaration = Node::expression(Expressions::VariableDeclaration {
			name: "value".to_string(),
			r#type: f32_type,
		})
		.into();
		let assignment = Node::expression(Expressions::Operator {
			operator: Operators::Assignment,
			left: declaration,
			right: call,
		})
		.into();
		root.add_child(Node::function("main", Vec::new(), void_type, vec![assignment]).into());

		// Lowering the first argument would report `continue` as an unsupported value expression.
		compile_error(ExecutableProgram::compile(root.into()))
	};
	assert_eq!(expression_error, VmError::CallArgumentMismatch { expected: 1, found: 2 });

	let statement_error = {
		let mut root = Node::root();
		let void_type = root.get_child("void").expect("Expected void type");
		let intrinsic = root.add_child(Node::intrinsic("write", Vec::new(), void_type.clone()).into());
		let call = Node::expression(Expressions::IntrinsicCall {
			intrinsic,
			arguments: Vec::new(),
			elements: Vec::new(),
		})
		.into();
		root.add_child(Node::function("main", Vec::new(), void_type, vec![call]).into());

		// Indexing the image argument before validation would panic for this empty call.
		compile_error(ExecutableProgram::compile(root.into()))
	};
	assert_eq!(statement_error, VmError::CallArgumentMismatch { expected: 3, found: 0 });
}

#[test]
fn void_statement_calls_execute_without_a_result_register() {
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	root.add_child(
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", u32_type).into()],
			},
			0,
			0,
			false,
			true,
		)
		.into(),
	);
	let program = compile_to_besl(
		r#"
		write_result: fn (value: u32) -> void {
			result.value = value;
		}
		main: fn () -> void {
			write_result(7);
		}
		"#,
		Some(root),
	)
	.expect("Expected lexed void call");
	let executable = ExecutableProgram::compile(program).expect("Expected void statement call lowering");
	let slot = DescriptorSlot::new(0, 0);
	let mut result = Buffer::new(executable.buffer_layout(slot).expect("Expected result layout").clone());
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot, &mut result);
	executable.run_main(&mut descriptors).expect("Expected void call execution");
	assert_eq!(result.read("value").expect("Expected result"), Value::U32(7));
}

#[test]
fn comparison_literals_inherit_the_typed_operand() {
	let mut root = Node::root();
	let bool_type = root.get_child("bool").expect("Expected bool type");
	root.add_child(
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", bool_type).into()],
			},
			0,
			0,
			false,
			true,
		)
		.into(),
	);

	let program = compile_to_besl(
		r#"
		main: fn () -> void {
			let signed: i32 = 2;
			result.value = signed < 3;
		}
		"#,
		Some(root),
	)
	.expect("Expected lexed comparison");
	let executable = ExecutableProgram::compile(program).expect("Expected typed comparison lowering");
	let slot = DescriptorSlot::new(0, 0);
	let mut result = Buffer::new(executable.buffer_layout(slot).expect("Expected result layout").clone());
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_buffer(slot, &mut result);
	executable.run_main(&mut descriptors).expect("Expected comparison execution");

	assert_eq!(result.read("value").expect("Expected comparison result"), Value::Bool(true));
}

#[test]
fn incompatible_typed_comparison_operands_fail_during_compilation() {
	let mut root = Node::root();
	let bool_type = root.get_child("bool").expect("Expected bool type");
	root.add_child(
		Node::binding(
			"result",
			BindingTypes::Buffer {
				members: vec![Node::member("value", bool_type).into()],
			},
			0,
			0,
			false,
			true,
		)
		.into(),
	);
	let program = compile_to_besl(
		r#"
		main: fn () -> void {
			let signed: i32 = 1;
			let unsigned: u32 = 1;
			result.value = signed == unsigned;
		}
		"#,
		Some(root),
	)
	.expect("Expected lexed comparison");

	assert_eq!(
		compile_error(ExecutableProgram::compile(program)),
		VmError::TypeMismatch {
			expected: "i32".to_string(),
			found: "u32".to_string(),
		}
	);
}

#[test]
fn atomic_add_requires_read_and_write_descriptor_access() {
	assert_eq!(
		compile_error(compile_atomic_add(true, false)),
		VmError::DescriptorAccessDenied {
			slot: DescriptorSlot::new(0, 0),
			access: "write",
		}
	);
	assert_eq!(
		compile_error(compile_atomic_add(false, true)),
		VmError::DescriptorAccessDenied {
			slot: DescriptorSlot::new(0, 0),
			access: "read",
		}
	);
	compile_atomic_add(true, true).expect("Read-write atomics must compile");
}

fn compile_atomic_add(read: bool, write: bool) -> Result<ExecutableProgram, VmError> {
	let mut root = Node::root();
	let u32_type = root.get_child("u32").expect("Expected u32 type");
	let atomic_u32_type = root.add_child(Node::r#struct("atomicu32", Vec::new()).into());
	root.add_child(
		Node::binding(
			"counter",
			BindingTypes::Buffer {
				members: vec![Node::member("count", atomic_u32_type.clone()).into()],
			},
			0,
			0,
			read,
			write,
		)
		.into(),
	);
	let atomic_add = root.add_child(Node::intrinsic("atomic_add", Vec::new(), u32_type.clone()).into());
	atomic_add.borrow_mut().add_children(vec![
		Node::new(Nodes::Parameter {
			name: "value".to_string(),
			r#type: atomic_u32_type,
		})
		.into(),
		Node::new(Nodes::Parameter {
			name: "increment".to_string(),
			r#type: u32_type,
		})
		.into(),
	]);
	let program = compile_to_besl(
		r#"
		main: fn () -> void {
			atomic_add(counter.count, 1);
		}
		"#,
		Some(root),
	)
	.expect("Expected lexed atomic program");
	ExecutableProgram::compile(program)
}

fn compile_error(result: Result<ExecutableProgram, VmError>) -> VmError {
	match result {
		Ok(_) => panic!("Expected VM compilation to fail"),
		Err(error) => error,
	}
}
