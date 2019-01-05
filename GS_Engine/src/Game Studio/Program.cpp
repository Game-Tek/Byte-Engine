#include "Program.h"

#include "glad.h"

#include "Shader.h"

#include "GL.h"

Program::Program()
{
	Shader VS(GL_VERTEX_SHADER, "W:/Game Studio/GS_Engine/src/Game Studio/VertexShader.vshader");
	Shader FS(GL_FRAGMENT_SHADER, "W:/Game Studio/GS_Engine/src/Game Studio/FragmentShader.fshader");

	RendererObjectId = GS_GL_CALL(glCreateProgram());
	GS_GL_CALL(glAttachShader(RendererObjectId, VS.GetId()));
	GS_GL_CALL(glAttachShader(RendererObjectId, FS.GetId()));
	GS_GL_CALL(glLinkProgram(RendererObjectId));
}

Program::~Program()
{
	GS_GL_CALL(glDeleteProgram(RendererObjectId));
}

void Program::Bind() const
{
	GS_GL_CALL(glUseProgram(RendererObjectId));
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
	glUniform1ui(glGetUniformLocation(RendererObjectId, UniformName), B1);
}
