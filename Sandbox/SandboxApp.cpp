#include <GameStudio.h>

#include <Game Studio/RAPI/Window.h>
#include <Game Studio/RAPI/Renderer.h>

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		WindowCreateInfo WCI;
		WCI.Extent = { 1280, 720 };
		WCI.Name = "Game Studio!";
		WCI.WindowType = WindowFit::NORMAL;
		auto mWindow = Window::CreateGSWindow(WCI);

		RenderContextCreateInfo RCCI;
		RCCI.Window = mWindow;
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