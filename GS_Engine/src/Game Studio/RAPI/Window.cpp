#include "Window.h"

#include "Platform/Windows/WindowsWindow.h"

#undef CreateWindow

RAPI::Window* RAPI::Window::CreateWindow(const WindowCreateInfo& _WCI)
{
#ifdef GS_PLATFORM_WIN
	return new WindowsWindow(_WCI);
#endif
}
