#include "VBO.h"

#include "glad.h"

VBO::VBO()
{
	glGenBuffers(1, & RendererObjectId);
	//glBufferData(GL_ARRAY_BUFFER, sizeof(vertices), vertices, Usage);
}


VBO::~VBO()
{
	glDeleteBuffers(1, & RendererObjectId);
}

void VBO::Bind(int Usage) const
{
	glBindBuffer(GL_ARRAY_BUFFER, RendererObjectId);
}