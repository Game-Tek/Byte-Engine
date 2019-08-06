#include <GameStudio.h>

#include <Game Studio/RAPI/Window.h>
#include <Game Studio/RAPI/Renderer.h>

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		RenderContextCreateInfo RCCI;
		RCCI.Window = GetWindow();
		auto RC = Renderer::GetRenderer()->CreateRenderContext(RCCI);

		ImageCreateInfo CACI;
		CACI.Extent = GetWindow()->GetWindowExtent();
		CACI.LoadOperation = LoadOperations::CLEAR;
		CACI.StoreOperation = StoreOperations::UNDEFINED;
		CACI.Dimensions = ImageDimensions::IMAGE_2D;
		CACI.InitialLayout = ImageLayout::COLOR_ATTACHMENT;
		CACI.FinalLayout = ImageLayout::COLOR_ATTACHMENT;
		CACI.Use = ImageUse::COLOR_ATTACHMENT;
		CACI.Type = ImageType::COLOR;
		auto CA = Renderer::GetRenderer()->CreateImage(CACI);


		RenderPassCreateInfo RPCI;
		RenderPassDescriptor RPD;
		RPD.RenderPassColorAttachments.push_back(CA);

		RPD.SubPasses[0].WriteColorAttachments[0].Index = 0;
		RPD.SubPasses[0].WriteColorAttachments[0].Layout = ImageLayout::COLOR_ATTACHMENT;
		RPD.SubPasses[0].WriteColorAttachments.setLength(1);

		RPCI.RPDescriptor = RPD;
		auto RP = Renderer::GetRenderer()->CreateRenderPass(RPCI);

		FramebufferCreateInfo FBCI;
		FBCI.RenderPass = RP;
		FBCI.Extent = GetWindow()->GetWindowExtent();
		FBCI.Images = CA;
		FBCI.ImagesCount = 1;

		auto FB = Renderer::GetRenderer()->CreateFramebuffer(FBCI);
	}

	~Sandbox()
	{

	}
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}