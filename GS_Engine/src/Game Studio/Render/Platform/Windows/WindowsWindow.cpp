#include "WindowsWindow.h"

#ifdef GS_PLATFORM_WIN
#define GLFW_INCLUDE_VULKAN
#include <GLFW/glfw3.h>
#define GLFW_EXPOSE_NATIVE_WIN32
#include <GLFW/glfw3native.h>
#endif // GS_PLATFORM_WIN

WindowsWindow::WindowsWindow(Extent2D _Extent, const String& _Name) : Window(_Extent)
{
	/* Initialize the library */
	if (!glfwInit())

	/* Create a windowed mode window and its OpenGL context */
	GLFWWindow = glfwCreateWindow(Extent.Width, Extent.Height, _Name.c_str(), NULL, NULL);
	if (!GLFWWindow)
	{
		glfwTerminate();
		GS_ASSERT(false);
	}

	glfwMakeContextCurrent(GLFWWindow);
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
		Extent.Width = X;
		Extent.Height = Y;
	}

	{
		double X, Y;
		glfwGetCursorPos(GLFWWindow, &X, &Y);
		MousePosition.X = X;
		MousePosition.Y = Y;
	}

	ShouldClose = glfwWindowShouldClose(GLFWWindow);

	for (uint8 i = 0; i < MAX_KEYBOARD_KEYS; i++)
	{
		Keys[i] = GLFWKeyStateToKeyState(glfwGetKey(GLFWWindow, KeyboardKeysToGLFWKeys(SCAST(KeyboardKeys, i))));
	}
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
	default:							return 0;
	}
}

KeyState WindowsWindow::GLFWKeyStateToKeyState(int32 _KS)
{
	switch (_KS)
	{
	case GLFW_PRESS:	return KeyState::PRESSED;
	case GLFW_RELEASE:	return KeyState::RELEASED;
	default:			break;
	}
}
