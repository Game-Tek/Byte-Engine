#include "Renderer.h"

#include "Window.h"

#include "glad.h"
#include "GL.h"

#include "Vertex.h"
#include "Shader.h"
#include "Program.h"

#include <iostream>

#include "TextureCoordinates.h"

Vertex Vertices[] = { { { -0.5f, -0.5f, 0.0f }, { 0.0f, 0.0f } }, { { 0.5f, -0.5f, 0.0f }, { 0.0f, 0.0f } }, { { 0.0f, 0.5f, 0.0f }, { 0.0f, 0.0f } } };
unsigned int Indices[] = { 0, 1, 2 };

VBO * VertexBuffer;
IBO * IndexBuffer;
VAO * VertexAttribute;
Program * Prog;

Renderer::Renderer(Window * WD) : WindowInstanceRef(WD)
{
	GS_ASSERT(gladLoadGLLoader((GLADloadproc)glfwGetProcAddress));

	GS_GL_CALL(glViewport(0, 0, (GLsizei)WindowInstanceRef->GetWindowWidth(), (GLsizei)WindowInstanceRef->GetWindowHeight()));
	GS_GL_CALL(glClearColor(0.5f, 0.5f, 0.5f, 1.0f));

	VertexBuffer = new VBO(Vertices, sizeof(Vertices), GL_STATIC_DRAW);
	IndexBuffer = new IBO(Indices, 3);
	VertexAttribute = new VAO();
	Prog = new Program();

	VertexAttribute->CreateVertexAttribute<Vertex>(3, GL_FLOAT, GL_FALSE, (void*)0);
	VertexAttribute->CreateVertexAttribute<Vertex>(2, GL_FLOAT, GL_FALSE, (void*)sizeof(Vector3));
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

	GS_GL_CALL(glDrawElements(GL_TRIANGLES, (GLsizei)ibo->GetCount(), GL_UNSIGNED_INT, 0));

	return;
}

void Renderer::OnUpdate()
{
	GS_GL_CALL(glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT));

	//DrawCalls = times to loop Draw().
	Draw(VertexBuffer, IndexBuffer, VertexAttribute, Prog);							//Perform draw call.

	return;
}