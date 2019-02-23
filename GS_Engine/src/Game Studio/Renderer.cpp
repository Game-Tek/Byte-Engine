#include "Renderer.h"

#include <GLAD/glad.h>
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
	GS_GL_CALL(glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT));

	for(uint32 i = 0; i < ActiveScene.StaticMeshList.length(); i++)
	{
		StaticMeshRenderProxy * loc = dynamic_cast<StaticMeshRenderProxy *>(ActiveScene.StaticMeshList[i]->GetRenderProxy());

		RenderFrame(loc->GetIndexBuffer(), loc->GetVertexArray(), Prog);
	}

	return;
}