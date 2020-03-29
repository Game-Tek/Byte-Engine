#pragma once

#include "Window.h"

#define WIN32_LEAN_AND_MEAN
#include <Windows.h>
#include "Extent.h"

class WindowsWindow : public Window
{
	HWND windowHandle = nullptr;
	Extent2D extent;

	float mouseX, mouseY;
	
	static uint64 __stdcall WindowProc(HWND hwnd, UINT uMsg, WPARAM wParam, LPARAM lParam);

	static void CalculateMousePos(uint32 x, uint32 y, float& xf, float& yf);
public:
	WindowsWindow(const WindowCreateInfo& windowCreateInfo);

	void SetState(WindowState windowState) override;
};
