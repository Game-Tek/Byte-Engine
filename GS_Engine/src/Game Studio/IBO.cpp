#include "IBO.h"

#include "glad.h"

IBO::IBO()
{
	glGenBuffers(1, & RendererObjectId);
	//glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(indices), indices, Usage);
}


IBO::~IBO()
{
	glDeleteBuffers(1, & RendererObjectId);
}

void IBO::Bind(int Usage) const
{
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, RendererObjectId);
}
