#include "LightRenderPass.h"

#include "Texture.h"
#include "GBufferRenderPass.h"
#include "Renderer.h"

#include "GSM.hpp"

#include "PointLightRenderProxy.h"

#include "WorldObject.h"

LightRenderPass::LightRenderPass(Renderer * RendererOwner) : RenderPass(RendererOwner), LightingPassProgram()
{
}

LightRenderPass::~LightRenderPass()
{
}

void LightRenderPass::Render()
{
	FBO::BindDefault();

	FBO::Clear();

	PointLightProg.Bind();

	PointLightProg.ViewMatrix.Set(RendererOwner->GetScene()->GetViewMatrix());
	PointLightProg.ProjectionMatrix.Set(RendererOwner->GetScene()->GetProjectionMatrix());

	Texture::SetTargetTextureUnit(0);
	RendererOwner->GetGBufferPass()->GetPositionTexture().Bind();

	/*
	for (size_t i = 0; i < RendererOwner->GetScene()->PointLightRenderProxyList.length(); i++)
	{
		PointLightProg.ModelMatrix.Set(GSM::Translation(RendererOwner->GetScene()->RenderProxyList[i]->GetOwner()->GetPosition()));

		RendererOwner->GetScene()->PointLightRenderProxyList[i]->Draw();
	}
	*/


	LightingPassProgram.Bind();

	Texture::SetTargetTextureUnit(0);
	RendererOwner->GetGBufferPass()->GetPositionTexture().Bind();
	LightingPassProgram.AlbedoTextureSampler.Set(0);

	Texture::SetTargetTextureUnit(1);
	RendererOwner->GetGBufferPass()->GetNormalTexture().Bind();
	LightingPassProgram.NormalTextureSampler.Set(1);

	Texture::SetTargetTextureUnit(2);
	RendererOwner->GetGBufferPass()->GetAlbedoTexture().Bind();
	LightingPassProgram.AlbedoTextureSampler.Set(2);


	LightingPassProgram.ViewMatrix.Set(RendererOwner->GetScene()->GetViewMatrix());
	LightingPassProgram.ProjectionMatrix.Set(RendererOwner->GetScene()->GetProjectionMatrix());

	Quad.Draw();

	RendererOwner->GetGBufferPass()->GetGBuffer().BindForRead();

	FBO::BindDefaultForWrite();

	FBO::CopyDepthFBOAttachment(ImageSize(1280, 720));

	FBO::BindDefault();
}