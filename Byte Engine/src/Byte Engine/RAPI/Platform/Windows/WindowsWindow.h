#pragma once

#include "Core.h"

#include <RAPI/Window.h>
#include <Windows.h>

struct GLFWwindow;

class WindowsWindow final : public RAPI::Window
{
	HWND WindowObject = nullptr;
	HINSTANCE WindowInstance = nullptr;

	GLFWwindow* GLFWWindow = nullptr;

	static int32 KeyboardKeysToGLFWKeys(KeyboardKeys _IE);
	static KeyState GLFWKeyStateToKeyState(int32 _KS);
public:
	WindowsWindow(const RAPI::WindowCreateInfo& _WCI);
	~WindowsWindow();

	INLINE HWND GetWindowObject() const { return WindowObject; }
	INLINE HINSTANCE GetHInstance() const { return WindowInstance; }

	void Update() override;

	void SetWindowFit(RAPI::WindowFit _Fit) override;
	void SetWindowResolution(Extent2D _Res) override;
	void SetWindowIcon(const RAPI::WindowIconInfo& _WII) override;
	void MinimizeWindow() override;
	void NotifyWindow() override;
	void FocusWindow() override;
	void SetWindowTitle(const char* _Title) override;

	Extent2D GetFramebufferSize() override;
	Vector2 GetContentScale() override;
};
