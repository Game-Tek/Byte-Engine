#pragma once

#include "Core.h"

#include "EngineSystem.h"

struct GLFWwindow;

GS_CLASS Window : public ESystem
{
public:
	Window(uint16 WindowWidth, uint16 WindowHeight, const char * WindowName);
	~Window();

	void OnUpdate() override;

	//Enable or disable V-Sync.
	void SetVsync(bool Enable);

	GLFWwindow * GetGLFWWindow() const { return GLWindow; }

	uint16 GetWindowWidth() const { return WindowWidth; }
	uint16 GetWindowHeight() const { return WindowHeight; }
private:
	GLFWwindow * GLWindow;

	uint16 WindowWidth;
	uint16 WindowHeight;
};