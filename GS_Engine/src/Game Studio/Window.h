#pragma once

#include "Core.h"

#include "W:\Game Studio\GS_Engine\vendor\GLFW\glfw3.h"

GS_CLASS Window
{
public:
	Window(unsigned short WindowWidth, unsigned short WindowHeight, const char * WindowName);
	~Window();

private:
	GLFWwindow * GLWindow;

	const unsigned short WNDW_WIDTH;
	const unsigned short WNDW_HEIGHT;

};

