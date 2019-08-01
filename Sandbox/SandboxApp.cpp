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
	}

	~Sandbox()
	{

	}
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}