#include "Renderer.h"
#include "RAPI/RenderDevice.h"

#include "Application/Application.h"
#include "Math/GSM.hpp"

#include "Resources/StaticMeshResource.h"

#include "Material.h"
#include "Game/StaticMesh.h"
#include "MeshRenderResource.h"
#include "MaterialRenderResource.h"
#include "Resources/TextureResource.h"

#include "Game/Texture.h"

#include "ScreenQuad.h"
#include "StaticMeshRenderableManager.h"

using namespace RAPI;

Renderer::Renderer() : Framebuffers(3), perViewData(1, 1), perInstanceData(1), perInstanceTransform(1)
{
	renderDevice = RenderDevice::CreateRenderDevice(RenderAPI::VULKAN);
	
	Win = GS::Application::Get()->GetActiveWindow();

	RenderContextCreateInfo RCCI;
	RCCI.Window = Win;
	RC = renderDevice->CreateRenderContext(RCCI);

	auto SCImages = RC->GetSwapchainImages();

	RenderTargetCreateInfo CACI;
	CACI.Extent = Extent3D{Win->GetWindowExtent().Width, Win->GetWindowExtent().Height, 1};
	CACI.Dimensions = ImageDimensions::IMAGE_2D;
	CACI.Use = ImageUse::DEPTH_STENCIL_ATTACHMENT;
	CACI.Type = ImageType::DEPTH_STENCIL;
	CACI.ImageFormat = Format::DEPTH24_STENCIL8;
	depthTexture = renderDevice->CreateRenderTarget(CACI);


	RenderPassCreateInfo RPCI;
	RenderPassDescriptor RPD;
	AttachmentDescriptor SIAD;

	SIAD.AttachmentImage = SCImages[0]; //Only first because it gets only properties, doesn't access actual data.
	SIAD.InitialLayout = ImageLayout::UNDEFINED;
	SIAD.FinalLayout = ImageLayout::PRESENTATION;
	SIAD.StoreOperation = StoreOperations::STORE;
	SIAD.LoadOperation = LoadOperations::CLEAR;


	RPD.RenderPassColorAttachments.push_back(&SIAD);

	AttachmentDescriptor depth_attachment;
	depth_attachment.AttachmentImage = depthTexture;
	depth_attachment.InitialLayout = ImageLayout::UNDEFINED;
	depth_attachment.FinalLayout = ImageLayout::DEPTH_STENCIL_ATTACHMENT;
	depth_attachment.LoadOperation = LoadOperations::CLEAR;
	depth_attachment.StoreOperation = StoreOperations::UNDEFINED;

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


	BindingLayoutCreateInfo ULCI;
	ULCI.DescriptorCount = 3;
	ULCI.BindingsSetLayout[0].BindingType = UniformType::UNIFORM_BUFFER;
	ULCI.BindingsSetLayout[0].ShaderStage = ShaderType::VERTEX_SHADER;
	ULCI.BindingsSetLayout[0].ArrayLength = 1;
	RAPI::BindingDescriptor uniform_set;
	uniform_set.ShaderStage = ShaderType::FRAGMENT_SHADER;
	uniform_set.BindingType = UniformType::COMBINED_IMAGE_SAMPLER;
	uniform_set.ArrayLength = 1;
	ULCI.BindingsSetLayout[1] = uniform_set;
	ULCI.BindingsSetLayout.resize(2);

	PushConstant MyPushConstant;
	MyPushConstant.Size = sizeof(uint32);

	UniformBufferCreateInfo UBCI;
	UBCI.Size = sizeof(Matrix4);
	UB = renderDevice->CreateUniformBuffer(UBCI);

	Framebuffers.resize(SCImages.getLength());
	for (uint8 i = 0; i < SCImages.getLength(); ++i)
	{
		FramebufferCreateInfo FBCI;
		FBCI.RenderPass = RP;
		FBCI.Extent = Win->GetWindowExtent();
		FBCI.Images = DArray<RenderTarget*>() = {SCImages[i], depthTexture};
		FBCI.ClearValues = {{0, 0, 0, 0}, {1, 0, 0, 0}};
		Framebuffers[i] = renderDevice->CreateFramebuffer(FBCI);
	}

	MeshCreateInfo MCI;
	MCI.IndexCount = ScreenQuad::IndexCount;
	MCI.VertexCount = ScreenQuad::VertexCount;
	MCI.VertexData = ScreenQuad::Vertices;
	MCI.IndexData = ScreenQuad::Indices;
	MCI.VertexLayout = &ScreenQuad::VD;
	FullScreenQuad = renderDevice->CreateMesh(MCI);

	GraphicsPipelineCreateInfo gpci;
	gpci.RenderDevice = renderDevice;
	gpci.RenderPass = RP;
	gpci.UniformLayout = UL;
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
	CB->BeginRecording(begin_recording_info);

	CommandBuffer::BeginRenderPassInfo RPBI;
	RPBI.RenderPass = RP;
	RPBI.Framebuffer = Framebuffers[RC->GetCurrentImage()];
	CB->BeginRenderPass(RPBI);

	RenderRenderables();

	CommandBuffer::EndRenderPassInfo end_render_pass_info;
	CB->EndRenderPass(end_render_pass_info);

	CommandBuffer::EndRecordingInfo end_recording_info;
	CB->EndRecording(end_recording_info);

	CB->AcquireNextImage();

	CB->Flush();
	CB->Present();
}

void Renderer::DrawMesh(const DrawInfo& _DrawInfo, MeshRenderResource* Mesh_)
{
	CommandBuffer::BindMeshInfo bind_mesh_info;
	bind_mesh_info.Mesh = Mesh_->mesh;
	CB->BindMesh(bind_mesh_info);

	CommandBuffer::DrawIndexedInfo draw_indexed_info;
	draw_indexed_info.IndexCount;
	draw_indexed_info.InstanceCount = 1;
	CB->DrawIndexed(draw_indexed_info);
	
	GS_DEBUG_ONLY(++DrawCalls)
	GS_DEBUG_ONLY(InstanceDraws += 1)
}

void Renderer::BindPipeline(GraphicsPipeline* _Pipeline)
{
	CommandBuffer::BindGraphicsPipelineInfo bind_graphics_pipeline_info;
	bind_graphics_pipeline_info.GraphicsPipeline = _Pipeline;
	bind_graphics_pipeline_info.RenderExtent = Win->GetWindowExtent();
	
	CB->BindGraphicsPipeline(bind_graphics_pipeline_info);
	GS_DEBUG_ONLY(++PipelineSwitches)
}

RenderMesh* Renderer::CreateMesh(StaticMesh* _SM)
{
	MeshRenderResource* NewMesh = nullptr;

	if (Meshes.find(_SM) == Meshes.end())
	{
		Model m = _SM->GetModel();

		MeshCreateInfo MCI;
		MCI.IndexCount = m.IndexCount;
		MCI.VertexCount = m.VertexCount;
		MCI.VertexData = m.VertexArray;
		MCI.IndexData = m.IndexArray;
		MCI.VertexLayout = StaticMeshResource::GetVertexDescriptor();
		Meshes[_SM] = renderDevice->CreateMesh(MCI);
	}
	else
	{
		NewMesh = Meshes[_SM];
	}


	return NewMesh;
}

MaterialRenderResource* Renderer::CreateMaterial(Material* Material_)
{
	auto Res = Pipelines.find(Id(Material_->GetMaterialType()));

	if (Res == Pipelines.end())
	{
		auto NP = CreatePipelineFromMaterial(Material_);
		Pipelines.insert({Id(Material_->GetMaterialType()).GetID(), NP});
	}

	MaterialRenderResourceCreateInfo material_render_resource_create_info;
	material_render_resource_create_info.ParentMaterial = Material_;

	for (uint8 i = 0; i < Material_->GetTextures().getLength(); ++i)
	{
		auto texture_resource = Material_->GetTextures()[i]->GetTextureResource();
		
		TextureCreateInfo texture_create_info;
		texture_create_info.ImageData = texture_resource->GetTextureData().ImageData;
		texture_create_info.ImageDataSize = texture_resource->GetTextureData().imageDataSize;
		texture_create_info.Extent = texture_resource->GetTextureData().TextureDimensions;
		texture_create_info.ImageFormat = texture_resource->GetTextureData().TextureFormat;
		texture_create_info.Layout = ImageLayout::SHADER_READ;
	
		auto texture = renderDevice->CreateTexture(texture_create_info);
	
		material_render_resource_create_info.textures.push_back(texture);
	}

	return new MaterialRenderResource(material_render_resource_create_info);
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
	GPCI.UniformLayout = UL;
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

		BuildPerspectiveMatrix(view.ProjectionMatrix, GetActiveCamera()->GetFOV(), Win->GetAspectRatio(), nfp.First,
		                       nfp.Second);

		view.ViewProjectionMatrix = view.ProjectionMatrix * view.ViewMatrix;
	}
}

void Renderer::RegisterRenderComponent(RenderComponent* _RC, RenderComponentCreateInfo* _RCCI)
{
	FString name(64);
	
	for(auto& manager : renderableTypeManagers)
	{
		manager->GetRenderableTypeName(name);

		if(name == _RC->GetRenderableType())
		{
			manager->RegisterComponent(_RC);
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

void Renderer::BuildPerspectiveMatrix(Matrix4& matrix, const float fov, const float aspectRatio, const float near, const float far)
{
	//Tangent of half the vertical view angle.
	const auto f = 1 / GSM::Tangent(fov * 0.5f);

	const auto far_m_near = far - near;

	//Zero to one
	//Left handed

	matrix(0, 0) = f / aspectRatio;

	matrix(1, 1) = -f;

	matrix(2, 2) = -((far + near) / far_m_near);
	matrix(2, 3) = -((2.f * far * near) / far_m_near);

	matrix(3, 2) = -1.0f;
}

void Renderer::MakeOrthoMatrix(Matrix4& matrix, const float right, const float left, const float top, const float bottom, const float near, const float far)
{
	//Zero to one
	//Left handed

	matrix(0, 0) = static_cast<float>(2) / (right - left);
	matrix(1, 1) = static_cast<float>(2) / (top - bottom);
	matrix(2, 2) = static_cast<float>(1) / (far - near);
	matrix(3, 0) = -(right + left) / (right - left);
	matrix(3, 1) = -(top + bottom) / (top - bottom);
	matrix(3, 2) = -near / (far - near);
}
