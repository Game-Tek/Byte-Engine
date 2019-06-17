#include "Window.h"

#include "InputManager.h"
#include "Application.h"
#include "ImageSize.h"

Window::Window(const ImageSize& _WindowSize, const char * WindowName) : WindowSize(_WindowSize)
{
}

Window::~Window()
{
}

void Window::OnUpdate()
{
}

void Window::ResizeWindow(const uint16 WWidth, const uint16 WHeight)
{
	WindowSize.Width  = WWidth;
	WindowSize.Height = WHeight;

	return;
}

void Window::ResizeWindow(const ImageSize & _WindowSize)
{
	WindowSize = _WindowSize;

	return;
}
