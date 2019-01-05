#include "Shader.h"

#include <iostream>
#include <string>
#include <fstream>
#include <sstream>

#include "glad.h"

#include "GL.h"

Shader::Shader(unsigned int ShaderType, const char * ShaderPath)
{
	std::string Source = ReadShader(ShaderPath);
	const char * ShaderSource = Source.c_str();

	RendererObjectId = GS_GL_CALL(glCreateShader(ShaderType));					//Tell OpenGL to create a shader and store the refernce to it in our int.
	GS_GL_CALL(glShaderSource(RendererObjectId, 1, & ShaderSource, NULL));		//Tell OpenGL to set the shader's source code as the code located in ShaderPath.
	GS_GL_CALL(glCompileShader(RendererObjectId));								//Tell OpenGL to compile the recently input source code, since we need it compiled to attach to the program.

	std::cout << ShaderSource << std::endl;
}

Shader::~Shader()
{
	GS_GL_CALL(glDeleteShader(RendererObjectId));
}

std::string Shader::ReadShader(const char * Path)
{
	std::ifstream File;
	std::stringstream Stream;
	std::string Code;

	File.open(Path);

	Stream << File.rdbuf();

	File.close();

	Code = Stream.str();

	if (Code.empty())
	{
		std::cout << "Failed to load shader!" << std::endl;
	}

	return Code;
}
