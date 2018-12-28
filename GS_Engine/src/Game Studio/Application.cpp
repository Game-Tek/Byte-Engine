#include "Application.h"

namespace GS
{
	Application::Application()
	{
		ClockInstance = new Clock();
		WindowInstance = new Window(1280, 720, "Game Studio");
		RendererInstance = new Renderer(WindowInstance);
		EventDispatcherInstance = new EventDispatcher();
	}

	Application::~Application()
	{
		delete ClockInstance;
		delete WindowInstance;
		delete RendererInstance;
		delete EventDispatcherInstance;
	}

	void Application::Run()
	{
		while (true);
		{
			ClockInstance->OnUpdate();
			RendererInstance->OnUpdate(ClockInstance->GetDeltaTime());
		}	
	}
}