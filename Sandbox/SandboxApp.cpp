#include <GameStudio.h>

#include "Game Studio/Render/RenderContext.h"

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		WindowCreateInfo WCI;
		WCI.Extent = { 1280, 720 };
		WCI.Name = "Game Studio!";
		WCI.WindowType = WindowFit::NORMAL;
		Window* mWindow = Window::CreateGSWindow(WCI);

		RenderContextCreateInfo RCCI;
		RCCI.Window = mWindow;
		auto RC = Renderer::GetRenderer()->CreateRenderContext(RCCI);

		ShaderCreateInfo SCIvs;
		SCIvs.ShaderName = "VertexShader.vert";
		SCIvs.Type = ShaderType::VERTEX_SHADER;
		auto VS = Renderer::GetRenderer()->CreateShader(SCIvs);

		ShaderCreateInfo SCIfs;
		SCIfs.ShaderName = "FragmentShader.frag";
		SCIfs.Type = ShaderType::FRAGMENT_SHADER;
		auto FS = Renderer::GetRenderer()->CreateShader(SCIfs);

		GraphicsPipelineCreateInfo GPCI;
		GPCI.StagesInfo.Shader[0] = VS;
		GPCI.StagesInfo.Shader[1] = FS;
		GPCI.StagesInfo.ShaderCount = 2;
		GPCI.SwapchainSize = WCI.Extent;
		Renderer::GetRenderer()->CreateGraphicsPipeline(GPCI);
	}

	~Sandbox()
	{

	}
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}