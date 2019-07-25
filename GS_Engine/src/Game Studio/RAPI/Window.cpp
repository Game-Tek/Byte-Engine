#include "Window.h"

#include "Platform/Windows/WindowsWindow.h"

Window* Window::CreateGSWindow(const WindowCreateInfo& _WCI)
{
#ifdef GS_PLATFORM_WIN
	return new WindowsWindow(_WCI.Extent, _WCI.WindowType, _WCI.Name);
#endif
}
