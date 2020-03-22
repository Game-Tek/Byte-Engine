#include "WindowsWindow.h"

#include "WindowsApplication.h"

uint64 nWindowsWindow::WindowProc(const HWND hwnd, const UINT uMsg, const WPARAM wParam, const LPARAM lParam)
{
	const auto windows_window = reinterpret_cast<nWindowsWindow*>(GetWindowLongPtrA(hwnd, GWLP_USERDATA));

	switch (uMsg)
	{
	case WM_CLOSE:       windows_window->onCloseDelegate();	return 0;
	case WM_MOUSEMOVE:   windows_window->onMouseMove(CalculateMousePos(LOWORD(lParam), HIWORD(lParam))); return 0;
	case WM_MOUSEHWHEEL: windows_window->onMouseWheelMove(GET_WHEEL_DELTA_WPARAM(wParam)); return 0;
	case WM_LBUTTONDOWN: windows_window->onMouseButtonClick(MouseButton::LEFT_BUTTON,   MouseButtonState::PRESSED);  return 0;
	case WM_LBUTTONUP:   windows_window->onMouseButtonClick(MouseButton::LEFT_BUTTON,   MouseButtonState::RELEASED); return 0;
	case WM_RBUTTONDOWN: windows_window->onMouseButtonClick(MouseButton::RIGHT_BUTTON,  MouseButtonState::PRESSED);  return 0;
	case WM_RBUTTONUP:   windows_window->onMouseButtonClick(MouseButton::RIGHT_BUTTON,  MouseButtonState::RELEASED); return 0;
	case WM_MBUTTONDOWN: windows_window->onMouseButtonClick(MouseButton::MIDDLE_BUTTON, MouseButtonState::PRESSED);  return 0;
	case WM_MBUTTONUP:   windows_window->onMouseButtonClick(MouseButton::MIDDLE_BUTTON, MouseButtonState::RELEASED); return 0;
	case WM_KEYDOWN:     windows_window->onKeyEvent(wParam, KeyboardKeyState::PRESSED);  return 0;
	case WM_KEYUP:       windows_window->onKeyEvent(wParam, KeyboardKeyState::RELEASED); return 0;
	case WM_SIZE:        windows_window->onWindowResize(Vector2(LOWORD(lParam), HIWORD(lParam))); return 0;
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
	{
		DWORD dwStyle = ::GetWindowLong(windowHandle, GWL_STYLE);
		DWORD dwRemove = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
		DWORD dwNewStyle = dwStyle & ~dwRemove;
		SetWindowLongPtrA(windowHandle, GWL_STYLE, dwNewStyle);
		SetWindowPos(windowHandle, HWND_TOP, 0, 0, 500, 500, SWP_FRAMECHANGED);
		ShowWindow(windowHandle, SW_SHOWMAXIMIZED);
	}
		break;
	case WindowState::MINIMIZED: ShowWindow(windowHandle, SW_MINIMIZE); break;
	}
}
