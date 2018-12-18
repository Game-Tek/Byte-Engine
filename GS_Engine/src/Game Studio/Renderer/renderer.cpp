#include "Renderer.h"

#include <GLAD\glad.h>
#include "GLFW\glfw3.h"

#include "..\Logger.h"

const unsigned int SCR_WIDTH = 1280;
const unsigned int SCR_HEIGHT = 720;

Renderer::Renderer()
{
	//Initialize GLFW.
	glfwInit();
	glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 4);
	glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 4);
	glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);

	//Create GLFW window.
	GLFWwindow * Window = glfwCreateWindow(SCR_WIDTH, SCR_HEIGHT, "My OpenGL Renderer", NULL, NULL);
	if (Window == NULL)
	{
		GS_LOG_ERROR("Failed to create Window!");
		glfwTerminate();
	}
	glfwMakeContextCurrent(Window);

	//Initialize GLAD.
	if (!gladLoadGLLoader((GLADloadproc)glfwGetProcAddress))
	{
		GS_LOG_ERROR("Failed to initialize GLAD");
	}
}

Renderer::~Renderer()
{
}