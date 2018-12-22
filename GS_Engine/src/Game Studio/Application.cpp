#include "Application.h"

namespace GS
{
	Application::Application()
	{
		ClockInstance = new Clock();
		WindowInstance = new Window(1280, 720, "Game Studio");
		RendererInstance = new Renderer();
	}

	Application::~Application()
	{
		delete ClockInstance;
		delete WindowInstance;
		delete RendererInstance;
	}

	void Application::Run()
	{
		while (true);
		{
			ClockInstance->Update();
			RendererInstance->Update(ClockInstance->GetDeltaTime());
		}	
	}
}