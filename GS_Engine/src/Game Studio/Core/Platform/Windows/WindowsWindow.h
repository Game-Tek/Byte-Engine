#pragma once

#include "Core/Window.h"

class nWindowsWindow : public nWindow
{
	void* windowHandle = nullptr;

	static uint64 __stdcall WindowProc(void* hwnd, uint32 uMsg, uint64* wParam, uint64* lParam);
public:
	nWindowsWindow(const WindowCreateInfo& windowCreateInfo);
};
