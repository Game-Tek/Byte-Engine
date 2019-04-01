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

#include "GBufferRenderPass.h"
#include "LightRenderPass.h"

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

	GBufferPass = new GBufferRenderPass(this);
	LightingRenderPass = new LightRenderPass(this);
}

Renderer::~Renderer()
{
}

void Renderer::RenderFrame() const
{
	GBufferPass->Render();
	LightingRenderPass->Render();
}

void Renderer::OnUpdate()
{
	ActiveScene.OnUpdate();

	RenderFrame();

	return;
}