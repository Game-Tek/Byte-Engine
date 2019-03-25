#include "Window.h"

#include <GLFW/glfw3.h>

#include "InputManager.h"
#include "Application.h"
#include "ImageSize.h"

Window::Window(const uint16 WindowWidth, const uint16 WindowHeight, const char * WindowName) : WindowWidth(WindowWidth), WindowHeight(WindowHeight)
{
	GS_ASSERT(glfwInit());														//Initialize GLFW.
	
	glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);								//Set context's max OpenGL version.
	glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 3);								//Set context's min OpenGL version.
	glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);				//Set context's OpenGL profile.

	GLWindow = glfwCreateWindow(WindowWidth, WindowHeight, WindowName, 0, 0);	//Create window.

	glfwMakeContextCurrent(GLWindow);											//Make the recently created window the current context.
}

Window::~Window()
{
	glfwTerminate();															//Tells GLFW to remove all of it's allocated resources.
}

void Window::OnUpdate()
{
	glfwPollEvents();

	glfwSwapBuffers(GLWindow);

	glfwGetWindowSize(GLWindow, &reinterpret_cast<int&>(WindowWidth), &reinterpret_cast<int&>(WindowHeight));

	int WState = glfwGetKey(GLWindow, GLFW_KEY_W);
	if (WState == GLFW_PRESS) GS::Application::Get()->GetInputManagerInstance()->KeyPressed(W);

	int AState = glfwGetKey(GLWindow, GLFW_KEY_A);
	if (AState == GLFW_PRESS) GS::Application::Get()->GetInputManagerInstance()->KeyPressed(A);

	int SState = glfwGetKey(GLWindow, GLFW_KEY_S);
	if (SState == GLFW_PRESS) GS::Application::Get()->GetInputManagerInstance()->KeyPressed(S);

	int DState = glfwGetKey(GLWindow, GLFW_KEY_D);
	if (DState == GLFW_PRESS) GS::Application::Get()->GetInputManagerInstance()->KeyPressed(D);

	double MousePositionX;
	double MousePositionY;

	glfwGetCursorPos(GLWindow, &MousePositionX, &MousePositionY);
	GS::Application::Get()->GetInputManagerInstance()->MouseMoved(Vector2(static_cast<float>(MousePositionX), static_cast<float>(MousePositionY)));
}

void Window::SetVsync(const bool Enable) const
{
	glfwSwapInterval(Enable);	//Set the swap interval to unlimited framerate (0) or in sync with the screen (1).

	return;
}

ImageSize Window::GetWindowSize() const
{
	return ImageSize(WindowWidth, WindowHeight);
}

void Window::ResizeWindow(uint16 WWidth, uint16 WHeight)
{
	WindowWidth  = WWidth;
	WindowHeight = WHeight;

	return;
}
