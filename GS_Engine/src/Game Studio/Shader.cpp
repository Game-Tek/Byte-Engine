#include "Shader.h"

#include "Logger.h"
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

	GLint success = 0;
	glGetShaderiv(RendererObjectId, GL_COMPILE_STATUS, &success);

	if (success)
	{
		GS_LOG_SUCCESS("Shader at %s, compiled succesfuly.", ShaderPath)
	}
	else
	{
		GS_LOG_ERROR("Shader at %s, failed to compile.", ShaderPath)
		
#ifdef GS_DEBUG
		GLint logSize = 0;
		glGetShaderiv(RendererObjectId, GL_INFO_LOG_LENGTH, &logSize);

		char * log = new char[logSize];

		glGetShaderInfoLog(RendererObjectId, logSize, NULL, log);

		GS_LOG_WARNING(log)

		delete[] log;
#endif // GS_DEBUG
	}
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
		GS_LOG_WARNING("Failed to load shader at %s!", Path)
	}

	return Code;
}
