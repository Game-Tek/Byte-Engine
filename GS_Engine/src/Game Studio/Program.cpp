#include "Program.h"

#include "glad.h"

#include "Shader.h"

#include "GL.h"

Program::Program(const char * VertexShaderPath, const char * FragmentShaderPath)
{
	Shader VS(GL_VERTEX_SHADER, VertexShaderPath);
	Shader FS(GL_FRAGMENT_SHADER, FragmentShaderPath);

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

void Program::SetUniform(const Uniform & Uniform, float F1) const
{
	GS_GL_CALL(glUniform1f(Uniform.GetId(), F1));
}

void Program::SetUniform(const Uniform & Uniform, float F1, float F2) const
{
	GS_GL_CALL(glUniform2f(Uniform.GetId(), F1, F2));
}

void Program::SetUniform(const Uniform & Uniform, float F1, float F2, float F3) const
{
	GS_GL_CALL(glUniform3f(Uniform.GetId(), F1, F2, F3));
}

void Program::SetUniform(const Uniform & Uniform, float F1, float F2, float F3, float F4) const
{
	GS_GL_CALL(glUniform4f(Uniform.GetId(), F1, F2, F3, F4));
}

void Program::SetUniform(const Uniform & Uniform, int I1) const
{
	GS_GL_CALL(glUniform1i(Uniform.GetId(), I1));
}

void Program::SetUniform(const Uniform & Uniform, int I1, int I2) const
{
	GS_GL_CALL(glUniform2i(Uniform.GetId(), I1, I2));
}

void Program::SetUniform(const Uniform & Uniform, int I1, int I2, int I3) const
{
	GS_GL_CALL(glUniform3i(Uniform.GetId(), I1, I2, I3));
}

void Program::SetUniform(const Uniform & Uniform, int I1, int I2, int I3, int I4) const
{
	GS_GL_CALL(glUniform4i(Uniform.GetId(), I1, I2, I3, I4));
}

void Program::SetUniform(const Uniform & Uniform, bool B1) const
{
	GS_GL_CALL(glUniform1ui(Uniform.GetId(), B1));
}

void Program::SetUniform(const Uniform & Uniform, const Matrix4 & Matrix) const
{
	GS_GL_CALL(glUniformMatrix4fv(Uniform.GetId(), 1, GL_FALSE, Matrix.GetData()));
}