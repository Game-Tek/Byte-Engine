#include "Application.h"
#include "Debug/Logger.h"
#include "Resources/AudioResourceManager.h"

BE::Application* BE::Application::ApplicationInstance = nullptr;

namespace BE
{
	Application::Application(const ApplicationCreateInfo& ACI)
	{
		ApplicationInstance = this;

		ResourceManagerInstance = new ResourceManager();

		ResourceManagerInstance->CreateSubResourceManager<AudioResourceManager>();
	}

	Application::~Application()
	{
	}

	int Application::Run(int argc, char** argv)
	{
		while (!ShouldClose())
		{
			ClockInstance.OnUpdate();
			InputManagerInstance.Update();

			OnUpdate(); //Update instanced engine class
		}

		BE_LOG_WARNING("Shutting down application!\nReason: %s", CloseReason.c_str())

		return 0;
	}

	void Application::Close(const char* _Reason)
	{
		if (_Reason) CloseReason = _Reason;
		flaggedForClose = true;
	}

	void Application::PromptClose()
	{
		//CloseDelegate.Dispatch();
	}

	bool Application::ShouldClose() const
	{
		return flaggedForClose;
	}
}
