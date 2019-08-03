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

		RenderPassCreateInfo RPCI;
		RenderPassDescriptor RPD;
		RPD.ColorAttachmentsCount = 1;
		RPD.ColorAttachments;
		RPD.DepthStencilAttachment.Layout;

		RPD.SubPasses[0].ColorAttachmentsCount = 1;
		RPD.SubPasses[0].PreserveAttachments[0] = 0;
		RPD.SubPasses[0].PreserveAttachmentsCount = 1;
		RPD.SubPasses[0].ReadColorAttachments[0].Layout = ImageLayout::COLOR_ATTACHMENT;
		RPD.SubPasses[0].ReadColorAttachments[0].Index = 0;
		RPD.SubPassesCount = 1;

		RPCI.RPDescriptor = RPD;

		FramebufferCreateInfo FBCI;
		FBCI.RenderPass;
		FBCI.Extent = GetWindow()->GetWindowExtent();
		FBCI.Images;
		FBCI.ImagesCount = 1;


	}

	~Sandbox()
	{

	}
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}