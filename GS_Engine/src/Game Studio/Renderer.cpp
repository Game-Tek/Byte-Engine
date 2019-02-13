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
	GS_ASSERT(gladLoadGLLoader((GLADloadproc)glfwGetProcAddress));

	GS_GL_CALL(glViewport(0, 0, static_cast<GLsizei>(WindowInstanceRef->GetWindowWidth()), static_cast<GLsizei>(WindowInstanceRef->GetWindowHeight())));
	GS_GL_CALL(glClearColor(0.5f, 0.5f, 0.5f, 1.0f));

	VertexAttribute = new VAO(sizeof(Vertex));
	Prog = new Program("W:/Game Studio/GS_Engine/src/Game Studio/VertexShader.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/FragmentShader.fshader");

	VertexAttribute->CreateVertexAttribute(3, GL_FLOAT, GL_FALSE, sizeof(Vector3));
	VertexAttribute->CreateVertexAttribute(3, GL_FLOAT, GL_FALSE, sizeof(Vector3));
	VertexAttribute->CreateVertexAttribute(2, GL_FLOAT, GL_FALSE, sizeof(TextureCoordinates));
	VertexAttribute->CreateVertexAttribute(3, GL_FLOAT, GL_FALSE, sizeof(Vector3));
	VertexAttribute->CreateVertexAttribute(3, GL_FLOAT, GL_FALSE, sizeof(Vector3));
	
	View = new Uniform(Prog, "uView");
	Projection = new Uniform(Prog, "uProjection");

	Matrix4 Model, Viewm, Projectionm;

	Viewm.Identity();
	Projectionm = BuildPerspectiveMatrix(1.0f, -1.0f, 0.8f, -0.8f, 0.01f, 100.0f);

	View->Set(Viewm);

	Projection->Set(Projectionm);
}

Renderer::~Renderer()
{
	delete VertexBuffer;
	delete IndexBuffer;
	delete VertexAttribute;
	delete Prog;
}

void Renderer::Draw(VBO * vbo, IBO * ibo, VAO * vao, Program * progr) const
{
	vao->Bind();
	vbo->Bind();
	ibo->Bind();
	progr->Bind();

	GS_GL_CALL(glDrawElements(GL_TRIANGLES, static_cast<GLsizei>(ibo->GetCount()), GL_UNSIGNED_INT, 0));

	return;
}

void Renderer::OnUpdate()
{
	GS_GL_CALL(glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT));

	//DrawCalls = times to loop Draw().
	Draw(VertexBuffer, IndexBuffer, VertexAttribute, Prog);							//Perform draw call.

	return;
}