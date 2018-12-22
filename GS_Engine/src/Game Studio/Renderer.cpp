#include "Renderer.h"

#include "GLAD\glad.h"
#include "GLFW\glfw3.h"

Renderer::Renderer()
{
	GS_ASSERT(gladLoadGLLoader((GLADloadproc)glfwGetProcAddress));
}

Renderer::~Renderer()
{
}

void Renderer::Update(float DeltaTime)
{
}