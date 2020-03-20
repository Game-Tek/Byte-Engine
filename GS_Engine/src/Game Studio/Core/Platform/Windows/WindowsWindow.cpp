#include "WindowsWindow.h"

#include <winuser.h>

uint64 nWindowsWindow::WindowProc(void* hwnd, uint32 uMsg, uint64* wParam, uint64* lParam)
{
	auto windows_window = reinterpret_cast<nWindowsWindow*>(GetWindowLongPtrA(hwnd, GWLP_USERDATA));
	
	switch (uMsg)
	{
	case WM_CLOSE:
		DestroyWindow(hwnd);
		break;
	}
}

nWindowsWindow::nWindowsWindow(const WindowCreateInfo& windowCreateInfo) : nWindow(windowCreateInfo)
{
	WNDCLASSA wndclass;
	wndclass.lpfnWndProc = reinterpret_cast<WNDPROC>(WindowProc);
	wndclass.hInstance = GetModuleHandle(nullptr);
	wndclass.lpszClassName = windowCreateInfo.Name.c_str();
	RegisterClassA(&wndclass);
	
	windowHandle = CreateWindowExA(0, wndclass.lpszClassName, windowCreateInfo.Name.c_str(), WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, windowCreateInfo.Extent.Width, windowCreateInfo.Extent.Height, static_cast<nWindowsWindow*>(windowCreateInfo.ParentWindow)->windowHandle, nullptr, static_cast<HINSTANCE>(static_cast<WindowsWindowData*>(windowCreateInfo.PlatformData)->Instance), nullptr);

	SetWindowLongPtrA(windowHandle, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(this));
}
