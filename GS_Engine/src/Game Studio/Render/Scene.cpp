#include "Scene.h"
#include "RAPI/RAPI.h"

#include "Application/Application.h"
#include "Math/GSM.hpp"

Scene::Scene()
{
	WindowCreateInfo WCI;
	WCI.Extent = { 1280, 720 };
	WCI.Name = "Game Studio!";
	WCI.WindowType = WindowFit::NORMAL;
	Win = Window::CreateWindow(WCI);

	GS::Application::Get()->SetActiveWindow(Win);


	RenderContextCreateInfo RCCI;
	RCCI.Window = Win;
	RC = RAPI::GetRAPI()->CreateRenderContext(RCCI);

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
	RP = RAPI::GetRAPI()->CreateRenderPass(RPCI);

	ShaderInfo VS;
	VS.Type = ShaderType::VERTEX_SHADER;
	const char* VertexShaderCode =
		R"(
		#version 450
		
		layout(push_constant) uniform PushConstant
		{
			mat4 ModelMatrix;
		} PushConstant;

		layout(binding = 0)uniform inObjPos
		{
			vec4 AddPos;
		} UBO;

		layout(location = 0)in vec2 inPos;
		layout(location = 1)in vec2 inTexCoords;

		layout(location = 0)out vec4 tPos;

		void main()
		{
			tPos = vec4(inPos, 0.0, 1.0) * PushConstant.ModelMatrix;
			gl_Position = tPos;
		})";
	FString VSC(VertexShaderCode);
	VS.ShaderCode = VSC;

	ShaderInfo FS;
	FS.Type = ShaderType::FRAGMENT_SHADER;
	const char* FragmentShaderCode =
		R"(
		#version 450

		layout(location = 0)in vec4 tPos;
		
		layout(location = 0) out vec4 outColor;

		void main()
		{
			outColor = tPos;
		})";
	FString FSC(FragmentShaderCode);
	FS.ShaderCode = FSC;

	UniformLayoutCreateInfo ULCI;
	ULCI.RenderContext = RC;
	ULCI.PipelineUniformSets[0].UniformSetType = UniformType::UNIFORM_BUFFER;
	ULCI.PipelineUniformSets[0].ShaderStage = ShaderType::VERTEX_SHADER;
	ULCI.PipelineUniformSets[0].UniformSetUniformsCount = 1;
	ULCI.PipelineUniformSets.setLength(1);
	UL = RAPI::GetRAPI()->CreateUniformLayout(ULCI);

	GraphicsPipelineCreateInfo GPCI;
	GPCI.RenderPass = RP;
	GPCI.PipelineDescriptor.Stages.VertexShader = &VS;
	GPCI.PipelineDescriptor.Stages.FragmentShader = &FS;
	GPCI.SwapchainSize = Win->GetWindowExtent();
	GPCI.UniformLayout = UL;
	GPCI.VDescriptor = &Vertex2D::Descriptor;
	GP = RAPI::GetRAPI()->CreateGraphicsPipeline(GPCI);

	Framebuffers.resize(SCImages.length());
	for (uint8 i = 0; i < SCImages.length(); ++i)
	{
		FramebufferCreateInfo FBCI;
		FBCI.RenderPass = RP;
		FBCI.Extent = Win->GetWindowExtent();
		FBCI.Images = DArray<Image*>(&SCImages[i], 1);
		Framebuffers[i] = RAPI::GetRAPI()->CreateFramebuffer(FBCI);
	}
}

Scene::~Scene()
{
	for (auto& Element : StaticMeshes)
	{
		delete Element;
	}

	delete GP;
	delete RP;
	delete RC;
}

void Scene::OnUpdate()
{
	RC->BeginRecording();

	RenderPassBeginInfo RPBI;
	RPBI.RenderPass = RP;
	RPBI.Framebuffers = Framebuffers.data();

	RC->BeginRenderPass(RPBI);

	RC->BindGraphicsPipeline(GP);
	//RC->BindUniformLayout(UL);

	PushConstantsInfo PCI;
	PCI.Size = sizeof(Matrix4);
	auto ModelMat = GSM::Translation(StaticMeshes[0]->GetOwner()->GetPosition());
	PCI.Data = &ModelMat;
	PCI.UniformLayout = UL;
	RC->UpdatePushConstant(PCI);

	DrawInfo DI;
	DI.IndexCount = MyQuad.IndexCount;
	DI.InstanceCount = 1;
	RC->DrawIndexed(DI);

	RC->EndRenderPass(RP);

	RC->EndRecording();

	RC->AcquireNextImage();
	RC->Flush();
	RC->Present();
}

StaticMeshRenderComponent* Scene::CreateStaticMeshRenderComponent(WorldObject* _Owner) const
{
	auto SMRC = new StaticMeshRenderComponent();
	SMRC->SetOwner(_Owner);
	StaticMeshes.push_back(SMRC);
	return SMRC;
}

void Scene::UpdateViewMatrix()
{
	//We get and store the camera's position so as to not access it several times.
	const Vector3 CamPos = GetActiveCamera()->GetPosition();

	//We set the view matrix's corresponding component to the inverse of the camera's position to make the matrix a translation matrix in the opposite direction of the camera.
	ViewMatrix[12] = CamPos.X;
	ViewMatrix[13] = CamPos.Y;
	ViewMatrix[14] = CamPos.Z;
}

void Scene::UpdateProjectionMatrix()
{
	ProjectionMatrix = BuildPerspectiveMatrix(GetActiveCamera()->GetFOV(), Win->GetAspectRatio(), 0.1f, 500.0f);
}

Matrix4 Scene::BuildPerspectiveMatrix(const float FOV, const float AspectRatio, const float Near, const float Far)
{
	const float Tangent = GSM::Tangent(GSM::Clamp(FOV * 0.5f, 0.0f, 90.0f)); //Tangent of half the vertical view angle.
	const float Height = Near * Tangent;			//Half height of the near plane(point that says where it is placed).
	const float Width = Height * AspectRatio;		//Half width of the near plane(point that says where it is placed).

	return BuildPerspectiveFrustrum(Width, -Width, Height, -Height, Near, Far);
}

Matrix4 Scene::BuildPerspectiveFrustrum(const float Right, const float Left, const float Top, const float Bottom, const float Near, const float Far)
{
	Matrix4 Result;

	Result[0] = (2.0f * Near) / (Right - Left);
	Result[5] = (2.0f * Near) / (Top - Bottom);
	Result[8] = (Right + Left) / (Right - Left);
	Result[9] = (Top + Bottom) / (Top - Bottom);
	Result[10] = -((Far + Near) / (Far - Near));
	Result[11] = -1.0f;
	Result[14] = -((2.0f * Far * Near) / (Far - Near));
	Result[15] = 0.0f;

	return Result;
}
