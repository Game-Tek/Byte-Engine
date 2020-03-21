#pragma once

#include "Core/Window.h"

#define WIN32_LEAN_AND_MEAN
#include <Windows.h>

class nWindowsWindow : public nWindow
{
	HWND windowHandle = nullptr;
	Extent2D extent;
	
	static uint64 __stdcall WindowProc(HWND hwnd, UINT uMsg, WPARAM wParam, LPARAM lParam);
public:
	nWindowsWindow(const WindowCreateInfo& windowCreateInfo);

	void SetState(WindowState windowState) override;
};
