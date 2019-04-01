#include "Uniform.h"

Uniform::Uniform(const Program & Program, const char * UniformName)
{
	GS_GL_CALL(RendererObjectId = glGetUniformLocation(Program.GetId(), UniformName));
}

Uniform::Uniform(Program * Program, const char * UniformName)
{
	GS_GL_CALL(RendererObjectId = glGetUniformLocation(Program->GetId(), UniformName));
}

Uniform::~Uniform()
{
}

void Uniform::Set(float Other) const
{
	GS_GL_CALL(glUniform1f(RendererObjectId, Other));
}

void Uniform::Set(const Vector2 & Other) const
{
	GS_GL_CALL(glUniform2f(RendererObjectId, Other.X, Other.Y));
}

void Uniform::Set(const Vector3 & Other) const
{
	GS_GL_CALL(glUniform3f(RendererObjectId, Other.X, Other.Y, Other.Z));
}

void Uniform::Set(const Vector4 & Other) const
{
	GS_GL_CALL(glUniform4f(RendererObjectId, Other.X, Other.Y, Other.Z, Other.W));
}

void Uniform::Set(int Other) const
{
	GS_GL_CALL(glUniform1i(RendererObjectId, Other));
}

void Uniform::Set(bool Other) const
{
	GS_GL_CALL(glUniform1i(RendererObjectId, Other));
}

void Uniform::Set(Matrix4 * Other) const
{
	GS_GL_CALL(glUniformMatrix4fv(RendererObjectId, 1, GL_FALSE, Other->GetData()));
}

void Uniform::Set(const Matrix4 & Other) const
{
	GS_GL_CALL(glUniformMatrix4fv(RendererObjectId, 1, GL_FALSE, Other.GetData()));
}