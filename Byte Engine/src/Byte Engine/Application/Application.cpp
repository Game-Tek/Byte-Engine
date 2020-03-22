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

		RAPI::WindowCreateInfo WCI;
		WCI.Extent = { 720, 720 };
		WCI.Name = ACI.ApplicationName;
		WCI.WindowType = RAPI::WindowFit::NORMAL;

		SetActiveWindow(RAPI::Window::CreateWindow(WCI));

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
			InputManagerInstance.OnUpdate();

			ActiveWindow->Update();

			OnUpdate(); //Update instanced engine class
		}

		BE_LOG_WARNING("Shutting down application!\nReason: %s", CloseReason.c_str())

		return 0;
	}

	void Application::SetActiveWindow(RAPI::Window* _NewWindow)
	{
		BE_DEBUG_ONLY(if (ActiveWindow) BE_LOG_WARNING("An active window is already set!\nAlthough the recently input window will be regarded as the new active window make sure you are doing what you intend."))
		ActiveWindow = _NewWindow;
		InputManagerInstance.SetActiveWindow(ActiveWindow);
	}

	void Application::Close(const char* _Reason)
	{
		if (_Reason) CloseReason = _Reason;
		flaggedForClose = true;
	}

	void Application::PromptClose()
	{
		//CloseDelegate.Dispatch();
		ActiveWindow->NotifyWindow();
		ActiveWindow->FocusWindow();
	}

	bool Application::ShouldClose() const
	{
		return ActiveWindow->GetShouldClose() || flaggedForClose;
	}
}
