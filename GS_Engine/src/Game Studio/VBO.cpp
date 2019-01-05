#include "VBO.h"

#include "glad.h"
#include "glad.c"

#include "GL.h"

VBO::VBO(const void * Data, unsigned int Size, int Usage)
{
	GS_GL_CALL(glGenBuffers(1, & RendererObjectId));
	Bind();
	GS_GL_CALL(glBufferData(GL_ARRAY_BUFFER, Size, Data, Usage));
}

VBO::~VBO()
{
	GS_GL_CALL(glDeleteBuffers(1, & RendererObjectId));
}

void VBO::Bind() const
{
	GS_GL_CALL(glBindBuffer(GL_ARRAY_BUFFER, RendererObjectId));
}