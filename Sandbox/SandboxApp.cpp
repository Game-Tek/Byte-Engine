#include <GameStudio.h>

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		WindowCreateInfo WCI;
		auto Window = Renderer::GetRenderer()->CreateWindow(WCI);

		RenderContextCreateInfo RCCI;
		RCCI.Extent = Extent2D(1280, 720);
		RCCI.Window = Window;
		Renderer::GetRenderer()->CreateRenderContext(RCCI);

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
		GPCI.SwapchainSize = RCCI.Extent;
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