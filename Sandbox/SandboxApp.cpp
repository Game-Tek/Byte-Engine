#include <GameStudio.h>

#include <Game Studio/RAPI/Window.h>
#include <Game Studio/RAPI/Renderer.h>
#include <Game Studio/Containers/FVector.hpp>
#include <Game Studio/ScreenQuad.h>
#include <string>

class Framebuffer;

class Sandbox final : public GS::Application
{
public:
	Sandbox()
	{
		RenderContextCreateInfo RCCI;
		RCCI.Window = GS::Application::GetWindow();
		RC = Renderer::GetRenderer()->CreateRenderContext(RCCI);
		
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
		//auto CA = Renderer::GetRenderer()->CreateImage(CACI);
		
		RenderPassCreateInfo RPCI;
		RenderPassDescriptor RPD;
		AttachmentDescriptor SIAD;
		SubPassDescriptor SPD;
		AttachmentReference SubPassWriteAttachmentReference;
		AttachmentReference SubPassReadAttachmentReference;
		AttachmentReference SubPassPreserveAttachmentReference;

		SIAD.AttachmentImage = SCImages[0];
		SIAD.InitialLayout = ImageLayout::UNDEFINED;
		SIAD.FinalLayout = ImageLayout::PRESENTATION;
		SIAD.StoreOperation = StoreOperations::UNDEFINED;
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
		RP = Renderer::GetRenderer()->CreateRenderPass(RPCI);
		
		ShaderInfo VS;
		VS.Type = ShaderType::VERTEX_SHADER;
		const char* VertexShaderCode =
		R"(
		#version 450

		layout(location = 0)in vec2 inPos;
		layout(location = 1)in vec2 inTexCoords;

		layout(location = 0)out vec4 tPos;

		void main()
		{
			tPos = vec4(inPos, 0.0, 1.0);
			gl_Position = vec4(inPos, 0.0, 1.0);
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
		
		GraphicsPipelineCreateInfo GPCI;
		GPCI.RenderPass = RP;
		GPCI.Stages.VertexShader = &VS;
		GPCI.Stages.FragmentShader = &FS;
		GPCI.SwapchainSize = GetWindow()->GetWindowExtent();
		GPCI.VDescriptor = &Vertex2D::Descriptor;
		GP = Renderer::GetRenderer()->CreateGraphicsPipeline(GPCI);
		
		Framebuffers.resize(SCImages.length());
		for (uint8 i = 0; i < SCImages.length(); ++i)
		{
			FramebufferCreateInfo FBCI;
			FBCI.RenderPass = RP;
			FBCI.Extent = GetWindow()->GetWindowExtent();
			FBCI.Images = DArray<Image*>(&SCImages[i], 1);
			Framebuffers[i] = Renderer::GetRenderer()->CreateFramebuffer(FBCI);
		}
		
		MeshCreateInfo MCI;
		MCI.VertexCount = MyQuad.VertexCount;
		MCI.IndexCount = MyQuad.IndexCount;
		MCI.VertexData = MyQuad.Vertices;
		MCI.IndexData = MyQuad.Indices;
		MCI.VertexLayout = &Vertex2D::Descriptor;
		M = Renderer::GetRenderer()->CreateMesh(MCI);


	}

	void Update() final override
	{
		RC->BeginRecording();
		
		RenderPassBeginInfo RPBI;
		RPBI.RenderPass = RP;
		RPBI.Framebuffers = Framebuffers.data();
		
		RC->BeginRenderPass(RPBI);
		
		RC->BindGraphicsPipeline(GP);
		RC->BindMesh(M);
		
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

	~Sandbox()
	{
		delete M;
		delete GP;
		delete RP;
		delete RC;
	}

	RenderContext* RC;
	RenderPass* RP;
	GraphicsPipeline* GP;
	Mesh* M;
	FVector<Framebuffer*> Framebuffers;
	ScreenQuad MyQuad = {};
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}