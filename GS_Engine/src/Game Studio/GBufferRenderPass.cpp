#include "GBufferRenderPass.h"

#include <GLAD/glad.h>
#include "GL.h"

#include "Renderer.h"
#include "RenderProxy.h"

GBufferRenderPass::GBufferRenderPass(Renderer * RendererOwner) : RenderPass(RendererOwner), Position(ImageSize(1280, 720), GL_RGB16F, GL_RGB, GL_FLOAT), Normal(ImageSize(1280, 720), GL_RGB16F, GL_RGB, GL_FLOAT), Albedo(ImageSize(1280, 720), GL_RGBA, GL_RGB, GL_UNSIGNED_BYTE), GBuffer(),
GBufferPassProgram("W:\Game Studio\GS_Engine\src\Game Studio\GBufferVS.vshader", "W:\Game Studio\GS_Engine\src\Game Studio\GBufferFS.fshader"), ViewMatrix(&GBufferPassProgram, "uView"), ProjMatrix(&GBufferPassProgram, "uProjection")
{
	//Bind the GBuffer frame buffer so all subsequent texture attachment calls are done on this frame buffer.
	GBuffer.Bind();

	//Attach textures to the bound frame buffer.
	GBuffer.AttachTexture(Position);		//Position Texture.
	GBuffer.AttachTexture(Normal);			//Normal Texture.
	GBuffer.AttachTexture(Albedo);			//Albedo Texture.
}

GBufferRenderPass::~GBufferRenderPass()
{
}

void GBufferRenderPass::Render()
{
	SetAsActive();

	DrawCalls = RendererOwner->GetScene()->RenderProxyList.length();

	for (size_t i = 0; i < DrawCalls; i++)
	{
		RendererOwner->GetScene()->RenderProxyList[i]->Draw();
	}

	return;
}

void GBufferRenderPass::SetAsActive() const
{
	GBuffer.Bind();

	//Set draw buffer.
	GS_GL_CALL(glDrawBuffers(GBuffer.GetNumberOfBoundTextures(), GBuffer.GetActiveColorAttachments()));

	ViewMatrix.Set(*RendererOwner->GetScene()->GetViewMatrix());
	ProjMatrix.Set(*RendererOwner->GetScene()->GetProjectionMatrix());

	return;
}