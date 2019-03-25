#include "FrameBuffer.h"

#include "GL.h"
#include "GLAD/glad.h"

#include "Texture.h"

FrameBuffer::FrameBuffer()
{
	GS_GL_CALL(glGenFramebuffers(1, &RendererObjectId));
}


FrameBuffer::~FrameBuffer()
{
	GS_GL_CALL(glDeleteFramebuffers(1, &RendererObjectId));
}

void FrameBuffer::Bind() const
{
	GS_GL_CALL(glBindFramebuffer(GL_FRAMEBUFFER, RendererObjectId));
}
void FrameBuffer::UnBind() const
{
	GS_GL_CALL(glBindFramebuffer(GL_FRAMEBUFFER, 0));
}

void FrameBuffer::AttachTexture(const Texture & Texture)
{
	GS_GL_CALL(glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, Texture.GetId(), 0));
}

void FrameBuffer::AttachTexture(Texture * Texture)
{
	GS_GL_CALL(glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, Texture->GetId(), 0));
}