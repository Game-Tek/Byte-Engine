#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "W:/Game Studio/GS_Engine/vendor/GLFW/glfw3.h"

GS_CLASS Window : public ESystem
{
public:
	Window(unsigned short WindowWidth, unsigned short WindowHeight, const char * WindowName);
	~Window();

	void OnUpdate() override;

	//Enable or disable V-Sync.
	void SetVsync(bool Enable);

	GLFWwindow * GetGLFWWindow() const { return GLWindow; }

	unsigned short GetWindowWidth() const { return WindowWidth; }
	unsigned short GetWindowHeight() const { return WindowHeight; }
private:
	GLFWwindow * GLWindow;

	unsigned short WindowWidth;
	unsigned short WindowHeight;

};

