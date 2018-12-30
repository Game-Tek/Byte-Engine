#include "VAO.h"

#include <stddef.h>

#include "glad.h"

#include "Vertex.h"


VAO::VAO()
{
	glGenVertexArrays(1, & RendererObjectId);
}


VAO::~VAO()
{
	glDeleteVertexArrays(1, & RendererObjectId);
}

void VAO::Bind() const
{
	glBindVertexArray(RendererObjectId);

	return;
}

void VAO::Enable() const
{
	glEnableVertexAttribArray(RendererObjectId);

	return;
}

void VAO::CreateVertexAttributes()
{
	glVertexAttribPointer(VertexAttributeIndex, 3, GL_FLOAT, GL_FALSE, sizeof(Vector3), (void*)0);

	VertexAttributeIndex++;

	return;
}