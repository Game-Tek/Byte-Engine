use std::{
    io::Write,
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::Arc,
};

use ghi::{BoundComputePipelineMode, BoundRasterizationPipelineMode, CommandBufferRecordable, GraphicsHardwareInterface, RasterizationRenderPassMode};
use resource_management::resource::resource_manager::ResourceManager;
use utils::{sync::RwLock, Extent, RGBA};

use crate::{
    core::{
        self, entity::{DomainType, EntityBuilder}, listener::{EntitySubscriber, Listener}, orchestrator, spawn, spawn_as_child, Entity, EntityHandle
    }, gameplay::space::{Space, Spawn}, ui::render_model::UIRenderModel, utils, window_system::{self, WindowSystem}, Vector3
};

use super::{render_pass::RenderPass, texture_manager::TextureManager, triangle::Triangle};

pub struct Renderer {
    ghi: Rc<RwLock<ghi::GHI>>,

    rendered_frame_count: usize,
    frame_queue_depth: usize,

    swapchain_handles: Vec<ghi::SwapchainHandle>,

    render_command_buffer: ghi::CommandBufferHandle,
    render_finished_synchronizer: ghi::SynchronizerHandle,

    result: ghi::ImageHandle,
    accumulation_map: ghi::ImageHandle,

    window_system: EntityHandle<window_system::WindowSystem>,

    root_render_pass: RootRenderPass,

    extent: Extent,
}

impl Renderer {
    pub fn new_as_system<'a>(
        window_system_handle: EntityHandle<WindowSystem>,
        resource_manager_handle: EntityHandle<ResourceManager>,
    ) -> EntityBuilder<'a, Self> {
        EntityBuilder::new_from_closure_with_parent(move |parent: DomainType| {
            let enable_validation = std::env::vars()
                .find(|(k, _)| k == "BE_RENDER_DEBUG")
                .is_some()
                || true;

            let ghi_instance = Rc::new(RwLock::new(ghi::create(
                ghi::Features::new()
                    .validation(enable_validation)
                    .api_dump(false)
                    .gpu_validation(false)
                    .debug_log_function(|message| {
                        log::error!("{}", message);
                    })
					// .gpu("llvmpipe (LLVM 15.0.7, 256 bits)")
					// .gpu("AMD Radeon Graphics (RADV RENOIR)")
            )));

            let extent = Extent::square(0); // Initialize extent to 0 to allocate memory lazily.

            let result;
            let accumulation_map;
            let depth_sampler;
			let render_command_buffer;
			let render_finished_synchronizer;

            {
                let mut ghi = ghi_instance.write();

                result = ghi.create_image(
                    Some("result"),
                    extent,
                    ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized),
                    ghi::Uses::Storage | ghi::Uses::TransferDestination,
                    ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead,
                    ghi::UseCases::DYNAMIC,
                    1,
                );
                accumulation_map = ghi.create_image(
                    Some("accumulate_map"),
                    extent,
                    ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized),
                    ghi::Uses::Storage | ghi::Uses::TransferSource | ghi::Uses::BlitDestination | ghi::Uses::RenderTarget,
                    ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead,
                    ghi::UseCases::DYNAMIC,
                    1,
                );
                depth_sampler = ghi.build_sampler(
                    ghi::sampler::Builder::new()
                        .addressing_mode(ghi::SamplerAddressingModes::Border {})
                        .reduction_mode(ghi::SamplingReductionModes::Min)
                        .filtering_mode(ghi::FilteringModes::Closest),
                );

				render_command_buffer = ghi.create_command_buffer(Some("Render"));
				render_finished_synchronizer = ghi.create_synchronizer(Some("Render Finisished"), true);
            };

            let texture_manager = Arc::new(RwLock::new(TextureManager::new()));

			let root_render_pass = RootRenderPass::new();

            Renderer {
                ghi: ghi_instance,

                rendered_frame_count: 0,
                frame_queue_depth: 2,

                swapchain_handles: vec![],

                render_command_buffer,
                render_finished_synchronizer,

                result,
                accumulation_map,

                window_system: window_system_handle,

                root_render_pass,

                extent,
            }
        })
        .listen_to::<window_system::Window>()
    }

	pub fn add_render_pass<T: RenderPass + Entity + 'static>(&mut self, space_handle: EntityHandle<Space>) {
		let render_pass = space_handle.spawn(T::create());
		self.root_render_pass.add_render_pass(render_pass);
	}

    pub fn render(&mut self) {
        if self.swapchain_handles.is_empty() {
            return;
        }

        let mut ghi = self.ghi.write();

        let swapchain_handle = self.swapchain_handles[0];

		let frame_key = ghi.start_frame(self.rendered_frame_count as u32);

        ghi.wait(frame_key, self.render_finished_synchronizer);

        ghi.start_frame_capture();

        let (present_key, extent) =
            ghi.acquire_swapchain_image(frame_key, swapchain_handle,);

        assert!(extent.width() <= 65535 && extent.height() <= 65535, "The extent is too large: {:?}. The renderer only supports dimensions as big as 16 bits.", extent);

        drop(ghi);

        if extent != self.extent {
            let mut ghi = self.ghi.write();

            ghi.resize_image(self.result, extent);
            ghi.resize_image(self.accumulation_map, extent);

            self.root_render_pass.resize(&mut ghi, extent);

            self.extent = extent;
        }

        let mut ghi = self.ghi.write();

        self.root_render_pass.prepare(&mut ghi, extent);

        let mut command_buffer_recording = ghi.create_command_buffer_recording(
            self.render_command_buffer,
            frame_key.into(),
        );

        self.root_render_pass
            .record(&mut command_buffer_recording, extent);

        command_buffer_recording.copy_to_swapchain(self.result, present_key, swapchain_handle);

        command_buffer_recording.execute(
            &[],
            &[self.render_finished_synchronizer],
            &[present_key],
            self.render_finished_synchronizer,
        );

        ghi.end_frame_capture();

        self.rendered_frame_count += 1;
    }
}

impl EntitySubscriber<window_system::Window> for Renderer {
    fn on_create<'a>(
        &'a mut self,
        handle: EntityHandle<window_system::Window>,
        window: &window_system::Window,
    ) -> () {
        let os_handles = self.window_system.map(|e| {
            let e = e.read();
            e.get_os_handles(&handle)
        });

        let mut ghi = self.ghi.write();

        let swapchain_handle = ghi.bind_to_window(
            &os_handles,
            ghi::PresentationModes::FIFO,
            Extent::rectangle(1920, 1080),
        );

        self.swapchain_handles.push(swapchain_handle);
    }
}

impl Entity for Renderer {}

struct RootRenderPass {
    render_passes: Vec<EntityHandle<dyn RenderPass>>,
}

impl RootRenderPass {
    pub fn new() -> Self {
        Self {
            render_passes: vec![],
        }
    }
}

impl RenderPass for RootRenderPass {
	fn create() -> EntityBuilder<'static, Self> where Self: Sized {
		Self::new().into()
	}

    fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
        self.render_passes.push(render_pass);
    }

    fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {
        for render_pass in &self.render_passes {
            render_pass.get_mut(|e| {
                e.prepare(ghi, extent);
            });
        }
    }

    fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent) {
        for render_pass in &self.render_passes {
            render_pass.get_mut(|e| {
                e.record(command_buffer_recording, extent);
            });
        }
    }

    fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {
        for render_pass in &self.render_passes {
            render_pass.get_mut(|e| {
                e.resize(ghi, extent);
            });
        }
    }
}

struct ScreenRenderPass {
	layout: ghi::PipelineLayoutHandle,
    pipeline: ghi::PipelineHandle,
    base: ghi::ImageHandle,
    mesh: ghi::MeshHandle,
    triangles: Vec<EntityHandle<Triangle>>,
}

impl ScreenRenderPass {
    pub fn new(ghi: &mut ghi::GHI, base: ghi::ImageHandle) -> EntityBuilder<'_, Self> {
        let vshader = ghi.create_shader(
            None,
            ghi::ShaderSource::GLSL(r#"#version 450 core
#pragma shader_stage(vertex)
layout(location = 0) in vec3 position;
void main() {
    gl_Position = vec4(position, 1.0);
}"#.to_owned()),
            ghi::ShaderTypes::Vertex,
            &[],
        ).unwrap();

        let fshader = ghi.create_shader(
            None,
            ghi::ShaderSource::GLSL(r#"#version 450 core
#pragma shader_stage(fragment)
layout(location = 0) out vec4 color;
void main() {
    color = vec4(1.0, 1.0, 1.0, 1.0);
}"#.to_owned()),
            ghi::ShaderTypes::Fragment,
            &[],
        ).unwrap();

        let layout = ghi.create_pipeline_layout(&[], &[]);

        let pipeline = ghi.create_raster_pipeline(&[
            ghi::PipelineConfigurationBlocks::Shaders {
                shaders: &[ghi::ShaderParameter::new(&vshader, ghi::ShaderTypes::Vertex), ghi::ShaderParameter::new(&fshader, ghi::ShaderTypes::Fragment)],
            },
            ghi::PipelineConfigurationBlocks::Layout { layout: &layout },
            ghi::PipelineConfigurationBlocks::InputAssembly {  },
            ghi::PipelineConfigurationBlocks::VertexInput {
                vertex_elements: &[
                    ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
                ],
            },
            ghi::PipelineConfigurationBlocks::RenderTargets {
                targets: &[ghi::PipelineAttachmentInformation::new(
                    ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Layouts::RenderTarget, ghi::ClearValue::None, true, true,
                )],
            },
        ]);

        let triangle_vertices = [
        	Vector3::new(0.0, -1.0, 0.0),
        	Vector3::new(1.0, 1.0, 0.0),
        	Vector3::new(-1.0,1.0, 0.0)
        ];

        let triangle_indices: [u16; 3] = [0, 1, 2];

        let mesh = ghi.add_mesh_from_vertices_and_indices(3, 3, unsafe { std::slice::from_raw_parts(triangle_vertices.as_ptr() as _, triangle_vertices.len() * std::mem::size_of::<Vector3>()) }, unsafe {
        	std::slice::from_raw_parts(triangle_indices.as_ptr() as _, triangle_indices.len() * std::mem::size_of::<u16>())
        }, &[ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0)]);

        EntityBuilder::new(Self {
			layout,
            pipeline,
            base,
            mesh,
            triangles: Vec::new(),
        }).listen_to::<Triangle>()
    }
}

impl Entity for ScreenRenderPass {}

impl EntitySubscriber<Triangle> for ScreenRenderPass {
    fn on_create<'a>(
        &'a mut self,
        handle: EntityHandle<Triangle>,
        params: &'a Triangle,
    ) -> () {
        self.triangles.push(handle);
    }
}

impl RenderPass for ScreenRenderPass {
	fn create() -> EntityBuilder<'static, Self> where Self: Sized {
		unimplemented!()
	}

    fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer_recording.bind_descriptor_sets(&self.layout, &[]);

		command_buffer_recording.start_render_pass(extent, &[ghi::AttachmentInformation::new(self.base, ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Layouts::RenderTarget, ghi::ClearValue::None, false, true)]);

    	let command_buffer_recording = command_buffer_recording.bind_raster_pipeline(&self.pipeline);

        for triangle in &self.triangles {
            command_buffer_recording.draw_mesh(&self.mesh);
        }

		command_buffer_recording.end_render_pass();
    }

    fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {

    }

    fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {

    }

    fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {

    }
}

struct RenderPassDriver {
    render_pass: EntityHandle<dyn RenderPass>,
}

impl RenderPassDriver {
    fn new(render_pass: EntityHandle<dyn RenderPass>) -> Self {
        Self { render_pass }
    }
}
