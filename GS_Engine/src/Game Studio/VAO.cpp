#include "VAO.h"

#include "GL.h"

#include "glad.h"

#include "Vertex.h"

VAO::VAO()
{
	GS_GL_CALL(glGenVertexArrays(1, & RendererObjectId));
	Bind();
}

VAO::~VAO()
{
	GS_GL_CALL(glDeleteVertexArrays(1, & RendererObjectId));
}

void VAO::Bind() const
{
	GS_GL_CALL(glBindVertexArray(RendererObjectId));

	return;
}

template <typename VertexType>
void VAO::CreateVertexAttribute(int NumberOfElementsInThisAttribute, int DataType, int Normalize, void * Offset)
{
	GS_GL_CALL(glVertexAttribPointer((GLuint)VertexAttributeIndex, (GLint)NumberOfElementsInThisAttribute, (GLenum)DataType, (GLboolean)Normalize, sizeof(VertexType), Offset));
	GS_GL_CALL(glEnableVertexAttribArray((GLuint)VertexAttributeIndex));

	VertexAttributeIndex++;

	return;
}