#pragma once

#include "Core.h"

#include "..\..\Window.h"

#include <windef.h>

class GLFWwindow;

class WindowsWindow final : public Window
{
	HWND WindowObject = nullptr;
	
	GLFWwindow* GLFWWindow = nullptr;

	static int32 KeyboardKeysToGLFWKeys(KeyboardKeys _IE);
	static KeyState GLFWKeyStateToKeyState(int32 _KS);
public:
	WindowsWindow(Extent2D _Extent, const String& _Name);
	~WindowsWindow();

	INLINE HWND GetWindowObject() const { return WindowObject; }

	virtual void Update();
};