#include "VertexShader.h"

#include "glad.h"

VertexShader::VertexShader()
{
	RendererObjectId = glCreateShader(GL_VERTEX_SHADER);					//Tell OpenGL to create a GL_VERTEX_SHADER and store the refernce to it in our int.
	glShaderSource(RendererObjectId, 1, &vertexShaderSource, NULL);			//Tell OpenGL to set the VertexShader's source code as the code located in vertexShaderSource.
	glCompileShader(RendererObjectId);										//Tell OpenGL to compile the recently input source code, since we need it compiled to attach to the program.
}


VertexShader::~VertexShader()
{
	glDeleteShader(RendererObjectId);
}
