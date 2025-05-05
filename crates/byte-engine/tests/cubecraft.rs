//! Cubecraft example application
//! This demonstrates a simple first person game, which is definitely not a clone of Minecraft.
//! It uses the Byte-Engine to create a simple game with a player character that can move around and jump.
//! It also includes a simple physics engine to handle collisions and movement.

use std::borrow::Borrow;
use std::cell::RefCell;
use std::rc::Rc;

use byte_engine::constants::FORWARD;
use byte_engine::constants::RIGHT;
use byte_engine::constants::UP;
use byte_engine::core::entity::EntityBuilder;
use byte_engine::core::listener::EntitySubscriber;
use byte_engine::core::listener::Listener;
use byte_engine::core::Entity;
use byte_engine::core::EntityHandle;

use byte_engine::core::Task;
use byte_engine::gameplay::space::Spawn;
use byte_engine::gameplay::Anchor;
use byte_engine::gameplay::Positionable;
use byte_engine::gameplay::Transform;
use byte_engine::rendering::aces_tonemap_render_pass::AcesToneMapPass;
use byte_engine::rendering::common_shader_generator::CommonShaderGenerator;
use byte_engine::rendering::render_pass::RenderPass;
use byte_engine::rendering::render_pass::RenderPassBuilder;
use byte_engine::rendering::view::View;
use byte_engine::{
    application::{Application, Parameter},
    camera::Camera,
    input::{Action, ActionBindingDescription, Function},
    rendering::directional_light::DirectionalLight,
    Vector3,
};
use ghi::graphics_hardware_interface::Device as _;
use ghi::raster_pipeline;
use ghi::BoundRasterizationPipelineMode;
use ghi::CommandBufferRecordable;
use ghi::RasterizationRenderPassMode;
use resource_management::glsl;
use resource_management::resources::image::Image;
use resource_management::Reference;
use resource_management::ResourceManager;
use utils::hash::HashMap;
use utils::hash::HashMapExt;
use utils::sync::RwLock;
use utils::Extent;

#[ignore]
#[test]
fn cubecraft() {
    // Create the Byte-Engine application
    let mut app = byte_engine::application::GraphicsApplication::new(
        "Cubecraft",
        &[
            Parameter::new("resources-path", "../../resources"),
            Parameter::new("assets-path", "../../assets"),
        ],
    );

    {
        let generator = {
            let common_shader_generator = CommonShaderGenerator::new();
            common_shader_generator
        };

        byte_engine::application::graphics_application::setup_default_resource_and_asset_management(
            &mut app, generator,
        );
    }

    {
        let mut renderer = app.get_renderer_handle().write();

        renderer.add_render_pass(|c| {
            app.get_root_space_handle()
                .spawn(CubeCraftRenderPass::create(
                    c,
                    app.get_resource_manager_handle().clone(),
                ))
        });

        renderer.add_render_pass(|c| {
            app.get_root_space_handle()
                .spawn(AcesToneMapPass::create(c))
        });
    }

    byte_engine::application::graphics_application::setup_default_input(&mut app);
    byte_engine::application::graphics_application::setup_default_window(&mut app);

    // Get the root space handle
    let space_handle = app.get_root_space_handle().clone();

    // Create the lookaround action handle
    let lookaround_action_handle = space_handle.spawn(Action::<Vector3>::new(
        "Lookaround",
        &[
            ActionBindingDescription::new("Mouse.Position")
                .mapped(Vector3::new(1f32, 1f32, 1f32).into(), Function::Sphere),
            ActionBindingDescription::new("Gamepad.RightStick"),
        ],
    ));

    // Create the move action
    let move_action_handle = space_handle.spawn(Action::<Vector3>::new(
        "Move",
        &[
            ActionBindingDescription::new("Keyboard.W").mapped(FORWARD.into(), Function::Linear),
            ActionBindingDescription::new("Keyboard.S").mapped((-FORWARD).into(), Function::Linear),
            ActionBindingDescription::new("Keyboard.A").mapped((-RIGHT).into(), Function::Linear),
            ActionBindingDescription::new("Keyboard.D").mapped(RIGHT.into(), Function::Linear),
        ],
    ));

    // Create the jump action
    let jump_action_handle = space_handle.spawn(Action::<bool>::new(
        "Jump",
        &[
            ActionBindingDescription::new("Keyboard.Space"),
            ActionBindingDescription::new("Gamepad.A"),
        ],
    ));

    // Create the right hand action
    let fire_action_handle = space_handle.spawn(Action::<bool>::new(
        "RightHand",
        &[
            ActionBindingDescription::new("Mouse.LeftButton"),
            ActionBindingDescription::new("Gamepad.RightTrigger"),
        ],
    ));

    let exit_action_handle = space_handle.spawn(Action::<bool>::new(
        "Exit",
        &[ActionBindingDescription::new("Keyboard.Escape")],
    ));

    space_handle.spawn(ChunkLoader::create());

    let mut camera = Camera::new(Vector3::new(0.0, 0.0, 0.0));

    camera.set_fov(90.0);

    // Create the camera
    let camera = space_handle.spawn(camera);

    // Create the directional light
    let _ = space_handle.spawn(DirectionalLight::new(maths_rs::normalize(-UP), 4000f32));

    {
        let camera = camera.clone();

        lookaround_action_handle
            .write()
            .value()
            .add(move |value: &Vector3| {
                let mut camera = camera.write();

                camera.set_orientation(*value);
            });
    }

    let player = space_handle.spawn(Player::create());

    let mut anchor = Anchor::new(Transform::default());

    anchor.attach_with_offset(camera, UP * 1.8);

    let anchor = space_handle.spawn(anchor);

    {
        let positionable = anchor.clone();
        let move_action_handle = move_action_handle.clone();

        space_handle.spawn(Task::tick(move || {
            let value = move_action_handle.read().value().get();

            let mut positionable = positionable.write();

            let position = positionable.transform().get_position();
            positionable.transform_mut().set_position(position + value);
        }));
    }

    let _ = space_handle.spawn(Physics::create(anchor));

    app.do_loop()
}

const CHUNK_SIZE: i32 = 16;
const HALF_CHUNK_SIZE: i32 = CHUNK_SIZE / 2;

struct ChunkLoader {
    loaded: Vec<Location>,
    camera: Option<EntityHandle<Camera>>,
}

impl ChunkLoader {
    fn new() -> Self {
        ChunkLoader {
            loaded: Vec::new(),
            camera: None,
        }
    }

    fn create() -> EntityBuilder<'static, Self> {
        EntityBuilder::new(ChunkLoader::new()).listen_to::<Camera>().then(|space, handle| {
			let a = space.clone();
			space.spawn(Task::tick(move || {
				let mut chunk_loader = handle.write();

				let p = if let Some(camera) = &chunk_loader.camera {
					let camera = camera.read();
					let position = camera.get_position();

					let chunk_x = (position.x / 16.0).round() as i32;
					let chunk_z = (position.z / 16.0).round() as i32;

					if !chunk_loader.loaded.contains(&(chunk_x, 0, chunk_z)) {
						let blocks = ((chunk_x * CHUNK_SIZE - HALF_CHUNK_SIZE)..(chunk_x * CHUNK_SIZE + HALF_CHUNK_SIZE))
							.map(move |x| {
								((chunk_z * CHUNK_SIZE - HALF_CHUNK_SIZE)..(chunk_z * CHUNK_SIZE + HALF_CHUNK_SIZE))
									.map(move |z| {
										(-HALF_CHUNK_SIZE..HALF_CHUNK_SIZE).filter_map(move |y| {
											let position = (x, y, z);
											let block = make_block(position);
			
											if block != Blocks::Air {
												Some(Block::create(position, block))
											} else {
												None
											}
										})
									})
									.flatten()
							})
							.flatten()
						.collect::<Vec<_>>();

						a.spawn(blocks);

						Some((chunk_x, 0, chunk_z))
					} else {
						None
					}
				} else {
					None
				};

				if let Some(p) = p {
					chunk_loader.loaded.push(p);
				}
			}));
        })
    }
}

impl Entity for ChunkLoader {}

impl EntitySubscriber<Camera> for ChunkLoader {
    fn on_create<'a>(&'a mut self, handle: EntityHandle<Camera>, params: &'a Camera) -> () {
        self.camera = Some(handle.clone());
    }
}

struct Player {
    position: Vector3,
}

impl Player {
    fn new() -> Self {
        Player {
            position: Vector3::new(0.0, 0.0, 0.0),
        }
    }

    fn create() -> EntityBuilder<'static, Self> {
        Player::new().into()
    }
}

impl Entity for Player {}

impl Positionable for Player {
    fn get_position(&self) -> Vector3 {
        self.position
    }

    fn set_position(&mut self, position: Vector3) {
        self.position = position;
    }
}

struct Physics {
    player: Option<EntityHandle<dyn Positionable>>,
    blocks: Vec<EntityHandle<Block>>,
}

impl Physics {
    fn new(player: EntityHandle<dyn Positionable>) -> Self {
        Physics {
            player: Some(player),
            blocks: Vec::new(),
        }
    }

    fn create(player: EntityHandle<dyn Positionable>) -> EntityBuilder<'static, Self> {
        EntityBuilder::new(Physics::new(player))
            .listen_to::<Block>()
            .then(|space, handle| {
                space.spawn(Task::tick(move || {
                    handle.write().update();
                }));
            })
    }

    fn update(&self) {
        if let Some(player) = &self.player {
            let mut player = player.write();
            let position = player.get_position();

            for block in &self.blocks {
                let block = block.read();
                let block_position = (
                    block.position.0 as f32,
                    block.position.1 as f32,
                    block.position.2 as f32,
                );

                if position.x > block_position.0 - 0.5
                    && position.x < block_position.0 + 0.5
                    && position.z > block_position.2 - 0.5
                    && position.z < block_position.2 + 0.5
                {
                    player.set_position(Vector3::new(
                        position.x,
                        block_position.1 + 0.5,
                        position.z,
                    ));
                }
            }
        }
    }
}

impl Entity for Physics {}

impl EntitySubscriber<Block> for Physics {
    fn on_create<'a>(&'a mut self, handle: EntityHandle<Block>, params: &'a Block) -> () {
        self.blocks.push(handle.clone());
    }
}

impl EntitySubscriber<Player> for Physics {
    fn on_create<'a>(&'a mut self, handle: EntityHandle<Player>, params: &'a Player) -> () {
        self.player = Some(handle.clone());
    }
}

#[derive(Clone, Copy)]
struct Block {
    position: Location,
    block: Blocks,
}

impl Block {
    fn new(position: Location, block: Blocks) -> Self {
        Block { position, block }
    }

    fn create(position: Location, block: Blocks) -> EntityBuilder<'static, Self> {
        Block { position, block }.into()
    }
}

impl Entity for Block {
    fn call_listeners<'a>(
        &'a self,
        listener: &'a byte_engine::core::listener::BasicListener,
        handle: EntityHandle<Self>,
    ) -> ()
    where
        Self: Sized,
    {
        listener.invoke_for(handle.clone(), self);
    }
}

type Location = (i32, i32, i32);

struct RenderParams {
    index_count: u32,
    vertex_count: u32,
    instance_count: u32,
}

struct CubeCraftRenderPass {
    vertex_buffer: ghi::BufferHandle<[((f32, f32, f32), (f32, f32)); 4]>,
    index_buffer: ghi::BufferHandle<[u16; 6]>,

    camera_data_buffer: ghi::BufferHandle<maths_rs::Mat4f>,

    face_data_buffer: ghi::BufferHandle<[FaceData; 16 * 16 * 256 * 32]>,

    set: ghi::DescriptorSetHandle,
    binding: ghi::DescriptorSetBindingHandle,

    layout: ghi::PipelineLayoutHandle,
    pipeline: ghi::PipelineHandle,

    render_params: Rc<RefCell<RenderParams>>,

    ghi: Rc<RwLock<ghi::Device>>,

    blocks: Vec<Block>,

    camera: Option<EntityHandle<Camera>>,
}

impl Entity for CubeCraftRenderPass {}

impl EntitySubscriber<Block> for CubeCraftRenderPass {
    fn on_create<'a>(&'a mut self, handle: EntityHandle<Block>, params: &'a Block) -> () {
        self.blocks.push(*params);
    }
}

impl EntitySubscriber<Camera> for CubeCraftRenderPass {
    fn on_create<'a>(&'a mut self, handle: EntityHandle<Camera>, params: &'a Camera) -> () {
        self.camera = Some(handle.clone());
    }
}

impl CubeCraftRenderPass {
    pub fn create<'a>(
        render_pass_builder: &'a mut RenderPassBuilder<'_>,
        resource_manager: EntityHandle<ResourceManager>,
    ) -> EntityBuilder<'static, Self>
    where
        Self: Sized,
    {
        let ghi = render_pass_builder.ghi();
        let mut ghi = ghi.write();

        let render_to_main = render_pass_builder.render_to("main");
        let render_to_depth = render_pass_builder.render_to("depth");

        let vertex_buffer: ghi::BufferHandle<[((f32, f32, f32), (f32, f32)); 4]> = ghi.create_buffer(
            Some("vertices"),
            ghi::Uses::Vertex,
            ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead,
            ghi::UseCases::DYNAMIC,
        );

        let index_buffer: ghi::BufferHandle<[u16; 6]> = ghi.create_buffer(
            Some("indices"),
            ghi::Uses::Index,
            ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead,
            ghi::UseCases::DYNAMIC,
        );

        let descriptor_set_template = ghi.create_descriptor_set_template(
            Some("template"),
            &[
                ghi::DescriptorSetBindingTemplate::new(
                    0,
                    ghi::DescriptorType::StorageBuffer,
                    ghi::Stages::VERTEX,
                ),
                ghi::DescriptorSetBindingTemplate::new(
                    1,
                    ghi::DescriptorType::StorageBuffer,
                    ghi::Stages::VERTEX | ghi::Stages::FRAGMENT,
                ),
                ghi::DescriptorSetBindingTemplate::new_array(
                    2,
                    ghi::DescriptorType::CombinedImageSampler,
                    ghi::Stages::FRAGMENT,
                    16,
                ),
            ],
        );
        let layout = ghi.create_pipeline_layout(&[descriptor_set_template], &[]);

        let v_shader_source = r#"#version 450 core
		#pragma shader_stage(vertex)
		#extension GL_EXT_shader_8bit_storage: require
		#extension GL_EXT_shader_16bit_storage: require
		#extension GL_EXT_shader_explicit_arithmetic_types: require
		#extension GL_EXT_scalar_block_layout: require
		#extension GL_EXT_nonuniform_qualifier: require
		// Row major matrices in buffers
		layout(row_major) buffer;

		layout(location = 0) in vec3 in_position;
		layout(location = 1) in vec2 in_uv;
		layout(location = 0) out uint instance_id;
		layout(location = 1) out vec2 out_uv;

		layout(set = 0, binding = 0) readonly buffer Camera {
			mat4 vp;
		} camera;

		struct Face {
			uint16_t block;
			uint8_t direction;
			vec3 position;
		};

		layout(set = 0, binding = 1, scalar) readonly buffer Faces {
			Face faces[];
		};

		void main() {
			Face face = faces[gl_InstanceIndex];

			float flip = face.direction == 1 || face.direction == 3 || face.direction == 5 ? -1.0 : 1.0;

			vec3 position = vec3(0.0, 0.0, 0.0);

			switch (uint32_t(face.direction)) {
				case 1:
				case 2:
					position.x = flip * 0.5;
					position.y = in_position.y;
					position.z = flip * in_position.x;
					break;
				case 3:
				case 4:
					position.x = in_position.x;
					position.y = flip * 0.5;
					position.z = flip * in_position.y;
					break;
				case 5:
				case 6:
					position.x = -flip * in_position.x;
					position.y = in_position.y;
					position.z = flip * 0.5;
					break;
				default:
					break;
			}

			position += face.position;
			gl_Position = camera.vp * vec4(position, 1.0);
			instance_id = gl_InstanceIndex;
			out_uv = in_uv;
		}
		"#;

        let f_shader_source = r#"#version 450 core
		#pragma shader_stage(fragment)
		#extension GL_EXT_shader_8bit_storage: require
		#extension GL_EXT_shader_16bit_storage: require
		#extension GL_EXT_scalar_block_layout: require
		#extension GL_EXT_nonuniform_qualifier: require

		layout(location = 0) out vec4 out_color;
		layout(location = 0) in flat uint instance_id;
		layout(location = 1) in vec2 in_uv;

		struct Face {
			uint16_t block;
			uint8_t direction;
			vec3 position;
		};

		layout(set = 0, binding = 1, scalar) readonly buffer Faces {
			Face faces[];
		};

		layout(set = 0, binding = 2) uniform sampler2D[] textures;

		void main() {
			uint in_block = uint(faces[instance_id].block);

			vec2 uv = in_uv;

			out_color = texture(textures[in_block], uv);
		}
		"#;

        let v_shader_artifact = glsl::compile(v_shader_source, "Cube Vertex Shader").unwrap();
        let f_shader_artifact = glsl::compile(f_shader_source, "Cube Fragment Shader").unwrap();

        let v_shader = ghi
            .create_shader(
                None,
                ghi::ShaderSource::SPIRV(v_shader_artifact.borrow().into()),
                ghi::ShaderTypes::Vertex,
                &[ghi::ShaderBindingDescriptor::new(
                    0,
                    0,
                    ghi::AccessPolicies::READ,
                )],
            )
            .unwrap();
        let f_shader = ghi
            .create_shader(
                None,
                ghi::ShaderSource::SPIRV(f_shader_artifact.borrow().into()),
                ghi::ShaderTypes::Fragment,
                &[
                    ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ),
                    ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ),
                    ghi::ShaderBindingDescriptor::new(0, 2, ghi::AccessPolicies::READ),
                ],
            )
            .unwrap();

        // TODO: notify user if provided shaders don't consume any bindings in the layout
        let pipeline = ghi.create_raster_pipeline(raster_pipeline::Builder::new(
            layout,
            &[ghi::VertexElement::new(
                "POSITION",
                ghi::DataTypes::Float3,
                0,
            ), ghi::VertexElement::new(
                "UV",
                ghi::DataTypes::Float2,
                0,
            )],
            &[
                ghi::ShaderParameter::new(&v_shader, ghi::ShaderTypes::Vertex),
                ghi::ShaderParameter::new(&f_shader, ghi::ShaderTypes::Fragment),
            ],
            &[render_to_main.into(), render_to_depth.into()],
        ));

        let camera_data_buffer = ghi.create_buffer(
            Some("camera"),
            ghi::Uses::Storage,
            ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead,
            ghi::UseCases::DYNAMIC,
        );
        let face_data_buffer = ghi.create_buffer(
            Some("face_data_buffer"),
            ghi::Uses::Storage,
            ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead,
            ghi::UseCases::DYNAMIC,
        );

        let set = ghi.create_descriptor_set(None, &descriptor_set_template);

		let sampler = ghi.build_sampler(
            ghi::sampler::Builder::new().filtering_mode(ghi::FilteringModes::Closest),
        );

		let camera_data_buffer_binding = ghi.create_descriptor_binding(
            set,
            ghi::BindingConstructor::buffer(
                &ghi::DescriptorSetBindingTemplate::new(
                    0,
                    ghi::DescriptorType::StorageBuffer,
                    ghi::Stages::VERTEX,
                ),
                camera_data_buffer.into(),
            ),
        );
        let face_data_buffer_binding = ghi.create_descriptor_binding(
            set,
            ghi::BindingConstructor::buffer(
                &ghi::DescriptorSetBindingTemplate::new(
                    1,
                    ghi::DescriptorType::StorageBuffer,
                    ghi::Stages::VERTEX | ghi::Stages::FRAGMENT,
                ),
                face_data_buffer.into(),
            ),
        );
        let texture_binding = ghi.create_descriptor_binding_array(
            set,
            &ghi::DescriptorSetBindingTemplate::new_array(
                2,
                ghi::DescriptorType::CombinedImageSampler,
                ghi::Stages::FRAGMENT,
                16,
            ),
        );

		for (block_type, a) in [(GRASS_TOP_FACE, "grass_carried.png"), (GRASS_SIDE_FACE, "grass_side_carried.png"), (STONE_FACE, "stone.png"), (DIRT_FACE, "dirt.png")] {
			let mut request: Reference<Image> = resource_manager
				.read()
				.request(a)
				.unwrap();

			let texture = ghi.create_image(
				None,
				Extent::square(16),
				ghi::Formats::RGBA8(ghi::Encodings::sRGB),
				ghi::Uses::Image,
				ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead,
				ghi::UseCases::STATIC,
				1,
			);
	
			ghi.write_texture(texture, |s| {
				request.load(s.into());
			});

			if block_type == GRASS_TOP_FACE {
				ghi.write(&[ghi::DescriptorWrite::combined_image_sampler_array(
					texture_binding,
					texture,
					sampler,
					ghi::Layouts::Read,
					0 as _,
				)]);
			}

			ghi.write(&[ghi::DescriptorWrite::combined_image_sampler_array(
				texture_binding,
				texture,
				sampler,
				ghi::Layouts::Read,
				block_type as _,
			)]);
		}

		*ghi.get_mut_buffer_slice(vertex_buffer) = [
			((-0.5, 0.5, 0.0), (0.0, 0.0)),
			((0.5, 0.5, 0.0), (1.0, 0.0)),
			((0.5, -0.5, 0.0), (1.0, 1.0)),
			((-0.5, -0.5, 0.0), (0.0, 1.0)),
		];

		*ghi.get_mut_buffer_slice(index_buffer) = [
			0, 1, 2, 2, 3, 0,
		];

        drop(ghi);

        let render_params = RenderParams {
            index_count: 6,
            vertex_count: 4,
            instance_count: 0,
        };

        EntityBuilder::new(Self {
            vertex_buffer,
            index_buffer,

            camera_data_buffer,

            face_data_buffer,

            set,
            binding: camera_data_buffer_binding,

            layout,
            pipeline,

            render_params: Rc::new(RefCell::new(render_params)),

            ghi: render_pass_builder.ghi(),

            blocks: Vec::with_capacity(8192 * 32),

            camera: None,
        })
        .listen_to::<Block>()
        .listen_to::<Camera>()
    }
}

impl RenderPass for CubeCraftRenderPass {
    fn get_read_attachments() -> Vec<&'static str>
    where
        Self: Sized,
    {
        vec![]
    }

    fn get_write_attachments() -> Vec<&'static str>
    where
        Self: Sized,
    {
        vec!["main"]
    }

    fn prepare(&self, ghi: &mut ghi::Device, extent: utils::Extent) {
        if let Some(camera) = &self.camera {
            let camera = camera.read();

            let view = View::new_perspective(
                camera.get_fov(),
                extent.aspect_ratio(),
                0.1f32,
                100f32,
                camera.get_position(),
                camera.get_orientation(),
            );

            *ghi.get_mut_buffer_slice(self.camera_data_buffer) = view.view_projection();
        }

        let faces = build_cube_faces(&self.blocks);

        ghi.get_mut_buffer_slice(self.face_data_buffer)[..faces.len()].copy_from_slice(&faces);

        let mut render_params = self.render_params.borrow_mut();

        render_params.instance_count = faces.len() as u32;
    }

    fn record(
        &self,
        command_buffer_recording: &mut ghi::CommandBufferRecording,
        extent: utils::Extent,
        attachments: &[ghi::AttachmentInformation],
    ) {
        let (vertex_count, index_count, instance_count) = {
            let render_params = self.render_params.borrow_mut();
            (
                render_params.vertex_count,
                render_params.index_count,
                render_params.instance_count,
            )
        };

        command_buffer_recording.bind_vertex_buffers(&[ghi::BufferDescriptor::new(
            self.vertex_buffer.into(),
            0,
            vertex_count as usize,
            0,
        )]);
        command_buffer_recording.bind_index_buffer(&ghi::BufferDescriptor::new(
            self.index_buffer.into(),
            0,
            index_count as usize,
            0,
        ));
        let render_pass = command_buffer_recording.start_render_pass(extent, attachments);
        render_pass.bind_descriptor_sets(&self.layout, &[self.set]);
        let pipeline = render_pass.bind_raster_pipeline(&self.pipeline);
        pipeline.draw_indexed(index_count, instance_count, 0, 0, 0);
        render_pass.end_render_pass();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Direction {
    Left = 1,
    Right = 2,
    Down = 3,
    Up = 4,
    Forward = 5,
    Backward = 6,
} 

impl Direction {
	fn to_normal(&self) -> (i32, i32, i32) {
		match self {
			Direction::Left => (-1, 0, 0),
			Direction::Right => (1, 0, 0),
			Direction::Down => (0, -1, 0),
			Direction::Up => (0, 1, 0),
			Direction::Forward => (0, 0, -1),
			Direction::Backward => (0, 0, 1),
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Blocks {
    Air,
    Grass,
    Stone,
    Dirt,
}

const GRASS_TOP_FACE: u32 = 1;
const GRASS_SIDE_FACE: u32 = 2;
const STONE_FACE: u32 = 3;
const DIRT_FACE: u32 = 4;

fn make_block(position: Location) -> Blocks {
    if position.1 > 0 {
        Blocks::Air
    } else {
        if position.1 == 0 {
            Blocks::Grass
        } else if position.1 == -1 {
            Blocks::Dirt
        } else {
            Blocks::Stone
        }
    }
}

type Vertex = (f32, f32, f32);

#[derive(Clone, Copy)]
struct Face {
	block: u32,
	direction: Direction,
	position: Location,
}

/// Returns a list of blocks and their external faces
fn compute_external_block_faces(blocks: &[Block]) -> Vec<Face> {
	let cube_sides = [
		Direction::Left,
		Direction::Right,
		Direction::Down,
		Direction::Up,
		Direction::Forward,
		Direction::Backward,
	];
	
	let mut sides = HashMap::with_capacity(8192 * 6);
	
	for block in blocks {
		let pos = block.position;
	
		for side in &cube_sides {
			let pos = (pos.0 * 2, pos.1 * 2, pos.2 * 2);
	
			let block = match block.block {
				Blocks::Grass => match side {
					Direction::Up | Direction::Down => GRASS_TOP_FACE,
					_ => GRASS_SIDE_FACE,
				},
				Blocks::Stone => STONE_FACE,
				Blocks::Dirt => DIRT_FACE,
				_ => unreachable!(),
			};

			let face = Face {
				block,
				direction: side.clone(),
				position: pos,
			};

			let side = side.to_normal();

			let face_position = (pos.0 + side.0, pos.1 + side.1, pos.2 + side.2);
	
			// If cube side already exists, then this wall is internal
			sides
				.entry(face_position)
				.and_modify(|(_, external): &mut (_, bool)| *external = false)
				.or_insert((face, true));
		}
	}
	
	let mut external_sides = sides
		.values()
		.filter(|(_, external)| *external)
		.map(|(k, _)| *k)
		.collect::<Vec<_>>();

	// Place Up faces first
	external_sides.sort_by(|a, b| {
		let a = a.direction;
		let b = b.direction;

		if a == Direction::Up {
			std::cmp::Ordering::Less
		} else if b == Direction::Up {
			std::cmp::Ordering::Greater
		} else {
			std::cmp::Ordering::Equal
		}
	});

	external_sides
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FaceData {
	block: u16,
	// 1: -x, 2: +x, 3: -y, 4: +y, 5: -z, 6: +z
	direction: u8,
	position: (f32, f32, f32),
}

/// Returns a list of per instance data to transform the squares
fn build_cube_faces(blocks: &[Block]) -> Vec<FaceData> {
	let external_sides = compute_external_block_faces(blocks);

	external_sides.into_iter().map(|face| {
		let direction = face.direction as u8;

		let position = (
			face.position.0 as f32 * 0.5,
			face.position.1 as f32 * 0.5,
			face.position.2 as f32 * 0.5,
		);

		FaceData {
			block: face.block as u16,
			direction,
			position,
		}
	}).collect()
}

#[cfg(test)]
mod tests {
    use crate::{compute_external_block_faces, Block, Direction, Blocks, GRASS_TOP_FACE};

	#[test]
	fn test_faces_single_block() {
		let block = Block::new((0, 0, 0), Blocks::Grass);
		let blocks = vec![block];

		let faces = compute_external_block_faces(&blocks);

		assert_eq!(faces.len(), 6);

		assert_eq!(faces[0].block, GRASS_TOP_FACE);
		assert_eq!(faces[0].direction, Direction::Up);
		assert_eq!(faces[0].position, (0, 0, 0));
	}

	#[test]
	fn test_faces_two_blocks() {
		let blocks = vec![Block::new((0, 0, 0), Blocks::Grass), Block::new((0, 0, 1), Blocks::Grass)];

		let faces = compute_external_block_faces(&blocks);

		assert_eq!(faces.len(), 10);

		assert_eq!(faces[0].block, GRASS_TOP_FACE);
		assert_eq!(faces[0].direction, Direction::Up);
		assert_eq!(faces[0].position, (0, 0, 0));
		assert_eq!(faces[1].block, GRASS_TOP_FACE);
		assert_eq!(faces[1].direction, Direction::Up);
		assert_eq!(faces[1].position, (0, 0, 2));
	}
}
