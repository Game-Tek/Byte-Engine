#include "Renderer.h"

#include "Window.h"

Renderer::Renderer(Window * WD) : WindowInstanceRef(WD)
{
	GS_ASSERT(gladLoadGLLoader((GLADloadproc)glfwGetProcAddress));
}

Renderer::~Renderer()
{
}

void Renderer::Draw()
{
	
}

void Renderer::OnUpdate(float DeltaTime)
{
	glClearColor(0.2f, 0.3f, 0.3f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);

	//DrawCalls = times to loop Draw().
	Draw();													//Perform draw call.

	glfwSwapBuffers(WindowInstanceRef->GetGLFWWindow());
}