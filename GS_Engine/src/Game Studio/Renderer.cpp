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
#include "StaticMeshRenderProxy.h"

Program * Prog;

Uniform * View;
Uniform * Projection;

Renderer::Renderer(Window * WD) : WindowInstanceRef(WD)
{
	//"Initialize" GLAD.
	GS_ASSERT(gladLoadGLLoader((GLADloadproc)glfwGetProcAddress));

	//Set viewport size.
	GS_GL_CALL(glViewport(0, 0, static_cast<GLsizei>(WindowInstanceRef->GetWindowWidth()), static_cast<GLsizei>(WindowInstanceRef->GetWindowHeight())));

	//Set clear color.
	GS_GL_CALL(glClearColor(0.5f, 0.5f, 0.5f, 1.0f));

	Prog = new Program("W:/Game Studio/GS_Engine/src/Game Studio/VertexShader.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/FragmentShader.fshader");

	View = new Uniform(Prog, "uView");
	Projection = new Uniform(Prog, "uProjection");
}

Renderer::~Renderer()
{
	delete Prog;
	delete View;
	delete Projection;
}

void Renderer::RenderFrame(IBO * ibo, VAO * vao, Program * progr) const
{
	vao->Bind();
	ibo->Bind();
	progr->Bind();

	GS_GL_CALL(glDrawElements(GL_TRIANGLES, ibo->GetCount(), GL_UNSIGNED_INT, nullptr));

	return;
}

void Renderer::OnUpdate()
{
	//Clear all buffers.
	GS_GL_CALL(glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT));

	//Loop through every object to render them.
	for(uint32 i = 0; i < ActiveScene.RenderProxyList.length(); i++)
	{
		//Draw the current object.
		ActiveScene.RenderProxyList[i]->Draw();
	}

	return;
}