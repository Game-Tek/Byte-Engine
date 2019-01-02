#include "Program.h"

#include "glad.h"

#include "VertexShader.h"
#include "FragmentShader.h"


Program::Program()
{
	VertexShader VS;
	FragmentShader FS;

	RendererObjectId = glCreateProgram();
	glAttachShader(RendererObjectId, VS.GetId());
	glAttachShader(RendererObjectId, FS.GetId());
	glLinkProgram(RendererObjectId);
}

Program::~Program()
{
	glDeleteProgram(RendererObjectId);
}

void Program::Bind() const
{
	glUseProgram(RendererObjectId);
}

void Program::SetUniform(const char * UniformName, float F1) const
{
	glUniform1f(glGetUniformLocation(RendererObjectId, UniformName), F1);
}

void Program::SetUniform(const char * UniformName, float F1, float F2) const
{
	glUniform2f(glGetUniformLocation(RendererObjectId, UniformName), F1, F2);
}

void Program::SetUniform(const char * UniformName, float F1, float F2, float F3) const
{
	glUniform3f(glGetUniformLocation(RendererObjectId, UniformName), F1, F2, F3);
}

void Program::SetUniform(const char * UniformName, float F1, float F2, float F3, float F4) const
{
	glUniform4f(glGetUniformLocation(RendererObjectId, UniformName), F1, F2, F3, F4);
}

void Program::SetUniform(const char * UniformName, int I1) const
{
	glUniform1i(glGetUniformLocation(RendererObjectId, UniformName), I1);
}

void Program::SetUniform(const char * UniformName, int I1, int I2) const
{
	glUniform2i(glGetUniformLocation(RendererObjectId, UniformName), I1, I2);
}

void Program::SetUniform(const char * UniformName, int I1, int I2, int I3) const
{
	glUniform3i(glGetUniformLocation(RendererObjectId, UniformName), I1, I2, I3);
}

void Program::SetUniform(const char * UniformName, int I1, int I2, int I3, int I4) const
{
	glUniform4i(glGetUniformLocation(RendererObjectId, UniformName), I1, I2, I3, I4);
}

void Program::SetUniform(const char * UniformName, bool B1)
{
	glUniform1uiv(glGetUniformLocation(RendererObjectId, UniformName), B1);
}
