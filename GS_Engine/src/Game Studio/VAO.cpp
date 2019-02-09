#include "VAO.h"

#include "GLAD/glad.h"

#include "GL.h"

#include "Vertex.h"

VAO::VAO(size_t VertexSize) : VertexSize(VertexSize)
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

void VAO::CreateVertexAttribute(int NOfElementsInThisAttribute, unsigned int DataType, unsigned char Normalize, size_t AttributeSize)
{
	GS_GL_CALL(glVertexAttribPointer((unsigned int)VertexAttributeIndex, NOfElementsInThisAttribute, DataType, Normalize, VertexSize, (void*)Offset));
	GS_GL_CALL(glEnableVertexAttribArray((unsigned int)VertexAttributeIndex));

	VertexAttributeIndex++;

	Offset += AttributeSize;

	return;
}