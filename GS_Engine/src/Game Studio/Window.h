#pragma once

#include "Core.h"

#include "W:/Game Studio/GS_Engine/vendor/GLFW/glfw3.h"

GS_CLASS Window
{
public:
	Window(unsigned short WindowWidth, unsigned short WindowHeight, const char * WindowName);
	~Window();

	//Enable or disable V-Sync.
	void SetVsync(bool Enable);

	GLFWwindow * GetGLFWWindow() const { return GLWindow; }

	unsigned short GetWindowWidth() const { return WNDW_WIDTH; }
	unsigned short GetWindowHeight() const { return WNDW_HEIGHT; }
private:
	GLFWwindow * GLWindow;

	unsigned short WNDW_WIDTH;
	unsigned short WNDW_HEIGHT;

};

