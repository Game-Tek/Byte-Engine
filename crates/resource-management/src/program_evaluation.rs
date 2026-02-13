use std::cell::RefCell;

/// The `BindingUsage` struct describes a used binding in a BESL program.
#[derive(Clone, Debug)]
pub struct BindingUsage {
    pub set: u32,
    pub binding: u32,
    pub read: bool,
    pub write: bool,
}

/// The `ProgramEvaluation` struct holds information derived from evaluating a BESL program.
#[derive(Clone, Debug)]
pub struct ProgramEvaluation {
    bindings: Vec<BindingUsage>,
}

impl ProgramEvaluation {
    pub fn from_program(program: &besl::NodeReference) -> Result<Self, String> {
        let main = program.get_main().ok_or_else(|| {
			"Main function not found. The program description likely does not define a `main` function.".to_string()
		})?;

        Self::from_main(&main)
    }

    pub fn from_main(main_function_node: &besl::NodeReference) -> Result<Self, String> {
        {
            let node_borrow = RefCell::borrow(main_function_node);
            let node_ref = node_borrow.node();

            match node_ref {
                besl::Nodes::Function { name, .. } => {
                    if name != "main" {
                        return Err("Main node is not `main`. The program description likely passed a non-main function node.".to_string());
                    }
                }
                _ => {
                    return Err("Invalid main node. The program description likely contains a `main` symbol that is not a function.".to_string());
                }
            }
        }

        let mut bindings = Vec::with_capacity(16);
        build_bindings(&mut bindings, main_function_node);

        bindings.sort_by(|a, b| {
            if a.set == b.set {
                a.binding.cmp(&b.binding)
            } else {
                a.set.cmp(&b.set)
            }
        });

        Ok(Self { bindings })
    }

    pub fn bindings(&self) -> &[BindingUsage] {
        &self.bindings
    }

    pub fn into_bindings(self) -> Vec<BindingUsage> {
        self.bindings
    }
}

fn build_bindings(bindings: &mut Vec<BindingUsage>, node: &besl::NodeReference) {
    let node_borrow = RefCell::borrow(node);
    let node_ref = node_borrow.node();

    match node_ref {
        besl::Nodes::Function { statements, .. } => {
            for statement in statements {
                build_bindings(bindings, statement);
            }
        }
        besl::Nodes::Expression(expressions) => {
            match expressions {
                besl::Expressions::FunctionCall {
                    parameters,
                    function,
                } => {
                    build_bindings(bindings, function);
                    for parameter in parameters {
                        build_bindings(bindings, parameter);
                    }
                }
                besl::Expressions::Accessor { left, right } => {
                    build_bindings(bindings, left);
                    build_bindings(bindings, right);
                }
                besl::Expressions::Expression { elements } => {
                    for element in elements {
                        build_bindings(bindings, element);
                    }
                }
                besl::Expressions::IntrinsicCall {
                    intrinsic,
                    elements,
                } => {
                    for element in elements {
                        build_bindings(bindings, element);
                    }
                    build_bindings(bindings, intrinsic);
                }
                besl::Expressions::Return | besl::Expressions::Literal { .. } => {
                    // Do nothing
                }
                besl::Expressions::Macro { body, .. } => {
                    build_bindings(bindings, body);
                }
                besl::Expressions::Member { source, .. } => {
                    build_bindings(bindings, source);
                }
                besl::Expressions::Operator { left, right, .. } => {
                    build_bindings(bindings, left);
                    build_bindings(bindings, right);
                }
                besl::Expressions::VariableDeclaration { r#type, .. } => {
                    build_bindings(bindings, r#type);
                }
            }
        }
        besl::Nodes::Binding {
            set,
            binding,
            read,
            write,
            ..
        } => {
            if bindings
                .iter()
                .find(|b| b.binding == *binding && b.set == *set)
                .is_none()
            {
                bindings.push(BindingUsage {
                    binding: *binding,
                    set: *set,
                    read: *read,
                    write: *write,
                });
            }
        }
        besl::Nodes::Raw { input, output, .. } => {
            for input in input {
                build_bindings(bindings, input);
            }
            for output in output {
                build_bindings(bindings, output);
            }
        }
        besl::Nodes::Struct { fields, .. } => {
            for member in fields {
                build_bindings(bindings, member);
            }
        }
        besl::Nodes::Intrinsic {
            elements, r#return, ..
        } => {
            for element in elements {
                build_bindings(bindings, element);
            }
            build_bindings(bindings, r#return);
        }
        besl::Nodes::Literal { value, .. } => {
            build_bindings(bindings, value);
        }
        besl::Nodes::Member { r#type, .. } => {
            build_bindings(bindings, r#type);
        }
        besl::Nodes::Input { format, .. } | besl::Nodes::Output { format, .. } => {
            build_bindings(bindings, format);
        }
        besl::Nodes::Null => {
            // Do nothing
        }
        besl::Nodes::Parameter { r#type, .. } => {
            build_bindings(bindings, r#type);
        }
        besl::Nodes::PushConstant { members } => {
            for member in members {
                build_bindings(bindings, member);
            }
        }
        besl::Nodes::Scope { children, .. } => {
            for child in children {
                build_bindings(bindings, child);
            }
        }
        besl::Nodes::Specialization { r#type, .. } => {
            build_bindings(bindings, r#type);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::shader_generator;

    #[test]
    fn bindings_from_main() {
        let main = shader_generator::tests::bindings();

        let evaluation = ProgramEvaluation::from_main(&main).expect("Failed to evaluate program");
        let bindings = evaluation.bindings();

        assert_eq!(bindings.len(), 3);

        let buffer_binding = &bindings[0];
        assert_eq!(buffer_binding.binding, 0);
        assert_eq!(buffer_binding.set, 0);
        assert_eq!(buffer_binding.read, true);
        assert_eq!(buffer_binding.write, true);

        let image_binding = &bindings[1];
        assert_eq!(image_binding.binding, 1);
        assert_eq!(image_binding.set, 0);
        assert_eq!(image_binding.read, false);
        assert_eq!(image_binding.write, true);

        let texture_binding = &bindings[2];
        assert_eq!(texture_binding.binding, 0);
        assert_eq!(texture_binding.set, 1);
        assert_eq!(texture_binding.read, true);
        assert_eq!(texture_binding.write, false);
    }

    #[test]
    fn bindings_from_program() {
        let script = r#"
		main: fn () -> void {
			buff;
			image;
			texture;
		}
		"#;

        let mut root_node = besl::Node::root();

        let float_type = root_node.get_child("f32").unwrap();

        root_node.add_children(vec![
            besl::Node::binding(
                "buff",
                besl::BindingTypes::Buffer {
                    members: vec![besl::Node::member("member", float_type).into()],
                },
                0,
                0,
                true,
                true,
            )
            .into(),
            besl::Node::binding(
                "image",
                besl::BindingTypes::Image {
                    format: "r8".to_string(),
                },
                0,
                1,
                false,
                true,
            )
            .into(),
            besl::Node::binding(
                "texture",
                besl::BindingTypes::CombinedImageSampler {
                    format: "".to_string(),
                },
                1,
                0,
                true,
                false,
            )
            .into(),
        ]);

        let program_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();
        let evaluation =
            ProgramEvaluation::from_program(&program_node).expect("Failed to evaluate program");
        let bindings = evaluation.bindings();

        assert_eq!(bindings.len(), 3);
    }
}
