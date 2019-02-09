#include "Uniform.h"

Uniform::Uniform(Program * Program, const char * UniformName)
{
	GS_GL_CALL(RendererObjectId = glGetUniformLocation(Program->GetId(), UniformName));
}

Uniform::~Uniform()
{
}