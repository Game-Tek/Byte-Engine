#pragma once

#include "Core.h"

#include <RAPI/Window.h>
#include <Windows.h>

struct GLFWwindow;

GS_CLASS WindowsWindow final : public Window
{
	HWND WindowObject = nullptr;
	HINSTANCE WindowInstance = nullptr;
	
	GLFWwindow* GLFWWindow = nullptr;

	static int32 KeyboardKeysToGLFWKeys(KeyboardKeys _IE);
	static KeyState GLFWKeyStateToKeyState(int32 _KS);
public:
	WindowsWindow(Extent2D _Extent, WindowFit _Fit, const FString& _Name);
	~WindowsWindow();

	INLINE HWND GetWindowObject() const { return WindowObject; }
	INLINE HINSTANCE GetHInstance() const { return WindowInstance; }

	void Update() override;

	void SetWindowFit(WindowFit _Fit) override;
	void MinimizeWindow() override;
	void NotifyWindow() override;
};