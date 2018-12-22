#include "Window.h"

Window::Window(unsigned short WindowWidth, unsigned short WindowHeight, const char * WindowName) : WNDW_WIDTH(WindowWidth), WNDW_HEIGHT(WindowHeight)
{
	GS_ASSERT(glfwInit())															//Initialize GLFW.
	
	glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 4);									//Set context's max OpenGL version.
	glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 4);									//Set context's min OpenGL version.
	glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);					//Set context's OpenGL profile.

	GLWindow = glfwCreateWindow(WindowWidth, WindowWidth, WindowName, NULL, NULL);	//Create window.

	glfwMakeContextCurrent(GLWindow);
}


Window::~Window()
{
	glfwTerminate();
}
