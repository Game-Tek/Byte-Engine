#include "LightRenderPass.h"

#include "Texture.h"
#include "GBufferRenderPass.h"
#include "Renderer.h"

#include "LightRenderProxy.h"

LightRenderPass::LightRenderPass(Renderer * RendererOwner) : RenderPass(RendererOwner), LightingPassProgram("W:/Game Studio/GS_Engine/src/Game Studio/LightingVS.vshader", "W:/Game Studio/GS_Engine/src/Game Studio/LightingFS.fshader"), ViewMatrix(LightingPassProgram, "uView"), ProjMatrix(LightingPassProgram, "uProjection"), PositionTextureSampler(LightingPassProgram, "uPosition"), NormalTextureSampler(LightingPassProgram, "uNormal"), AlbedoTextureSampler(LightingPassProgram, "uAlbedo")
{
}

LightRenderPass::~LightRenderPass()
{
}

void LightRenderPass::Render()
{
	FBO::BindDefault();

	FBO::Clear();

	LightingPassProgram.Bind();

	Texture::SetActiveTextureUnit(0);
	RendererOwner->GetGBufferPass()->GetPositionTexture().Bind();
	PositionTextureSampler.Set(0);
	Texture::SetActiveTextureUnit(1);
	RendererOwner->GetGBufferPass()->GetNormalTexture().Bind();
	NormalTextureSampler.Set(1);
	Texture::SetActiveTextureUnit(2);
	RendererOwner->GetGBufferPass()->GetAlbedoTexture().Bind();
	AlbedoTextureSampler.Set(2);

	DrawCalls = RendererOwner->GetScene()->LightRenderProxyList.length();

	ViewMatrix.Set(RendererOwner->GetScene()->GetViewMatrix());
	ProjMatrix.Set(RendererOwner->GetScene()->GetProjectionMatrix());

	//for (size_t i = 0; i < DrawCalls; i++)
	//{
	//	RendererOwner->GetScene()->LightRenderProxyList[i]->Draw();
	//}

	Quad.Draw();

	RendererOwner->GetGBufferPass()->GetGBuffer().BindForRead();

	FBO::BindDefaultForWrite();

	FBO::CopyDepthFBOAttachment(ImageSize(1280, 720));

	FBO::BindDefault();
}