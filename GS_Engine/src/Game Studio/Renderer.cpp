#include "Renderer.h"

#include <GLAD/glad.h>
#include <GLFW/glfw3.h>
#include "GL.h"

#include "IBO.h"
#include "VAO.h"
#include "Shader.h"
#include "Uniform.h"

#include "Application.h"

#include "GSM.hpp"

#include "RenderProxy.h"

#include "GBufferPass.h"
#include "LightingPass.h"

Uniform * Projection;
Uniform * View;
Uniform * Model;

Renderer::Renderer(Window * WD) : WindowInstanceRef(WD)
{
	//"Initialize" GLAD.
	GS_ASSERT(gladLoadGLLoader((GLADloadproc)glfwGetProcAddress));

	//Set viewport size.
	GS_GL_CALL(glViewport(0, 0, static_cast<int32>(WindowInstanceRef->GetWindowWidth()), static_cast<int32>(WindowInstanceRef->GetWindowHeight())));

	glEnable(GL_DEPTH_TEST);

	//Set clear color.
	GS_GL_CALL(glClearColor(0.5f, 0.5f, 0.5f, 1.0f));

	GBufferRenderPass = new GBufferPass(this);
}

Renderer::~Renderer()
{
}

void Renderer::RenderFrame() const
{
	GBufferRenderPass->Render();
}

void Renderer::OnUpdate()
{
	//Clear all buffers.
	GS_GL_CALL(glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT));

	ActiveScene.OnUpdate();

	RenderFrame();

	return;
}