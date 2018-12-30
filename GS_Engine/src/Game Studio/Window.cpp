#include "Window.h"

Window::Window(unsigned short WindowWidth, unsigned short WindowHeight, const char * WindowName) : WNDW_WIDTH(WindowWidth), WNDW_HEIGHT(WindowHeight)
{
	GS_ASSERT(glfwInit());															//Initialize GLFW.
	
	glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 4);									//Set context's max OpenGL version.
	glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 4);									//Set context's min OpenGL version.
	glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);					//Set context's OpenGL profile.

	GLWindow = glfwCreateWindow(WindowWidth, WindowHeight, WindowName, NULL, NULL);	//Create window.

	glfwMakeContextCurrent(GLWindow);												//Make the recently created window the current context.

	glfw
}


Window::~Window()
{
	glfwTerminate();																//Tells GLFW to remove all of it's allocated resources.
}

void Window::SetVsync(bool Enable)
{
	glfwSwapInterval(Enable ? 1 : 0);												//Set the swap interval to unlimited framerate (0) or in sync with the screen (1).
	return;
}

GLFWwindow * Window::GetGLFWWindow()
{
	return GLWindow;
}
