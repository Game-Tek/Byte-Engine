#include <GameStudio.h>

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		RenderContextCreateInfo RCCI;
		RCCI;
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
		GPCI.SwapchainSize = Extent2D(1280, 720);
		Renderer::GetRenderer()->CreateGraphicsPipeline(GPCI);

		CommandBufferCreateInfo CBCI;
		CBCI;
		Renderer::GetRenderer()->CreateCommandBuffer(CBCI);
	}

	~Sandbox()
	{

	}
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}