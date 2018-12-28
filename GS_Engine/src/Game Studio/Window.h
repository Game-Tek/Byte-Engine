#pragma once

#include "Core.h"

#include "glfw3.h"

GS_CLASS Window
{
public:
	Window(unsigned short WindowWidth, unsigned short WindowHeight, const char * WindowName);
	~Window();

	void SetVsync(bool Enable);
	static GLFWwindow * GetWindowInstance();
private:
	static GLFWwindow * GLWindow;

	const unsigned short WNDW_WIDTH;
	const unsigned short WNDW_HEIGHT;

};

