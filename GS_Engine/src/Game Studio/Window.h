#pragma once

#include "Core.h"

#include "glfw3.h"

GS_CLASS Window
{
public:
	Window(unsigned short WindowWidth, unsigned short WindowHeight, const char * WindowName);
	~Window();

	//Enable or disable V-Sync.
	void SetVsync(bool Enable);

	GLFWwindow * GetGLFWWindow();
private:
	GLFWwindow * GLWindow;

	const unsigned short WNDW_WIDTH;
	const unsigned short WNDW_HEIGHT;

};

