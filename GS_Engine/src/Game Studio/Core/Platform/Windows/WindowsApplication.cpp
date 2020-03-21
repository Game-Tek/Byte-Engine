#include "WindowsApplication.h"

WindowsApplication::WindowsApplication(const ApplicationCreateInfo& applicationCreateInfo) : nApplication(applicationCreateInfo), instance(GetModuleHandle(nullptr))
{
}

void WindowsApplication::Update()
{
	MSG message;
	GetMessage(&message, nullptr, 0, 0);
	TranslateMessage(&message);
	DispatchMessage(&message);
}

void WindowsApplication::Close()
{
	PostQuitMessage(0);
}
