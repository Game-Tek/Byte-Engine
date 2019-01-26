#include "Uniform.h"

#include "glad.h"

#include "GL.h"

Uniform::Uniform(const RendererObject & Program, const char * UniformName)
{
	GS_GL_CALL(RendererObjectId = glGetUniformLocation(Program.GetId(), UniformName));
}


Uniform::~Uniform()
{
}
