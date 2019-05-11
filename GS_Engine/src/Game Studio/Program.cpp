#include "Program.h"

#include "GLAD/glad.h"

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

	ModelMatrix.Setup(this, "uModel");
	ViewMatrix.Setup(this, "uView");
	ProjectionMatrix.Setup(this, "uProjection");
}

Program::~Program()
{
	GS_GL_CALL(glDeleteProgram(RendererObjectId));
}

void Program::Bind() const
{
	GS_GL_CALL(glUseProgram(RendererObjectId));
}