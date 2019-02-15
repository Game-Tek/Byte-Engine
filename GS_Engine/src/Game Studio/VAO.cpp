#include "VAO.h"

#include "GLAD/glad.h"

#include "GL.h"

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

void VAO::CreateVertexAttribute(uint8 NOfElementsInThisAttribute, uint32 DataType, uint8 Normalize, size_t AttributeSize)
{
	GS_GL_CALL(glEnableVertexAttribArray(static_cast<uint32>(VertexAttributeIndex)));
	GS_GL_CALL(glVertexAttribPointer(static_cast<uint32>(VertexAttributeIndex), static_cast<uint32>(NOfElementsInThisAttribute), DataType, Normalize, VertexSize, 0/*reinterpret_cast<void*>(Offset)*/));

	//Increment index count so when the next attribute is created it has the index corresponding to the next one.
	VertexAttributeIndex++;

	//Add this attribute's size to offset so when the next attribute is created it has the offset corresponding to the next one.
	Offset += AttributeSize;

	return;
}