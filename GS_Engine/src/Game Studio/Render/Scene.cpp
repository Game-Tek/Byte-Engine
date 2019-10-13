#include "Scene.h"
#include "RAPI/RenderDevice.h"

#include "Application/Application.h"
#include "Math/GSM.hpp"

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
	UBCI.Size = sizeof(Vector4);
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
}

Scene::~Scene()
{
	for (auto& Element : RenderComponents)
	{
		delete Element;
	}

	delete RP;
	delete RC;
}

void Scene::OnUpdate()
{
	UpdateMatrices();

	RC->BeginRecording();

	RenderPassBeginInfo RPBI;
	RPBI.RenderPass = RP;
	RPBI.Framebuffers = Framebuffers.data();

	RC->BeginRenderPass(RPBI);

	RenderRenderables();

	RC->EndRenderPass(RP);

	RC->EndRecording();

	RC->AcquireNextImage();
	RC->Flush();
	RC->Present();
}

void Scene::DrawMesh(const DrawInfo& _DI)
{
	RC->DrawIndexed(_DI);
	GS_DEBUG_ONLY(++DrawCalls)
}

void Scene::UpdateMatrices()
{
	//We get and store the camera's position so as to not access it several times.
	const Vector3 CamPos = GetActiveCamera()->GetPosition();

	//We set the view matrix's corresponding component to the inverse of the camera's position to make the matrix a translation matrix in the opposite direction of the camera.
	ViewMatrix[12] = CamPos.X;
	ViewMatrix[13] = CamPos.Y;
	ViewMatrix[14] = CamPos.Z;

	ProjectionMatrix = BuildPerspectiveMatrix(GetActiveCamera()->GetFOV(), Win->GetAspectRatio(), 0.1f, 500.0f);

	ViewProjectionMatrix = ProjectionMatrix * ViewMatrix;
}

void Scene::RegisterRenderComponent(RenderComponent* _RC) const
{
	auto RI = _RC->GetRenderableInstructions();
	RenderableInstructionsMap.try_emplace(Id(_RC->GetRenderableTypeName()).GetID(), _RC->GetRenderableInstructions());
	RenderComponents.emplace_back(_RC);

	CreateInstanceResourcesInfo CIRI { _RC };
	RI.CreateInstanceResources(CIRI);

	ResourcesManager.RegisterMesh(CIRI.StaticMesh);
	ResourcesManager.RegisterMaterial(CIRI.Material);
}

Matrix4 Scene::BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far)
{
	const float Tangent = GSM::Tangent(GSM::Clamp(FOV * 0.5f, 0.0f, 90.0f)); //Tangent of half the vertical view angle.
	const float Height = Near * Tangent;			//Half height of the near plane(point that says where it is placed).
	const float Width = Height * AspectRatio;		//Half width of the near plane(point that says where it is placed).

	return BuildPerspectiveFrustum(Width, -Width, Height, -Height, Near, Far);
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
	Result[10] = -((Far + Near) / far_m_near);
	Result[11] = -1.0f;
	Result[14] = -((near2 * Far) / far_m_near);
	Result[15] = 0.0f;

	return Result;
}
