#include "LightRenderPass.h"

#include "Texture.h"
#include "GBufferRenderPass.h"
#include "Renderer.h"

#include "LightRenderProxy.h"

LightRenderPass::LightRenderPass(Renderer * RendererOwner) : RenderPass(RendererOwner), LightingPassProgram("W:/Game Studio/GS_Engine/src/Game Studio/LightingVS.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/LightingFS.fshader"), ViewMatrix(LightingPassProgram, "uView"), ProjMatrix(LightingPassProgram, "uProjection")
{
}

LightRenderPass::~LightRenderPass()
{
}

void LightRenderPass::SetAsActive() const
{
	LightingPassProgram.Bind();

	Texture::SetActiveTextureUnit(0);
	RendererOwner->GetGBufferPass()->GetPositionTexture().Bind();
	Texture::SetActiveTextureUnit(1);
	RendererOwner->GetGBufferPass()->GetNormalTexture().Bind();
	Texture::SetActiveTextureUnit(2);
	RendererOwner->GetGBufferPass()->GetAlbedoTexture().Bind();

	return;
}

void LightRenderPass::Render()
{
	FrameBuffer::Clear();

	SetAsActive();

	DrawCalls = RendererOwner->GetScene()->LightRenderProxyList.length();

	ViewMatrix.Set(RendererOwner->GetScene()->GetViewMatrix());
	ProjMatrix.Set(RendererOwner->GetScene()->GetProjectionMatrix());

	for (size_t i = 0; i < DrawCalls; i++)
	{
		RendererOwner->GetScene()->LightRenderProxyList[i]->Draw();
	}
}