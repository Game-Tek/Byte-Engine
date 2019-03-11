#pragma once

#include "Core.h"

#include "EngineSystem.h"

struct ImageSize;
struct GLFWwindow;

GS_CLASS Window : public ESystem
{
public:
	Window(uint16 WindowWidth, uint16 WindowHeight, const char * WindowName);
	~Window();

	void OnUpdate() override;

	//Enable or disable V-Sync.
	void SetVsync(const bool Enable) const;

	INLINE GLFWwindow * GetGLFWWindow() const { return GLWindow; }

	ImageSize GetWindowSize() const;
	INLINE uint16 GetWindowWidth() const { return WindowWidth; }
	INLINE uint16 GetWindowHeight() const { return WindowHeight; }

	INLINE float GetAspectRatio() const { return static_cast<float>(WindowWidth) / static_cast<float>(WindowHeight); }

	void ResizeWindow(uint16 WWidth, uint16 WHeight);

protected:
	GLFWwindow * GLWindow;

	uint16 WindowWidth;
	uint16 WindowHeight;
};