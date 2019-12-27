#include "Application.h"
#include "Debug/Logger.h"

GS::Application * GS::Application::ApplicationInstance = nullptr;

namespace GS
{
	Application::Application(const ApplicationCreateInfo& ACI)
	{
		ApplicationInstance = this;

		ResourceManagerInstance = new ResourceManager();

		WindowCreateInfo WCI;
		WCI.Extent = { 720, 720 };
		WCI.Name = ACI.ApplicationName;
		WCI.WindowType = WindowFit::NORMAL;

		SetActiveWindow(Window::CreateWindow(WCI));
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

		GS_LOG_WARNING("Shutting down application!\nReason: %s", CloseReason.c_str())

		return 0;
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
