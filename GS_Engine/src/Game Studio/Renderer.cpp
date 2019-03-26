#include "Renderer.h"

#include <GLAD/glad.h>
#include <GLFW/glfw3.h>
#include "GL.h"

#include "IBO.h"
#include "VAO.h"
#include "Program.h"
#include "Shader.h"
#include "Uniform.h"

#include "Application.h"

#include "GSM.hpp"

#include "RenderProxy.h"

#include "WorldObject.h"

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


	Projection = new Uniform(Prog, "uProjection");
	View = new Uniform(Prog, "uView");
	Model = new Uniform(Prog, "uModel");

	GBufferRenderPass = new GBufferPass();
}

Renderer::~Renderer()
{
	delete Projection;
	delete View;
	delete Model;
}

void Renderer::RenderFrame() const
{
	GBufferRenderPass->SetAsActive();
}

void Renderer::OnUpdate()
{
	//Clear all buffers.
	GS_GL_CALL(glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT));

	ActiveScene.OnUpdate();

	View->Set(*ActiveScene.GetViewMatrix());
	Projection->Set(*ActiveScene.GetProjectionMatrix());

	RenderFrame();

	return;
}