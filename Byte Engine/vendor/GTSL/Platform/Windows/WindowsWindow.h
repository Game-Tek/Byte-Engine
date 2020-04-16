#pragma once

#include "Window.h"

#if (_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <Windows.h>

namespace GTSL
{
	class WindowsWindow : public Window
	{
		HWND windowHandle = nullptr;

		WindowSizeState windowSizeState;
		
		float mouseX{0}, mouseY{0};

		DWORD defaultWindowStyle{ 0 };

		static uint64 __stdcall WindowProc(HWND hwnd, UINT uMsg, WPARAM wParam, LPARAM lParam);

		void CalculateMousePos(uint32 x, uint32 y);
		static void TranslateKeys(uint32 win32Key, uint64 context, KeyboardKeys& key);
	public:
		WindowsWindow(const WindowCreateInfo& windowCreateInfo);

		void SetState(const WindowState& windowState) override;

		void GetNativeHandles(void* nativeHandlesStruct) override;

		void Notify() override;

		void SetTitle(const char* title) override;
	};
}
#endif