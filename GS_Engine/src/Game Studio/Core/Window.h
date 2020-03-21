#pragma once
#include "Containers/FString.h"
#include "Utility/Extent.h"

#include "Delegate.h"

class nWindow
{
protected:
	Delegate<void()> onCloseDelegate;
public:
	struct WindowCreateInfo
	{
		FString Name;
		Extent2D Extent;
		nWindow* ParentWindow = nullptr;
		class nApplication* Application = nullptr;
	};
	nWindow(const WindowCreateInfo& windowCreateInfo);

	void SetOnCloseDelegate(const Delegate<void()>& delegate) { onCloseDelegate = delegate; }
	
	enum class WindowState
	{
		MINIMIZED, MAXIMIZED, FULLSCREEN
	};
	virtual void SetState(WindowState windowState) = 0;
};
