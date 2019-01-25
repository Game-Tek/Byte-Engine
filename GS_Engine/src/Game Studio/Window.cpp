#include "Window.h"

#include "InputManager.h"

Window::Window(unsigned short WindowWidth, unsigned short WindowHeight, const char * WindowName) : WindowWidth(WindowWidth), WindowHeight(WindowHeight)
{
	GS_ASSERT(glfwInit());															//Initialize GLFW.
	
	glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);									//Set context's max OpenGL version.
	glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 3);									//Set context's min OpenGL version.
	glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);					//Set context's OpenGL profile.

	GLWindow = glfwCreateWindow(WindowWidth, WindowHeight, WindowName, NULL, NULL);	//Create window.

	glfwMakeContextCurrent(GLWindow);												//Make the recently created window the current context.
}


Window::~Window()
{
	glfwTerminate();																//Tells GLFW to remove all of it's allocated resources.
}

void Window::OnUpdate()
{
	glfwPollEvents();

	glfwSwapBuffers(GLWindow);

	int WState = glfwGetKey(GLWindow, GLFW_KEY_W);
	if (WState == GLFW_PRESS) InputManager::KeyPressed(W);

	int AState = glfwGetKey(GLWindow, GLFW_KEY_A);
	if (AState == GLFW_PRESS) InputManager::KeyPressed(A);

	int SState = glfwGetKey(GLWindow, GLFW_KEY_S);
	if (SState == GLFW_PRESS) InputManager::KeyPressed(S);

	int DState = glfwGetKey(GLWindow, GLFW_KEY_D);
	if (DState == GLFW_PRESS) InputManager::KeyPressed(D);

	double MousePositionX;
	double MousePositionY;

	glfwGetCursorPos(GLWindow, &MousePositionX, &MousePositionY);
	InputManager::MouseMoved(Vector2((float)MousePositionX, (float)MousePositionY));
}

void Window::SetVsync(bool Enable)
{
	glfwSwapInterval(Enable);														//Set the swap interval to unlimited framerate (0) or in sync with the screen (1).
	return;
}