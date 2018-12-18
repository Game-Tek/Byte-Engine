#include "Application.h"

#include "Clock.h"
#include "Renderer/Renderer.h"

namespace GS
{
	Application::Application()
	{
		//FileManagerInstace = new FileManager();
		//EventDispatcherInstance = new EventDispatcher();
		//ClockInstance = new Clock();
		//RendererInstance = new Renderer();
		//SoundInstance = new Sound();
		//EntityManager = new EntityManager();
	}

	Application::~Application()
	{
		//delete ClockInstance;
		//delete RendererInstance;
		//delete EventDispatcherInstance;
	}

	void Application::Run()
	{
		while (true);
		{
			//RendererInstance::Update();
		}	
	}
}