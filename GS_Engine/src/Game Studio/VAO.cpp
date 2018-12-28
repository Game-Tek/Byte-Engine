#include "VAO.h"

#include <stddef.h>

#include "Vertex.h"

VAO::VAO()
{
	glGenVertexArrays(1, & (unsigned int&) Id);
}


VAO::~VAO()
{
	glDeleteVertexArrays(1, & (unsigned int&) Id);
}

void VAO::Bind()
{
	glBindVertexArray(Id);

	return;
}

void VAO::Enable()
{
	glEnableVertexAttribArray(Id);

	return;
}

void VAO::CreateVertexAttributes()
{
	glVertexAttribPointer(VertexAttributeIndex, 3, GL_FLOAT, GL_FALSE, sizeof(Vector3), (void*)0);

	VertexAttributeIndex++;

	glVertexAttribPointer(VertexAttributeIndex, 2, GL_FLOAT, GL_FALSE, sizeof(TextureCoordinates), (void*)offsetof(Vertex, TextCoord));

	VertexAttributeIndex++;

	return;
}