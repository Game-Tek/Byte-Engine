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

Program * Prog;

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

	Prog = new Program("W:/Game Studio/GS_Engine/src/Game Studio/VertexShader.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/FragmentShader.fshader");

	Projection = new Uniform(Prog, "uProjection");
	View = new Uniform(Prog, "uView");
	Model = new Uniform(Prog, "uModel");
}

Renderer::~Renderer()
{
	delete Prog;
	delete Projection;
	delete View;
	delete Model;
}

void Renderer::RenderFrame() const
{
	//Loop through every object to render them.
	for (uint32 i = 0; i < ActiveScene.RenderProxyList.length(); i++)
	{
		Model->Set(GSM::Translation(ActiveScene.RenderProxyList[i]->GetOwner()->GetPosition()));

		//Draw the current object.
		ActiveScene.RenderProxyList[i]->Draw();
	}
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