#include "FragmentShader.h"

#include "glad.h"

FragmentShader::FragmentShader()
{
	RendererObjectId = glCreateShader(GL_FRAGMENT_SHADER);							//Tell OpenGL to create a GL_FRAGMENT_SHADER and store the refernce to it in our int.
	glShaderSource(RendererObjectId, 1, &fragmentShaderSource, NULL);				//Tell OpenGL to set the FragmentShader's source code as the code located in vertexShaderSource.
	glCompileShader(RendererObjectId);												//Tell OpenGL to compile the recently input source code, since we need it compiled to attach to the program.
}


FragmentShader::~FragmentShader()
{
	glDeleteShader(RendererObjectId);
}
