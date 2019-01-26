#include "Renderer.h"

#include "Window.h"

#include "glad.h"
#include "GL.h"

#include "Vertex.h"
#include "Shader.h"
#include "Program.h"
#include "Texture.h"

#include <iostream>

#include "TextureCoordinates.h"

#include "Matrix4.h"

#include "GSM.hpp"

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

Renderer::Renderer(Window * WD) : WindowInstanceRef(WD)
{
	GS_ASSERT(gladLoadGLLoader((GLADloadproc)glfwGetProcAddress));

	GS_GL_CALL(glViewport(0, 0, (GLsizei)WindowInstanceRef->GetWindowWidth(), (GLsizei)WindowInstanceRef->GetWindowHeight()));
	GS_GL_CALL(glClearColor(0.5f, 0.5f, 0.5f, 1.0f));

	VertexBuffer = new VBO(Vertices, sizeof(Vertices), GL_STATIC_DRAW);

	IndexBuffer = new IBO(Indices, 6);
	VertexAttribute = new VAO(sizeof(Vertex));
	Prog = new Program("W:/Game Studio/GS_Engine/src/Game Studio/VertexShader.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/FragmentShader.fshader");
	Text = new Texture("W:/Game Studio/bin/Sandbox/Debug-x64/texture.png");

	VertexAttribute->CreateVertexAttribute(3, GL_FLOAT, GL_FALSE, sizeof(Vector3));
	VertexAttribute->CreateVertexAttribute(3, GL_FLOAT, GL_FALSE, sizeof(Vector3));
	VertexAttribute->CreateVertexAttribute(2, GL_FLOAT, GL_FALSE, sizeof(TextureCoordinates));

	Text->Bind();
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