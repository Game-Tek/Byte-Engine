#pragma once

#include "Core.h"

#include <RAPI/Window.h>
#include <Windows.h>

struct GLFWwindow;

class GS_API WindowsWindow final : public Window
{
	HWND WindowObject = nullptr;
	HINSTANCE WindowInstance = nullptr;
	
	GLFWwindow* GLFWWindow = nullptr;

	static int32 KeyboardKeysToGLFWKeys(KeyboardKeys _IE);
	static KeyState GLFWKeyStateToKeyState(int32 _KS);
public:
	WindowsWindow(const WindowCreateInfo& _WCI);
	~WindowsWindow();

	INLINE HWND GetWindowObject() const { return WindowObject; }
	INLINE HINSTANCE GetHInstance() const { return WindowInstance; }

	void Update() override;

	void SetWindowFit(WindowFit _Fit) override;
	void MinimizeWindow() override;
	void NotifyWindow() override;
	void SetWindowTitle(const char* _Title) override;
};