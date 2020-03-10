#include "Renderer.h"
#include "RAPI/RenderDevice.h"

#include "Application/Application.h"
#include "Math/GSM.hpp"

#include "Resources/StaticMeshResource.h"

#include "Material.h"
#include "Game/StaticMesh.h"
#include "Resources/TextureResource.h"

#include "Game/Texture.h"

#include "ScreenQuad.h"
#include "StaticMeshRenderableManager.h"

using namespace RAPI;

Renderer::Renderer() : Framebuffers(3), perViewData(1, 1), perInstanceData(1), perInstanceTransform(1)
{
	RAPI::RenderDevice::RenderDeviceCreateInfo render_device_create_info;
	render_device_create_info.RenderingAPI = RenderAPI::VULKAN;
	render_device_create_info.ApplicationName = GS::Application::Get()->GetApplicationName();
	FVector<Queue::QueueCreateInfo> queue_create_infos = { { Queue::QueueCapabilities::GRAPHICS, 1.0f, &graphicsQueue }, { Queue::QueueCapabilities::TRANSFER, 1.0f, &transferQueue } };
	render_device_create_info.QueueCreateInfos = &queue_create_infos;
	renderDevice = RAPI::RenderDevice::CreateRenderDevice(render_device_create_info);
	
	Win = GS::Application::Get()->GetActiveWindow();

	RenderContextCreateInfo RCCI;
	RCCI.Window = Win;
	RC = renderDevice->CreateRenderContext(RCCI);
	auto SCImages = RC->GetSwapchainImages();

	CommandBuffer::CommandBufferCreateInfo command_buffers_create_info;
	graphicsCommandBuffer = renderDevice->CreateCommandBuffer(command_buffers_create_info);
	transferCommandBuffer = renderDevice->CreateCommandBuffer(command_buffers_create_info);

	RenderTarget::RenderTargetCreateInfo CACI;
	CACI.Extent = Extent3D{Win->GetWindowExtent().Width, Win->GetWindowExtent().Height, 1};
	CACI.Dimensions = ImageDimensions::IMAGE_2D;
	CACI.Use = ImageUse::DEPTH_STENCIL_ATTACHMENT;
	CACI.Type = ImageType::DEPTH_STENCIL;
	CACI.Format = ImageFormat::DEPTH24_STENCIL8;
	depthTexture = renderDevice->CreateRenderTarget(CACI);


	RenderPassCreateInfo RPCI;
	RenderPassDescriptor RPD;
	AttachmentDescriptor SIAD;

	SIAD.AttachmentImage = SCImages[0]; //Only first because it gets only properties, doesn't access actual data.
	SIAD.InitialLayout = ImageLayout::UNDEFINED;
	SIAD.FinalLayout = ImageLayout::PRESENTATION;
	SIAD.StoreOperation = RenderTargetStoreOperations::STORE;
	SIAD.LoadOperation = RenderTargetLoadOperations::CLEAR;


	RPD.RenderPassColorAttachments.push_back(&SIAD);

	AttachmentDescriptor depth_attachment;
	depth_attachment.AttachmentImage = depthTexture;
	depth_attachment.InitialLayout = ImageLayout::UNDEFINED;
	depth_attachment.FinalLayout = ImageLayout::DEPTH_STENCIL_ATTACHMENT;
	depth_attachment.LoadOperation = RenderTargetLoadOperations::CLEAR;
	depth_attachment.StoreOperation = RenderTargetStoreOperations::UNDEFINED;

	RPD.DepthStencilAttachment = depth_attachment;


	SubPassDescriptor SPD;
	AttachmentReference SubPassWriteAttachmentReference;
	SubPassWriteAttachmentReference.Layout = ImageLayout::COLOR_ATTACHMENT;
	SubPassWriteAttachmentReference.Index = 0;

	SPD.WriteColorAttachments.push_back(&SubPassWriteAttachmentReference);


	AttachmentReference SubPassReadAttachmentReference;
	SubPassReadAttachmentReference.Layout = ImageLayout::UNDEFINED;
	SubPassReadAttachmentReference.Index = ATTACHMENT_UNUSED;
	SPD.ReadColorAttachments.push_back(&SubPassReadAttachmentReference);

	AttachmentReference depth_reference;
	depth_reference.Layout = ImageLayout::DEPTH_STENCIL_ATTACHMENT;
	depth_reference.Index = 1;
	SPD.DepthAttachmentReference = &depth_reference;

	RPD.SubPasses.push_back(&SPD);
	RPCI.Descriptor = RPD;
	RP = renderDevice->CreateRenderPass(RPCI);

	PushConstant MyPushConstant;
	MyPushConstant.Size = sizeof(uint32);

	Framebuffers.resize(SCImages.getLength());
	for (uint8 i = 0; i < SCImages.getLength(); ++i)
	{
		FramebufferCreateInfo FBCI;
		FBCI.RenderPass = RP;
		FBCI.Extent = Win->GetWindowExtent();
		FBCI.Images = DArray<RenderTarget*>() = { SCImages[i], depthTexture };
		FBCI.ClearValues = {{0, 0, 0, 0}, {1, 0, 0, 0}};
		Framebuffers[i] = renderDevice->CreateFramebuffer(FBCI);
	}

	RAPI::RenderMesh::RenderMeshCreateInfo MCI;
	MCI.IndexCount = ScreenQuad::IndexCount;
	MCI.VertexCount = ScreenQuad::VertexCount;
	MCI.VertexData = ScreenQuad::Vertices;
	MCI.IndexData = ScreenQuad::Indices;
	MCI.VertexLayout = &ScreenQuad::VD;
	FullScreenQuad = renderDevice->CreateRenderMesh(MCI);

	GraphicsPipelineCreateInfo gpci;
	gpci.RenderDevice = renderDevice;
	gpci.RenderPass = RP;
	gpci.VDescriptor = &ScreenQuad::VD;
	gpci.PipelineDescriptor.BlendEnable = false;
	gpci.ActiveWindow = Win;

	FString VS(
		"#version 450\nlayout(push_constant) uniform Push {\nmat4 Mat;\n} inPush;\nlayout(binding = 0, row_major) uniform Data {\nmat4 Pos;\n} inData;\nlayout(location = 0)in vec3 inPos;\nlayout(location = 1)in vec3 inTexCoords;\nlayout(location = 0)out vec4 tPos;\nvoid main()\n{\ngl_Position = inData.Pos * vec4(inPos, 1.0);\n}");
	gpci.PipelineDescriptor.Stages.push_back(ShaderInfo{ShaderType::VERTEX_SHADER, &VS});

	FString FS(
		"#version 450\nlayout(location = 0)in vec4 tPos;\nlayout(binding = 1) uniform sampler2D texSampler;\nlayout(location = 0) out vec4 outColor;\nvoid main()\n{\noutColor = vec4(1, 1, 1, 1);\n}");
	gpci.PipelineDescriptor.Stages.push_back(ShaderInfo{ShaderType::FRAGMENT_SHADER, &FS});

	FullScreenRenderingPipeline = renderDevice->CreateGraphicsPipeline(gpci);
}

Renderer::~Renderer()
{
	for (auto& Element : ComponentToInstructionsMap)
	{
		delete Element.second;
	}

	for (auto const& x : Pipelines)
	{
		delete x.second;
	}

	delete RP;
	delete RC;

	RenderDevice::DestroyRenderDevice(renderDevice);
}

void Renderer::OnUpdate()
{
	/*Update debug vars*/
	GS_DEBUG_ONLY(DrawCalls = 0)
	GS_DEBUG_ONLY(InstanceDraws = 0)
	GS_DEBUG_ONLY(PipelineSwitches = 0)
	GS_DEBUG_ONLY(DrawnComponents = 0)
	/*Update debug vars*/

	UpdateViews();

	UpdateRenderables();

	CommandBuffer::BeginRecordingInfo begin_recording_info;
	graphicsCommandBuffer->BeginRecording(begin_recording_info);

	CommandBuffer::BeginRenderPassInfo RPBI;
	RPBI.RenderPass = RP;
	RPBI.Framebuffer = Framebuffers[RC->GetCurrentImage()];
	graphicsCommandBuffer->BeginRenderPass(RPBI);

	RenderRenderables();

	CommandBuffer::EndRenderPassInfo end_render_pass_info;
	graphicsCommandBuffer->EndRenderPass(end_render_pass_info);

	CommandBuffer::EndRecordingInfo end_recording_info;
	graphicsCommandBuffer->EndRecording(end_recording_info);

	RAPI::RenderContext::AcquireNextImageInfo acquire_info;
	acquire_info.RenderDevice = renderDevice;
	RC->AcquireNextImage(acquire_info);

	//RenderContext::FlushInfo flush_info;
	//flush_info.RenderDevice = renderDevice;
	//RC->Flush(flush_info);

	Queue::DispatchInfo dispatch_info;
	dispatch_info.RenderDevice = renderDevice;
	dispatch_info.CommandBuffer = graphicsCommandBuffer;
	graphicsQueue->Dispatch(dispatch_info);

	RenderContext::PresentInfo present_info;
	present_info.RenderDevice = renderDevice;
	RC->Present(present_info);
}

void Renderer::DrawMeshes(const RAPI::CommandBuffer::DrawIndexedInfo& drawInfo, RAPI::RenderMesh* Mesh_)
{
	CommandBuffer::BindMeshInfo bind_mesh_info;
	bind_mesh_info.Mesh = Mesh_;
	graphicsCommandBuffer->BindMesh(bind_mesh_info);

	CommandBuffer::DrawIndexedInfo draw_indexed_info;
	draw_indexed_info.IndexCount = drawInfo.IndexCount;
	draw_indexed_info.InstanceCount = drawInfo.InstanceCount;
	graphicsCommandBuffer->DrawIndexed(draw_indexed_info);
	
	GS_DEBUG_ONLY(++DrawCalls)
	GS_DEBUG_ONLY(InstanceDraws += draw_indexed_info.InstanceCount)
}

void Renderer::BindPipeline(GraphicsPipeline* _Pipeline)
{
	CommandBuffer::BindGraphicsPipelineInfo bind_graphics_pipeline_info;
	bind_graphics_pipeline_info.GraphicsPipeline = _Pipeline;
	bind_graphics_pipeline_info.RenderExtent = Win->GetWindowExtent();
	
	graphicsCommandBuffer->BindGraphicsPipeline(bind_graphics_pipeline_info);
	GS_DEBUG_ONLY(++PipelineSwitches)
}

RAPI::RenderMesh* Renderer::CreateMesh(StaticMesh* _SM)
{
	RAPI::RenderMesh* NewMesh = nullptr;

	if (Meshes.find(_SM) == Meshes.end())
	{
		Model m = _SM->GetModel();

		RAPI::RenderMesh::RenderMeshCreateInfo MCI;
		MCI.IndexCount = m.IndexCount;
		MCI.VertexCount = m.VertexCount;
		MCI.VertexData = m.VertexArray;
		MCI.IndexData = m.IndexArray;
		MCI.VertexLayout = StaticMeshResource::GetVertexDescriptor();
		Meshes[_SM] = renderDevice->CreateRenderMesh(MCI);
	}
	else
	{
		NewMesh = Meshes[_SM];
	}


	return NewMesh;
}

GraphicsPipeline* Renderer::CreatePipelineFromMaterial(Material* _Mat) const
{
	GraphicsPipelineCreateInfo GPCI;

	GPCI.VDescriptor = StaticMeshResource::GetVertexDescriptor();

	FVector<ShaderInfo> si;
	_Mat->GetRenderingCode(si);

	for (auto& e : si)
	{
		GPCI.PipelineDescriptor.Stages.push_back(e);
	}

	GPCI.PipelineDescriptor.BlendEnable = _Mat->GetHasTransparency();
	GPCI.PipelineDescriptor.ColorBlendOperation = BlendOperation::ADD;
	GPCI.PipelineDescriptor.CullMode = _Mat->GetIsTwoSided() ? CullMode::CULL_NONE : CullMode::CULL_BACK;
	GPCI.PipelineDescriptor.DepthCompareOperation = CompareOperation::LESS;

	GPCI.RenderPass = RP;
	GPCI.ActiveWindow = Win;

	return renderDevice->CreateGraphicsPipeline(GPCI);
}

void Renderer::UpdateViews()
{
	for (auto& view : perViewData)
	{
		//We get and store the camera's position so as to not access it several times.
		const Vector3 CamPos = GetActiveCamera()->GetPosition();

		view.ViewMatrix.MakeIdentity(); //Reset view matrix or it will accumulate operation over time(BAD).

		Transform3 camera_transform;
		camera_transform.Position.X = -CamPos.X;
		camera_transform.Position.Y = -CamPos.Y;
		camera_transform.Position.Z = CamPos.Z;
		camera_transform.Rotation = GetActiveCamera()->GetTransform().Rotation;

		auto t = GetActiveCamera()->GetTransform().Rotation;

		//GSM::Rotate(view.ViewMatrix, t);

		GSM::Translate(view.ViewMatrix, camera_transform.Position);

		auto& nfp = GetActiveCamera()->GetNearFarPair();

		GSM::BuildPerspectiveMatrix(view.ProjectionMatrix, GetActiveCamera()->GetFOV(), Win->GetAspectRatio(), nfp.First, nfp.Second);

		view.ViewProjectionMatrix = view.ProjectionMatrix * view.ViewMatrix;
	}
}

void Renderer::RegisterRenderComponent(RenderComponent* _RC, RenderComponentCreateInfo* _RCCI)
{
	for(auto& manager : renderableTypeManagers)
	{
		if(manager->GetRenderableTypeName() == _RC->GetRenderableType())
		{
			manager->RegisterComponent(this, _RC);
		}
	}
}

void Renderer::UpdateRenderables()
{
	//for (auto& e : ComponentToInstructionsMap)
	//{
	//	auto ri = e.second->GetRenderableInstructions();
	//
	//	BindTypeResourcesInfo btpi{ this };
	//	ri->BindTypeResources(btpi);
	//}

	uint32 i = 0;

	for (auto& e : ComponentToInstructionsMap)
	{
		GSM::Translate(perInstanceTransform[i], e.second->GetOwner()->GetPosition());
		perInstanceTransform[i] = perViewData[0].ViewProjectionMatrix * perInstanceTransform[i];

		++i;
	}

	//UL->UpdateBindingSet()
}

void Renderer::RenderRenderables()
{
	//BindPipeline(Pipelines.begin()->second);

	uint32 i = 0;

	for (auto& e : ComponentToInstructionsMap)
	{
		++i;
	}
}