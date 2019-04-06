#include "FBO.h"

#include "GL.h"
#include "GLAD/glad.h"

#include "Texture.h"

FBO::FBO(const uint8 NumberOfColorAttachments) : ColorAttachments(GenColorAttachments(NumberOfColorAttachments))
{
	GS_GL_CALL(glGenFramebuffers(1, &RendererObjectId));
}


FBO::~FBO()
{
	GS_GL_CALL(glDeleteFramebuffers(1, &RendererObjectId));
}

void FBO::Bind() const
{
	GS_GL_CALL(glBindFramebuffer(GL_FRAMEBUFFER, RendererObjectId));
}

void FBO::BindForRead() const
{
	GS_GL_CALL(glBindFramebuffer(GL_READ_FRAMEBUFFER, RendererObjectId));
}

void FBO::BindForWrite() const
{
	GS_GL_CALL(glBindFramebuffer(GL_DRAW_FRAMEBUFFER, RendererObjectId));
}

void FBO::UnBind() const
{
	GS_GL_CALL(glBindFramebuffer(GL_FRAMEBUFFER, 0));
}

void FBO::BindDefault()
{
	GS_GL_CALL(glBindFramebuffer(GL_FRAMEBUFFER, 0));
}

void FBO::BindDefaultForWrite()
{
	GS_GL_CALL(glBindFramebuffer(GL_DRAW_FRAMEBUFFER, 0));
}

void FBO::AttachTexture(const Texture & Texture)
{
	GS_GL_CALL(glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + BoundTextures, GL_TEXTURE_2D, Texture.GetId(), 0));

	BoundTextures += 1;
}

void FBO::AttachTexture(Texture * Texture)
{
	GS_GL_CALL(glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + BoundTextures, GL_TEXTURE_2D, Texture->GetId(), 0));
	
	BoundTextures += 1;
}

void FBO::AttachDepthTexture(const Texture & Texture)
{
	GS_GL_CALL(glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, Texture.GetId(), 0));
}

void FBO::Clear()
{
	GS_GL_CALL(glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT));

	return;
}

void FBO::CopyFBO(const ImageSize & Size)
{
	GS_GL_CALL(glBlitFramebuffer(0, 0, Size.Width, Size.Height, 0, 0, Size.Width, Size.Height, GL_COLOR_BUFFER_BIT, GL_LINEAR));
}

void FBO::CopyDepthFBOAttachment(const ImageSize & Size)
{
	GS_GL_CALL(glBlitFramebuffer(0, 0, Size.Width, Size.Height, 0, 0, Size.Width, Size.Height, GL_DEPTH_BUFFER_BIT, GL_NEAREST));
}

void FBO::SetAsDrawBuffer() const
{
	GS_GL_CALL(glDrawBuffers(BoundTextures, ColorAttachments));
}

void FBO::UnBindWrite()
{
	GS_GL_CALL(glBindFramebuffer(GL_DRAW_FRAMEBUFFER, 0));
}

void FBO::UnBindRead()
{
	GS_GL_CALL(glBindFramebuffer(GL_READ_FRAMEBUFFER, 0));
}

void FBO::SetReadBuffer(const uint8 Index)
{
	GS_GL_CALL(glReadBuffer(GL_COLOR_ATTACHMENT0 + Index));
}

uint32 * FBO::GenColorAttachments(const uint8 N)
{
	uint32 * ca_ = new uint32[N];

	for (uint8 i = 0; i < N; i++)
	{
		ca_[i] = GL_COLOR_ATTACHMENT0 + i;
	}

	return ca_;
}