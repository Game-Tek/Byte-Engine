#include "WindowsWindow.h"
#include "Debug/Logger.h"

#ifdef GS_PLATFORM_WIN
#define GLFW_INCLUDE_VULKAN
#include <GLFW/glfw3.h>
#define GLFW_EXPOSE_NATIVE_WIN32
#include <GLFW/glfw3native.h>
#endif // GS_PLATFORM_WIN

static float ScrollValue;

void scroll_callback(GLFWwindow* window, double xoffset, double yoffset)
{
	ScrollValue = yoffset;
}

WindowsWindow::WindowsWindow(const WindowCreateInfo& _WCI) : Window(_WCI.Extent, _WCI.WindowType)
{
	glfwInit();

	/* Create a windowed mode window and its OpenGL context */
	glfwWindowHint(GLFW_CLIENT_API, GLFW_NO_API);
	glfwWindowHint(GLFW_DECORATED, _WCI.IsDecorated);
	GLFWWindow = glfwCreateWindow(Extent.Width, Extent.Height, _WCI.Name.c_str(), nullptr, nullptr);

	if (!GLFWWindow)
	{
		const char* Error = nullptr;
		glfwGetError(&Error);
		glfwTerminate();
		GS_BASIC_LOG_ERROR("Window creation failed, Reason: %s", Error)
	}

	WindowObject = glfwGetWin32Window(GLFWWindow);
	WindowInstance = GetModuleHandle(nullptr);

	//SetWindowFit(_Fit);

	for (uint8 i = 0; i < 4; ++i)
	{
		JoystickCount += glfwJoystickPresent(i);
	}

	glfwSetScrollCallback(GLFWWindow, scroll_callback);
}

WindowsWindow::~WindowsWindow()
{
	glfwTerminate();
}

void WindowsWindow::Update()
{
	glfwPollEvents();

	{
		int32 X, Y;
		glfwGetWindowSize(GLFWWindow, &X, &Y);
		Extent.Width = SCAST(uint16, X);
		Extent.Height = SCAST(uint16, Y);
	}

	{
		double X, Y;
		glfwGetCursorPos(GLFWWindow, &X, &Y);
		WindowMouseState.MousePosition.X = SCAST(float, (X - Extent.Width / 2) / Extent.Width * 2);
		WindowMouseState.MousePosition.Y = SCAST(float, (Y - Extent.Height / 2) / Extent.Height * -2);

		WindowMouseState.MouseWheelMove = ScrollValue;
	}

	{
		const auto RightMouseButtonState = glfwGetMouseButton(GLFWWindow, GLFW_MOUSE_BUTTON_RIGHT);
		const auto LeftMouseButtonState = glfwGetMouseButton(GLFWWindow, GLFW_MOUSE_BUTTON_LEFT);
		const auto MiddleMouseButtonState = glfwGetMouseButton(GLFWWindow, GLFW_MOUSE_BUTTON_MIDDLE);

		WindowMouseState.IsRightButtonPressed = RightMouseButtonState;
		WindowMouseState.IsLeftButtonPressed = LeftMouseButtonState;
		WindowMouseState.IsMouseWheelPressed = MiddleMouseButtonState;
	}

	ShouldClose = glfwWindowShouldClose(GLFWWindow);

	for (uint8 i = 0; i < MAX_KEYBOARD_KEYS; ++i)
	{
		Keys[i] = glfwGetKey(GLFWWindow, KeyboardKeysToGLFWKeys(SCAST(KeyboardKeys, i)));
	}

	for (uint8 i = 0; i < JoystickCount; ++i)
	{
		GLFWgamepadstate GamepadState;

		glfwGetGamepadState(i, &GamepadState);

		JoystickStates[i].RightJoystickPosition.X = GamepadState.axes[GLFW_GAMEPAD_AXIS_RIGHT_X];
		JoystickStates[i].RightJoystickPosition.Y = GamepadState.axes[GLFW_GAMEPAD_AXIS_RIGHT_Y];
		JoystickStates[i].LeftJoystickPosition.X = GamepadState.axes[GLFW_GAMEPAD_AXIS_LEFT_X];
		JoystickStates[i].LeftJoystickPosition.Y = GamepadState.axes[GLFW_GAMEPAD_AXIS_LEFT_Y];
		JoystickStates[i].RightTriggerDepth = GamepadState.axes[GLFW_GAMEPAD_AXIS_RIGHT_TRIGGER];
		JoystickStates[i].LeftTriggerDepth = GamepadState.axes[GLFW_GAMEPAD_AXIS_LEFT_TRIGGER];

		JoystickStates[i].IsRightBumperPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_RIGHT_BUMPER];
		JoystickStates[i].IsLeftBumperPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_LEFT_BUMPER];

		JoystickStates[i].IsUpFaceButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_Y];
		JoystickStates[i].IsRightFaceButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_B];
		JoystickStates[i].IsBottomFaceButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_A];
		JoystickStates[i].IsLeftFaceButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_X];

		JoystickStates[i].IsUpDPadButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_DPAD_UP];
		JoystickStates[i].IsRightDPadButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_DPAD_RIGHT];
		JoystickStates[i].IsDownDPadButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_DPAD_DOWN];
		JoystickStates[i].IsLeftDPadButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_DPAD_LEFT];

		JoystickStates[i].IsRightStickPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_RIGHT_THUMB];
		JoystickStates[i].IsLeftStickPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_LEFT_THUMB];

		JoystickStates[i].IsRightMenuButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_START];
		JoystickStates[i].IsLeftMenuButtonPressed = GamepadState.buttons[GLFW_GAMEPAD_BUTTON_BACK];
	}

	ScrollValue = 0;
}

void WindowsWindow::SetWindowFit(WindowFit _Fit)
{
	const GLFWvidmode* mode = glfwGetVideoMode(glfwGetPrimaryMonitor());

	switch (_Fit)
	{
	case WindowFit::NORMAL:		glfwRestoreWindow(GLFWWindow);
	case WindowFit::MAXIMIZED:	glfwMaximizeWindow(GLFWWindow);
	case WindowFit::FULLSCREEN: glfwSetWindowMonitor(GLFWWindow, glfwGetPrimaryMonitor(), 0, 0, mode->width, mode->height, mode->refreshRate);
	default: ;
	}
}

void WindowsWindow::SetWindowResolution(Extent2D _Res)
{
	glfwSetWindowSize(GLFWWindow, _Res.Width, _Res.Height);
}

void WindowsWindow::SetWindowIcon(const WindowIconInfo& _WII)
{
	GLFWimage Image;
	Image.width = _WII.Size.Width;
	Image.height = _WII.Size.Height;
	Image.pixels = SCAST(uint8*, _WII.Data);
	glfwSetWindowIcon(GLFWWindow, 1, &Image);
}

void WindowsWindow::MinimizeWindow()
{
	glfwIconifyWindow(GLFWWindow);
}

void WindowsWindow::NotifyWindow()
{
	glfwRequestWindowAttention(GLFWWindow);
}

void WindowsWindow::FocusWindow()
{
	glfwFocusWindow(GLFWWindow);
}

void WindowsWindow::SetWindowTitle(const char* _Title)
{
	glfwSetWindowTitle(GLFWWindow, _Title);
}

Extent2D WindowsWindow::GetFramebufferSize()
{
	int Width, Height;
	glfwGetFramebufferSize(GLFWWindow, &Width, &Height);
	return Extent2D(Width, Height);
}

Vector2 WindowsWindow::GetContentScale()
{
	Vector2 Return;
	glfwGetWindowContentScale(GLFWWindow, &Return.X, &Return.Y);
	return Return;
}

int32 WindowsWindow::KeyboardKeysToGLFWKeys(KeyboardKeys _IE)
{
	switch (_IE)
	{
	case KeyboardKeys::Q:				return GLFW_KEY_Q;
	case KeyboardKeys::W:				return GLFW_KEY_W;
	case KeyboardKeys::E:				return GLFW_KEY_E;
	case KeyboardKeys::R:				return GLFW_KEY_R;
	case KeyboardKeys::T:				return GLFW_KEY_T;
	case KeyboardKeys::Y:				return GLFW_KEY_Y;
	case KeyboardKeys::U:				return GLFW_KEY_U;
	case KeyboardKeys::I:				return GLFW_KEY_I;
	case KeyboardKeys::O:				return GLFW_KEY_O;
	case KeyboardKeys::P:				return GLFW_KEY_P;
	case KeyboardKeys::A:				return GLFW_KEY_A;
	case KeyboardKeys::S:				return GLFW_KEY_S;
	case KeyboardKeys::D:				return GLFW_KEY_D;
	case KeyboardKeys::F:				return GLFW_KEY_F;
	case KeyboardKeys::G:				return GLFW_KEY_G;
	case KeyboardKeys::H:				return GLFW_KEY_H;
	case KeyboardKeys::J:				return GLFW_KEY_J;
	case KeyboardKeys::K:				return GLFW_KEY_K;
	case KeyboardKeys::L:				return GLFW_KEY_L;
	case KeyboardKeys::Z:				return GLFW_KEY_Z;
	case KeyboardKeys::X:				return GLFW_KEY_X;
	case KeyboardKeys::C:				return GLFW_KEY_C;
	case KeyboardKeys::V:				return GLFW_KEY_V;
	case KeyboardKeys::B:				return GLFW_KEY_B;
	case KeyboardKeys::N:				return GLFW_KEY_N;
	case KeyboardKeys::M:				return GLFW_KEY_M;
	case KeyboardKeys::Keyboard0:		return GLFW_KEY_0;
	case KeyboardKeys::Keyboard1:		return GLFW_KEY_1;
	case KeyboardKeys::Keyboard2:		return GLFW_KEY_2;
	case KeyboardKeys::Keyboard3:		return GLFW_KEY_3;
	case KeyboardKeys::Keyboard4:		return GLFW_KEY_4;
	case KeyboardKeys::Keyboard5:		return GLFW_KEY_5;
	case KeyboardKeys::Keyboard6:		return GLFW_KEY_6;
	case KeyboardKeys::Keyboard7:		return GLFW_KEY_7;
	case KeyboardKeys::Keyboard8:		return GLFW_KEY_8;
	case KeyboardKeys::Keyboard9:		return GLFW_KEY_9;
	case KeyboardKeys::Enter:			return GLFW_KEY_ENTER;
	case KeyboardKeys::Tab:				return GLFW_KEY_TAB;
	case KeyboardKeys::Esc:				return GLFW_KEY_ESCAPE;
	case KeyboardKeys::RShift:			return GLFW_KEY_RIGHT_SHIFT;
	case KeyboardKeys::LShift:			return GLFW_KEY_LEFT_SHIFT;
	case KeyboardKeys::RControl:		return GLFW_KEY_RIGHT_CONTROL;
	case KeyboardKeys::LControl:		return GLFW_KEY_LEFT_CONTROL;
	case KeyboardKeys::Alt:				return GLFW_KEY_LEFT_ALT;
	case KeyboardKeys::AltGr:			return GLFW_KEY_RIGHT_ALT;
	case KeyboardKeys::UpArrow:			return GLFW_KEY_UP;
	case KeyboardKeys::RightArrow:		return GLFW_KEY_RIGHT;
	case KeyboardKeys::DownArrow:		return GLFW_KEY_DOWN;
	case KeyboardKeys::LeftArrow:		return GLFW_KEY_LEFT;
	case KeyboardKeys::SpaceBar:		return GLFW_KEY_SPACE;
	case KeyboardKeys::Numpad0:			return GLFW_KEY_KP_0;
	case KeyboardKeys::Numpad1:			return GLFW_KEY_KP_1;
	case KeyboardKeys::Numpad2:			return GLFW_KEY_KP_2;
	case KeyboardKeys::Numpad3:			return GLFW_KEY_KP_3;
	case KeyboardKeys::Numpad4:			return GLFW_KEY_KP_4;
	case KeyboardKeys::Numpad5:			return GLFW_KEY_KP_5;
	case KeyboardKeys::Numpad6:			return GLFW_KEY_KP_6;
	case KeyboardKeys::Numpad7:			return GLFW_KEY_KP_7;
	case KeyboardKeys::Numpad8:			return GLFW_KEY_KP_8;
	case KeyboardKeys::Numpad9:			return GLFW_KEY_KP_9;
	case KeyboardKeys::F1:				return GLFW_KEY_F1;
	case KeyboardKeys::F2:				return GLFW_KEY_F2;
	case KeyboardKeys::F3:				return GLFW_KEY_F3;
	case KeyboardKeys::F4:				return GLFW_KEY_F4;
	case KeyboardKeys::F5:				return GLFW_KEY_F5;
	case KeyboardKeys::F6:				return GLFW_KEY_F6;
	case KeyboardKeys::F7:				return GLFW_KEY_F7;
	case KeyboardKeys::F8:				return GLFW_KEY_F8;
	case KeyboardKeys::F9:				return GLFW_KEY_F9;
	case KeyboardKeys::F10:				return GLFW_KEY_F10;
	case KeyboardKeys::F11:				return GLFW_KEY_F11;
	case KeyboardKeys::F12:				return GLFW_KEY_F12;
	default:							return GLFW_KEY_UNKNOWN;
	}
}

KeyState WindowsWindow::GLFWKeyStateToKeyState(int32 _KS)
{
	switch (_KS)
	{
	case GLFW_PRESS:	return KeyState::PRESSED;
	case GLFW_RELEASE:	return KeyState::RELEASED;
	default:			return KeyState::NONE;
	}

	return {};
}
