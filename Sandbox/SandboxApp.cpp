#include <GameStudio.h>

#include <Game Studio/RAPI/Window.h>
#include <Game Studio/RAPI/Renderer.h>
#include <Game Studio/Containers/FVector.hpp>
#include "ScreenQuad.h"

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
		RPCI.RPDescriptor = RPD;
		RPD.RenderPassColorAttachments.push_back(SCImages[0]);
		RPD.SubPasses[0].WriteColorAttachments[0].Index = 0;
		RPD.SubPasses[0].WriteColorAttachments[0].Layout = ImageLayout::COLOR_ATTACHMENT;
		RPD.SubPasses[0].WriteColorAttachments.setLength(1);
		RP = Renderer::GetRenderer()->CreateRenderPass(RPCI);

		ShaderInfo VS;
		VS.Type = ShaderType::VERTEX_SHADER;
		VS.ShaderCode;

		ShaderInfo FS;
		FS.Type = ShaderType::FRAGMENT_SHADER;
		FS.ShaderCode;

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
			FBCI.Images = SCImages[i];
			FBCI.ImagesCount = 1;
			Framebuffers[i] = Renderer::GetRenderer()->CreateFramebuffer(FBCI);
		}

		MeshCreateInfo MCI;
		MCI.VertexCount = ScreenQuad::VertexCount;
		MCI.IndexCount = ScreenQuad::IndexCount;
		MCI.VertexData = ScreenQuad::Vertices;
		MCI.IndexData = ScreenQuad::Indices;
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
		DI.IndexCount = ScreenQuad::IndexCount;
		DI.InstanceCount = 1;

		RC->DrawIndexed(DI);

		RC->EndRenderPass(RP);

		RC->EndRecording();

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
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}