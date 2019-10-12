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

	ShaderInfo VS;
	VS.Type = ShaderType::VERTEX_SHADER;
	const char* VertexShaderCode =
		R"(
		#version 450
		
		layout(push_constant) uniform PushConstant
		{
			mat4 ModelMatrix;
		} callData;

		layout(binding = 0)uniform inObjPos
		{
			vec4 AddPos;
		} UBO;

		layout(location = 0)in vec3 inPos;
		layout(location = 1)in vec3 inTexCoords;

		layout(location = 0)out vec4 tPos;

		void main()
		{
			tPos = vec4(inPos, 1.0);// * callData.ModelMatrix;
			gl_Position = tPos;
		})";
	VS.ShaderCode = VertexShaderCode;

	ShaderInfo FS;
	FS.Type = ShaderType::FRAGMENT_SHADER;
	const char* FragmentShaderCode =
		R"(
		#version 450

		layout(location = 0)in vec4 tPos;
		
		layout(location = 0) out vec4 outColor;

		void main()
		{
			outColor = vec4(0.3, 0.1, 0.5, 0);//tPos;
		})";
	FS.ShaderCode = FragmentShaderCode;

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

	GraphicsPipelineCreateInfo GPCI;
	GPCI.RenderPass = RP;
	GPCI.PipelineDescriptor.Stages.VertexShader = &VS;
	GPCI.PipelineDescriptor.Stages.FragmentShader = &FS;
	GPCI.SwapchainSize = Win->GetWindowExtent();
	GPCI.UniformLayout = UL;
	GPCI.VDescriptor = &ScreenQuad::VD;//StaticMesh::GetVertexDescriptor();
	GP = RenderDevice::Get()->CreateGraphicsPipeline(GPCI);

	Framebuffers.resize(SCImages.length());
	for (uint8 i = 0; i < SCImages.length(); ++i)
	{
		FramebufferCreateInfo FBCI;
		FBCI.RenderPass = RP;
		FBCI.Extent = Win->GetWindowExtent();
		FBCI.Images = DArray<Image*>(&SCImages[i], 1);
		Framebuffers[i] = RenderDevice::Get()->CreateFramebuffer(FBCI);
	}

	ScreenQuad SQ;
}

Scene::~Scene()
{
	for (auto& Element : RenderComponents)
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
	RC->BindUniformLayout(UL);

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

void Scene::DrawMesh(const DrawInfo& _DI)
{
	RC->DrawIndexed(_DI);
	++DrawCalls;
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
