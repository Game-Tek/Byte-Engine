#include "Application.h"
#include "Debug/Logger.h"

GS::Application * GS::Application::ApplicationInstance = nullptr;

namespace GS
{
	Application::Application()
	{
		ApplicationInstance = this;

		ResourceManagerInstance = new ResourceManager();
	}

	Application::~Application()
	{
	}

	void Application::Run()
	{
		while (!ShouldClose())
		{
			ClockInstance.OnUpdate();
			InputManagerInstance.OnUpdate();

			ActiveWindow->Update();

			OnUpdate();
		}

		GS_LOG_WARNING("Shutting down application!\nReason: %s", CloseReason.c_str())
	}

	void Application::SetActiveWindow(Window* _NewWindow)
	{
		GS_DEBUG_ONLY(if (ActiveWindow) GS_LOG_WARNING("An active window is already set!\nAlthough the recently input window will be regarded as the new active window make sure you are doing what you intend."))
		ActiveWindow = _NewWindow;
		InputManagerInstance.SetActiveWindow(ActiveWindow);
	}

	void Application::Close(const char* _Reason)
	{
		if (_Reason) CloseReason = _Reason;
		FlaggedForClose = true;
	}

	void Application::PromptClose()
	{
		//CloseDelegate.Dispatch();
		ActiveWindow->NotifyWindow();
		ActiveWindow->FocusWindow();
	}

	bool Application::ShouldClose() const
	{
		return ActiveWindow->GetShouldClose() || FlaggedForClose;
	}
}