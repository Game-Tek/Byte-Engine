use besl::{BeslStruct, BeslStructDefinition};

#[derive(BeslStruct)]
#[allow(dead_code)]
struct Light {
	position: Vec3f,
	color: Vec3f,
	indices: [u32; 3],
}

#[derive(BeslStruct)]
#[besl_name = "MaterialParameters"]
struct Material {
	#[besl_name = "base_color"]
	#[besl_type = "vec4f"]
	albedo: Color,
	#[besl_type = "Texture2D"]
	base_color_texture: TextureHandle,
}

struct Vec3f;
struct Vec4f;
struct Color;
struct TextureHandle;

#[test]
fn derive_besl_struct_builds_a_parser_node_definition() {
	let mut node = Light::besl_struct_node();

	match node.node_mut() {
		besl::parser::Nodes::Struct { name, fields } => {
			assert_eq!((*name).to_string(), "Light");
			assert_eq!(fields.len(), 3);

			match fields[0].node_mut() {
				besl::parser::Nodes::Member { name, r#type } => {
					assert_eq!((*name).to_string(), "position");
					assert_eq!(r#type, "Vec3f");
				}
				_ => panic!("Expected member node."),
			}

			match fields[2].node_mut() {
				besl::parser::Nodes::Member { name, r#type } => {
					assert_eq!((*name).to_string(), "indices");
					assert_eq!(r#type, "u32[3]");
				}
				_ => panic!("Expected member node."),
			}
		}
		_ => panic!("Expected struct node."),
	}
}

#[test]
fn derive_besl_struct_supports_instance_access_and_overrides() {
	let material = Material {
		albedo: Color,
		base_color_texture: TextureHandle,
	};
	let mut node = material.besl_definition();

	let _ = (&material.albedo, &material.base_color_texture);
	let _ = Vec4f;

	match node.node_mut() {
		besl::parser::Nodes::Struct { name, fields } => {
			assert_eq!((*name).to_string(), "MaterialParameters");
			assert_eq!(fields.len(), 2);

			match fields[0].node_mut() {
				besl::parser::Nodes::Member { name, r#type } => {
					assert_eq!((*name).to_string(), "base_color");
					assert_eq!(r#type, "vec4f");
				}
				_ => panic!("Expected member node."),
			}

			match fields[1].node_mut() {
				besl::parser::Nodes::Member { name, r#type } => {
					assert_eq!((*name).to_string(), "base_color_texture");
					assert_eq!(r#type, "Texture2D");
				}
				_ => panic!("Expected member node."),
			}
		}
		_ => panic!("Expected struct node."),
	}
}
