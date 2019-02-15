#include "Renderer.h"

#include <GLAD/glad.h>
#include "GL.h"

#include "Vertex.h"
#include "Shader.h"
#include "Program.h"
#include "Texture.h"
#include "Uniform.h"

#include <iostream>

#include "TextureCoordinates.h"

#include "Matrix4.h"
#include "Application.h"

Vertex Vertices[] =	{ { { -0.5f, -0.5f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } }
					, { { -0.5f, 0.5f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 1.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } }
					, { { 0.5f, 0.5f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 1.0f, 1.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } }
					, { { 0.5f, -0.5f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 1.0f, 0.0f }, { 0.0f, 0.0f, 0.0f }, { 0.0f, 0.0f, 0.0f } } };

unsigned int Indices[] = { 0, 1, 2, 2, 3, 0 };

VBO * VertexBuffer;
IBO * IndexBuffer;
VAO * VertexAttribute;
Program * Prog;
Texture * Text;

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

	VertexAttribute->CreateVertexAttribute(3, GL_FLOAT, GL_FALSE, sizeof(Vector3));
	VertexAttribute->CreateVertexAttribute(3, GL_FLOAT, GL_FALSE, sizeof(Vector3));
	VertexAttribute->CreateVertexAttribute(2, GL_FLOAT, GL_FALSE, sizeof(TextureCoordinates));

	View = new Uniform(Prog, "uView");
	Projection = new Uniform(Prog, "uProjection");

	Matrix4 Model, Viewm, Projectionm;

	Viewm.Identity();
	Projectionm = BuildPerspectiveMatrix(1.0f, -1.0f, 0.8f, -0.8f, 0.01f, 100.0f);

	View->Set(Viewm);

	Projection->Set(Viewm);
}

Renderer::~Renderer()
{
	delete VertexBuffer;
	delete IndexBuffer;
	delete VertexAttribute;
	delete Prog;
}

void Renderer::Draw(IBO * ibo, VAO * vao, Program * progr) const
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

	for(uint32 i = 0; i < GS::Application::GetGameInstanceInstance()->GetWorld()->GetEntityList().length(); i++)
	{
		//RenderProxy * loc = GS::Application::GetGameInstanceInstance()->GetWorld()->GetEntityList()[i]->GetRenderProxy();

		//Draw(loc->GetVertexBuffer(), loc->GetIndexBuffer(), VertexAttribute, Prog);
	}

	return;
}