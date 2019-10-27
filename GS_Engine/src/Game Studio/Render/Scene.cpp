#include "Scene.h"
#include "RAPI/RenderDevice.h"

#include "Application/Application.h"
#include "Math/GSM.hpp"
#include "Resources/StaticMeshResource.h"

#include "Material.h"
#include "Game/StaticMesh.h"

Scene::Scene() : RenderComponents(10), Framebuffers(3)
{
	Win = GS::Application::Get()->GetActiveWindow();

	RenderContextCreateInfo RCCI;
	RCCI.Window = Win;
	RC = RenderDevice::Get()->CreateRenderContext(RCCI);

	auto SCImages = RC->GetSwapchainImages();

	//ImageCreateInfo CACI;
	//CACI.Extent = GetWindow()->GetWindowExtent();
	//CACI.LoadOperation = LoadOperations::CLEAR;
	//CACI.StoreOperation = StoreOperations::UNDEFINED;
	//CACI.Dimensions = ImageDimensions::IMAGE_2D;
	//CACI.InitialLayout = ImageLayout::COLOR_ATTACHMENT;
	//CACI.FinalLayout = ImageLayout::COLOR_ATTACHMENT;
	//CACI.Use = ImageUse::COLOR_ATTACHMENT;
	//CACI.Type = ImageType::COLOR;
	//CACI.ImageFormat = Format::RGB_I8;
	//auto CA = RAPI::GetRAPI()->CreateImage(CACI);

	RenderPassCreateInfo RPCI;
	RenderPassDescriptor RPD;
	AttachmentDescriptor SIAD;
	SubPassDescriptor SPD;
	AttachmentReference SubPassWriteAttachmentReference;
	AttachmentReference SubPassReadAttachmentReference;

	SIAD.AttachmentImage = SCImages[0]; //Only first because it gets only properties, doesn't access actual data.
	SIAD.InitialLayout = ImageLayout::UNDEFINED;
	SIAD.FinalLayout = ImageLayout::PRESENTATION;
	SIAD.StoreOperation = StoreOperations::STORE;
	SIAD.LoadOperation = LoadOperations::CLEAR;

	SubPassWriteAttachmentReference.Layout = ImageLayout::COLOR_ATTACHMENT;
	SubPassWriteAttachmentReference.Index = 0;

	SubPassReadAttachmentReference.Layout = ImageLayout::GENERAL;
	SubPassReadAttachmentReference.Index = ATTACHMENT_UNUSED;

	SPD.WriteColorAttachments.push_back(&SubPassWriteAttachmentReference);
	SPD.ReadColorAttachments.push_back(&SubPassReadAttachmentReference);

	RPD.RenderPassColorAttachments.push_back(&SIAD);
	RPD.SubPasses.push_back(&SPD);

	RPCI.Descriptor = RPD;
	RP = RenderDevice::Get()->CreateRenderPass(RPCI);

	UniformLayoutCreateInfo ULCI;
	ULCI.RenderContext = RC;
	ULCI.PipelineUniformSets[0].UniformSetType = UniformType::UNIFORM_BUFFER;
	ULCI.PipelineUniformSets[0].ShaderStage = ShaderType::VERTEX_SHADER;
	ULCI.PipelineUniformSets[0].UniformSetUniformsCount = 1;
	ULCI.PipelineUniformSets.setLength(1);
	
	PushConstant MyPushConstant;
	MyPushConstant.Size = sizeof(Matrix4);
	ULCI.PushConstant = &MyPushConstant;
	
	UL = RenderDevice::Get()->CreateUniformLayout(ULCI);

	UniformBufferCreateInfo UBCI;
	UBCI.Size = sizeof(Matrix4);
	UB = RenderDevice::Get()->CreateUniformBuffer(UBCI);

	UniformLayoutUpdateInfo ULUI;
	ULUI.PipelineUniformSets[0].UniformSetType = UniformType::UNIFORM_BUFFER;
	ULUI.PipelineUniformSets[0].ShaderStage = ShaderType::VERTEX_SHADER;
	ULUI.PipelineUniformSets[0].UniformSetUniformsCount = 1;
	ULUI.PipelineUniformSets[0].UniformData = UB;
	ULUI.PipelineUniformSets.setLength(1);
	UL->UpdateUniformSet(ULUI);

	Framebuffers.resize(SCImages.length());
	for (uint8 i = 0; i < SCImages.length(); ++i)
	{
		FramebufferCreateInfo FBCI;
		FBCI.RenderPass = RP;
		FBCI.Extent = Win->GetWindowExtent();
		FBCI.Images = DArray<Image*>(&SCImages[i], 1);
		Framebuffers[i] = RenderDevice::Get()->CreateFramebuffer(FBCI);
	}

	MeshCreateInfo MCI;
	MCI.IndexCount = ScreenQuad::IndexCount;
	MCI.VertexCount = ScreenQuad::VertexCount;
	MCI.VertexData = ScreenQuad::Vertices;
	MCI.IndexData = ScreenQuad::Indices;
	MCI.VertexLayout = &ScreenQuad::VD;
	FullScreenQuad = RenderDevice::Get()->CreateMesh(MCI);

	GraphicsPipelineCreateInfo gpci;
	gpci.RenderPass = RP;
	gpci.UniformLayout = UL;
	gpci.VDescriptor = &ScreenQuad::VD;
	gpci.PipelineDescriptor.BlendEnable = false;
	
	FString VS("#version 450\nlayout(push_constant) uniform Push {\nmat4 Mat;\n} inPush;\nlayout(binding = 0) uniform Data {\nmat4 Pos;\n} inData;\nlayout(location = 0)in vec3 inPos;\nlayout(location = 1)in vec3 inTexCoords;\nlayout(location = 0)out vec4 tPos;\nvoid main()\n{\ngl_Position = vec4(inPos, 1.0) * inData.Pos;\n}");
	gpci.PipelineDescriptor.Stages.push_back(ShaderInfo{ ShaderType::VERTEX_SHADER, &VS });
	
	FString FS("#version 450\nlayout(location = 0)in vec4 tPos;\nlayout(location = 0) out vec4 outColor;\nvoid main()\n{\noutColor = vec4(1, 1, 1, 1);\n}");
	gpci.PipelineDescriptor.Stages.push_back(ShaderInfo{ ShaderType::FRAGMENT_SHADER, &FS });

	FullScreenRenderingPipeline = RenderDevice::Get()->CreateGraphicsPipeline(gpci);
}

Scene::~Scene()
{
	for (auto& Element : RenderComponents)
	{
		delete Element;
	}

	for (auto const& x : Pipelines)
	{
		delete x.second;
	}

	delete RP;
	delete RC;
}

void Scene::OnUpdate()
{
	/*Update debug vars*/
	GS_DEBUG_ONLY(DrawCalls = 0)
	GS_DEBUG_ONLY(InstanceDraws = 0)
	GS_DEBUG_ONLY(PipelineSwitches = 0)
	GS_DEBUG_ONLY(DrawnComponents = 0)
	/*Update debug vars*/
	
	UpdateMatrices();
	
	UniformBufferUpdateInfo uniform_buffer_update_info;
	uniform_buffer_update_info.Data = &ViewProjectionMatrix;
	uniform_buffer_update_info.Size = sizeof(Matrix4);
	UB->UpdateBuffer(uniform_buffer_update_info);

	RC->BeginRecording();	
	
	RenderPassBeginInfo RPBI;
	RPBI.RenderPass = RP;
	RPBI.Framebuffers = Framebuffers.data();

	RC->BeginRenderPass(RPBI);

	RC->BindUniformLayout(UL);

	//BindPipeline(FullScreenRenderingPipeline);
	//DrawMesh(DrawInfo{ ScreenQuad::IndexCount, 1 }, FullScreenQuad);
	
	UpdateRenderables();
	
	RenderRenderables();

	RC->EndRenderPass(RP);

	RC->EndRecording();

	RC->AcquireNextImage();
	RC->Flush();
	RC->Present();
}

void Scene::DrawMesh(const DrawInfo& _DrawInfo, Mesh* _Mesh)
{
	RC->BindMesh(_Mesh);
	RC->DrawIndexed(_DrawInfo);
	GS_DEBUG_ONLY(++DrawCalls)
	GS_DEBUG_ONLY(InstanceDraws += _DrawInfo.InstanceCount)
}

void Scene::BindPipeline(GraphicsPipeline* _Pipeline)
{
	RC->BindGraphicsPipeline(_Pipeline);
	GS_DEBUG_ONLY(++PipelineSwitches);
}

GraphicsPipeline* Scene::CreatePipelineFromMaterial(Material* _Mat) const
{
	GraphicsPipelineCreateInfo GPCI;

	GPCI.VDescriptor = StaticMeshResource::GetVertexDescriptor();

	ShaderInfo VSI;
	ShaderInfo FSI;
	_Mat->GetRenderingCode(VSI, FSI);

	GPCI.PipelineDescriptor.Stages.push_back(VSI);
	GPCI.PipelineDescriptor.Stages.push_back(FSI);
	GPCI.PipelineDescriptor.BlendEnable = _Mat->GetHasTransparency();
	GPCI.PipelineDescriptor.ColorBlendOperation = BlendOperation::ADD;
	GPCI.PipelineDescriptor.CullMode = _Mat->GetIsTwoSided() ? CullMode::CULL_NONE : CullMode::CULL_BACK;
	GPCI.PipelineDescriptor.DepthCompareOperation = CompareOperation::GREATER;

	GPCI.RenderPass = RP;
	GPCI.UniformLayout = UL;

	return RenderDevice::Get()->CreateGraphicsPipeline(GPCI);
}

Mesh* Scene::RegisterMesh(StaticMesh* _SM)
{
	Mesh* NewMesh = nullptr;

	if (Meshes.find(_SM) == Meshes.end())
	{
		Model m = _SM->GetModel();

		MeshCreateInfo MCI;
		MCI.IndexCount = m.IndexCount;
		MCI.VertexCount = m.VertexCount;
		MCI.VertexData = m.VertexArray;
		MCI.IndexData = m.IndexArray;
		MCI.VertexLayout = StaticMeshResource::GetVertexDescriptor();
		NewMesh = RenderDevice::Get()->CreateMesh(MCI);
	}
	else
	{
		NewMesh = Meshes[_SM];
	}

	return NewMesh;
}

GraphicsPipeline* Scene::RegisterMaterial(Material* _Mat)
{
	auto Res = Pipelines.find(Id(_Mat->GetMaterialName()).GetID());
	if (Res != Pipelines.end())
	{
		return Pipelines[Res->first];
	}

	auto NP = CreatePipelineFromMaterial(_Mat);
	Pipelines.insert({ Id(_Mat->GetMaterialName()).GetID(), NP });
	return NP;
}

void Scene::UpdateMatrices()
{
	//We get and store the camera's position so as to not access it several times.
	const Vector3 CamPos = GetActiveCamera()->GetPosition();

	//We set the view matrix's corresponding component to the inverse of the camera's position to make the matrix a translation matrix in the opposite direction of the camera.
	ViewMatrix[12] = CamPos.X;
	ViewMatrix[13] = -CamPos.Y;
	ViewMatrix[14] = CamPos.Z;

	auto& nfp = GetActiveCamera()->GetNearFarPair();

	ProjectionMatrix = BuildPerspectiveMatrix(GetActiveCamera()->GetFOV(), Win->GetAspectRatio(), nfp.First, nfp.Second);

	ViewProjectionMatrix = ProjectionMatrix * ViewMatrix;
}

void Scene::RegisterRenderComponent(RenderComponent* _RC, RenderComponentCreateInfo* _RCCI)
{
	auto RI = _RC->GetRenderableInstructions();
	
	CreateInstanceResourcesInfo CIRI{ _RC, this };
	CIRI.RenderComponentCreateInfo = _RCCI;
	RI.CreateInstanceResources(CIRI);

	RegisterMaterial(CIRI.Material);
	
	RenderableInstructionsMap.try_emplace(Id(_RC->GetRenderableTypeName()).GetID(), _RC->GetRenderableInstructions());
	RenderComponents.emplace_back(_RC);
}

void Scene::UpdateRenderables()
{
	for (auto& e : RenderComponents)
	{
	}
}

void Scene::RenderRenderables()
{
	BindPipeline(Pipelines.begin()->second);
	
	for(auto& e : RenderComponents)
	{
		DrawInstanceInfo DII{this, e };
		e->GetRenderableInstructions().DrawInstance(DII);
	}
}

Matrix4 Scene::BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far)
{
	const auto Tangent = GSM::Tangent(GSM::Clamp(FOV * 0.5f, 0.0f, 90.0f)); //Tangent of half the vertical view angle.
	const auto Height = Near * Tangent;			//Half height of the near plane(point that says where it is placed).
	const auto Width = Height * AspectRatio;	//Half width of the near plane(point that says where it is placed).

	return Matrix4( 1 / (AspectRatio * Tangent), 0, 0, 0,
					0, 1 / Tangent, 0, 0,
					0, 0, Far / (Near - Far), 1, 
					0, 0, -(Far * Near) / (Far - Near), 0);
	
	//return BuildPerspectiveFrustum(Width, -Width, Height, -Height, Near, Far);

		/*return Matrix4(
		  Tangent / AspectRatio,
		  0.0f,
		  0.0f,
		  0.0f,

		  0.0f,
		  -Tangent,
		  0.0f,
		  0.0f,

		  0.0f,
		  0.0f,
		  Far / (Near - Far),
		  -1.0f,

		  0.0f,
		  0.0f,
		  (Near * Far) / (Near - Far),
		  0.0f);
		*/
	
	//return BuildPerspectiveFrustum(Width, -Width, Height, -Height, Near, Far);	
}

Matrix4 Scene::BuildPerspectiveFrustum(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far)
{
	Matrix4 Result;

	const auto near2 = Near * 2.0f;
	const auto top_m_bottom = Top - Bottom;
	const auto far_m_near = Far - Near;
	const auto right_m_left = Right - Left;

	Result[0] = near2 / right_m_left;
	Result[5] = near2 / top_m_bottom;
	Result[8] = (Right + Left) / right_m_left;
	Result[9] = (Top + Bottom) / top_m_bottom;
	Result[10] = -(Far + Near) / (far_m_near);
	Result[11] = -1.0f;
	Result[14] = -near2 * Far / far_m_near;
	Result[15] = 0.0f;
	
	return Result;
}
