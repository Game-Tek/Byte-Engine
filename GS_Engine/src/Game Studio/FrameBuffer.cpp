#include "FrameBuffer.h"

#include "GL.h"
#include "GLAD/glad.h"

#include "Texture.h"

FrameBuffer::FrameBuffer(const uint8 NumberOfColorAttachments) : ColorAttachments(GenColorAttachments(NumberOfColorAttachments))
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

void FrameBuffer::BindForRead() const
{
	GS_GL_CALL(glBindFramebuffer(GL_READ_FRAMEBUFFER, RendererObjectId));
}

void FrameBuffer::BindForWrite() const
{
	GS_GL_CALL(glBindFramebuffer(GL_DRAW_FRAMEBUFFER, RendererObjectId));
}

void FrameBuffer::UnBind() const
{
	GS_GL_CALL(glBindFramebuffer(GL_FRAMEBUFFER, 0));
}

void FrameBuffer::AttachTexture(const Texture & Texture)
{
	GS_GL_CALL(glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + BoundTextures, GL_TEXTURE_2D, Texture.GetId(), 0));

	BoundTextures++;
}

void FrameBuffer::AttachTexture(Texture * Texture)
{
	GS_GL_CALL(glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + BoundTextures, GL_TEXTURE_2D, Texture->GetId(), 0));
	
	BoundTextures++;
}

void FrameBuffer::Clear()
{
	GS_GL_CALL(glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT));

	return;
}

void FrameBuffer::CopyFrameBuffer(const ImageSize & Size)
{
	GS_GL_CALL(glBlitFramebuffer(0, 0, Size.Width, Size.Height, 0, 0, Size.Width, Size.Height, GL_COLOR_BUFFER_BIT, GL_LINEAR););
}

void FrameBuffer::SetAsDrawBuffer() const
{
	GS_GL_CALL(glDrawBuffers(BoundTextures, ColorAttachments));
}

void FrameBuffer::UnBindWrite()
{
	GS_GL_CALL(glBindFramebuffer(GL_DRAW_FRAMEBUFFER, 0));
}

void FrameBuffer::UnBindRead()
{
	GS_GL_CALL(glBindFramebuffer(GL_READ_FRAMEBUFFER, 0));
}

void FrameBuffer::SetReadBuffer(const uint8 Index)
{
	GS_GL_CALL(glReadBuffer(GL_COLOR_ATTACHMENT0 + Index));
}

uint32 * FrameBuffer::GenColorAttachments(const uint8 N)
{
	uint32 * ca_ = new uint32[N];

	for (uint8 i = 0; i < N; i++)
	{
		ca_[i] = GL_COLOR_ATTACHMENT0 + i;
	}

	return ca_;
}