#include "Application.h"

#include "Byte Engine/Resources/AudioResourceManager.h"

BE::Application* BE::Application::applicationInstance = nullptr;

void onAssert(const char* text, int line, const char* file, const char* function)
{
	BE_BASIC_LOG_ERROR("ASSERT: Error: %s, Line: %i, File: %s, Function: %s.", text, line, file, function);
}

namespace BE
{
	Application::Application(const ApplicationCreateInfo& ACI)
	{
		applicationInstance = this;

		clockInstance = new Clock();
		resourceManagerInstance = new ResourceManager();
		inputManagerInstance = new InputManager();
	}

	Application::~Application()
	{
	}

	int Application::Run(int argc, char** argv)
	{
		while (!shouldClose())
		{
			transientAllocator.Clear();
			clockInstance->OnUpdate();

			if(isInBackground)
			{
				OnBackgroundUpdate();
			}
			else
			{
				inputManagerInstance->Update();
				OnNormalUpdate();
			}
		}

		if(closeMode != CloseMode::OK)
		{
			BE_LOG_WARNING("Shutting down application!\nReason: %s", closeReason.c_str())
		}

		return 0;
	}

	void Application::PromptClose()
	{
		//CloseDelegate.Dispatch();
	}

	void Application::Close(const CloseMode closeMode, const char* reason)
	{
		if (reason) closeReason = reason;
		flaggedForClose = true;
		this->closeMode = closeMode;
	}

	bool Application::shouldClose() const
	{
		return flaggedForClose;
	}
}
