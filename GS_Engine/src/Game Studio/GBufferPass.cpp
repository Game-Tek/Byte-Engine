#include "GBufferPass.h"

#include <GLAD/glad.h>
#include "GL.h"

GBufferPass::GBufferPass() : Position(ImageSize(1280, 720), GL_RGB16F, GL_RGB, GL_FLOAT), Normal(ImageSize(1280, 720), GL_RGB16F, GL_RGB, GL_FLOAT), Albedo(ImageSize(1280, 720), GL_RGBA, GL_RGB, GL_UNSIGNED_BYTE)
{
	//Bind the GBuffer frame buffer so all subsequent texture attachment calls are done on this frame buffer.
	GBuffer.Bind();

	//Attach textures to the frame buffer.
	GBuffer.AttachTexture(Position);		//Position Texture.
	GBuffer.AttachTexture(Normal);			//Normal Texture.
	GBuffer.AttachTexture(Albedo);			//Albedo Texture.
}

GBufferPass::~GBufferPass()
{
}

void GBufferPass::SetAsActive() const
{
	//Bind draw buffer.
	GS_GL_CALL(glDrawBuffers(GBuffer.GetNumberOfBoundTextures(), GBuffer.GetActiveColorAttachments().GetData()));
}
