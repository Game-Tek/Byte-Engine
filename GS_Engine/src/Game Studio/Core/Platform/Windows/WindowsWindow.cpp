#include "WindowsWindow.h"

#include "WindowsApplication.h"

uint64 nWindowsWindow::WindowProc(const HWND hwnd, const UINT uMsg, WPARAM wParam, LPARAM lParam)
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
	WNDCLASSA wndclass{};
	wndclass.lpfnWndProc = reinterpret_cast<WNDPROC>(WindowProc);
	wndclass.hInstance = static_cast<WindowsApplication*>(windowCreateInfo.Application)->GetInstance();
	wndclass.lpszClassName = windowCreateInfo.Name.c_str();
	RegisterClassA(&wndclass);
	
	windowHandle = CreateWindowExA(0, wndclass.lpszClassName, windowCreateInfo.Name.c_str(), WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, windowCreateInfo.Extent.Width, windowCreateInfo.Extent.Height, static_cast<nWindowsWindow*>(windowCreateInfo.ParentWindow)->windowHandle, nullptr, static_cast<WindowsApplication*>(windowCreateInfo.Application)->GetInstance(), nullptr);

	SetWindowLongPtrA(windowHandle, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(this));

	//ShowWindow(windowHandle, SW_SHOWNORMAL);
}

void nWindowsWindow::SetState(const WindowState windowState)
{
	switch (windowState)
	{
	case WindowState::MAXIMIZED: ShowWindow(windowHandle, SW_SHOWMAXIMIZED); break;
	case WindowState::FULLSCREEN:
		DWORD dwStyle = ::GetWindowLong(windowHandle, GWL_STYLE);
		DWORD dwRemove = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
		DWORD dwNewStyle = dwStyle & ~dwRemove;
		SetWindowLongPtrA(windowHandle, GWL_STYLE, dwNewStyle);
		SetWindowPos(windowHandle, HWND_TOP, 0, 0, 500, 500, SWP_FRAMECHANGED);
		ShowWindow(windowHandle, SW_SHOWMAXIMIZED);
		break;
	case WindowState::MINIMIZED: ShowWindow(windowHandle, SW_MINIMIZE); break;
	}
}
