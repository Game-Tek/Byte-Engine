#include "Scene.h"
#include "RAPI/RenderDevice.h"

#include "Application/Application.h"
#include "Math/GSM.hpp"
#include "Resources/StaticMeshResource.h"

#include "Material.h"
#include "Game/StaticMesh.h"

Scene::Scene() : RenderComponents(10), Framebuffers(3), ViewMatrix(1), ProjectionMatrix(1), ViewProjectionMatrix(1)
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

	Framebuffers.resize(SCImages.getLength());
	for (uint8 i = 0; i < SCImages.getLength(); ++i)
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
	
	FString VS("#version 450\nlayout(push_constant) uniform Push {\nmat4 Mat;\n} inPush;\nlayout(binding = 0) uniform Data {\nmat4 Pos;\n} inData;\nlayout(location = 0)in vec3 inPos;\nlayout(location = 1)in vec3 inTexCoords;\nlayout(location = 0)out vec4 tPos;\nvoid main()\n{\ngl_Position = inData.Pos * vec4(inPos, 1.0);\n}");
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
	RPBI.Framebuffers = Framebuffers.getData();

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
	ViewMatrix(3, 0) = CamPos.X;
	ViewMatrix(3, 1) = -CamPos.Y;
	ViewMatrix(3, 2) = CamPos.Z;

	auto& nfp = GetActiveCamera()->GetNearFarPair();

	auto t = Win->GetAspectRatio();
	
	BuildPerspectiveMatrix(ProjectionMatrix, GetActiveCamera()->GetFOV(), Win->GetAspectRatio(), 1, 500);

	//MakeOrthoMatrix(ProjectionMatrix, 16, -16, 9, -9, 1, 500);
	
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

void Scene::BuildPerspectiveMatrix(Matrix4& _Matrix, const float _FOV, const float _AspectRatio, const float _Near, const float _Far)
{
	const auto tan_half_fov = GSM::Tangent(GSM::Clamp(_FOV * 0.5f, 0.0f, 90.0f)); //Tangent of half the vertical view angle.

	//Zero to one
	//Left handed

	/*GLM LH_ZO Code*/
	
	//_Matrix(0, 0) = 1 / (_AspectRatio * tan_half_fov);
	//_Matrix(1, 1) = 1 / tan_half_fov;
	//_Matrix(2, 2) = _Far / (_Far - _Near);
	//_Matrix(3, 2) = 1;
	//_Matrix(2, 3) = -(_Far * _Near) / (_Far - _Near);
	//_Matrix(3, 3) = 0;

	
	/*GLM LH_ZO Code*/
	
	/*Vulkan Cookbook Code*/

	const auto f = 1 / tan_half_fov;
	
	_Matrix(0, 0) = f / _AspectRatio;
	_Matrix(1, 1) = -f;
	_Matrix(2, 2) = _Far / (_Near - _Far);
	_Matrix(2, 3) = -1;
	_Matrix(3, 2) = (_Near * _Far) / (_Near - _Far);
	_Matrix(3, 3) = 0;

	/*Vulkan Cookbook Code*/


	/*https://stackoverflow.com/questions/18404890/how-to-build-perspective-projection-matrix-no-api*/
	
	//_Matrix(1, 1) = tan_half_fov;
	//_Matrix(0, 0) = 1 * _Matrix(1, 1) / _AspectRatio;
	//_Matrix(2, 2) = _Far * (1 / (_Far - _Near));
	//_Matrix(3, 2) = (-_Far * _Near) * (1 / (_Far - _Near));
	//_Matrix(2, 3) = -1;
	//_Matrix(3, 3) = 0;

	/*https://stackoverflow.com/questions/18404890/how-to-build-perspective-projection-matrix-no-api*/
	
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

void Scene::MakeOrthoMatrix(Matrix4& _Matrix, const float _Right, const float _Left, const float _Top,
	const float _Bottom, const float _Near, const float _Far)
{
	//Zero to one
	//Left handed

	_Matrix(0, 0) = static_cast<float>(2) / (_Right - _Left);
	_Matrix(1, 1) = static_cast<float>(2) / (_Top - _Bottom);
	_Matrix(2, 2) = static_cast<float>(1) / (_Far - _Near);
	_Matrix(3, 0) = -(_Right + _Left) / (_Right - _Left);
	_Matrix(3, 1) = -(_Top + _Bottom) / (_Top - _Bottom);
	_Matrix(3, 2) = -_Near / (_Far - _Near);
}
