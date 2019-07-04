#pragma once

#include "Core.h"

#include "FVector.hpp"
#include "Extent.h"

class ColorRenderTarget;
class DepthStencilRenderTarget;

GS_STRUCT FramebufferCreateInfo
{
	RenderPass* RenderPass;
	Extent2D Extent;
};

GS_CLASS Framebuffer
{
	FVector<ColorRenderTarget *>		ColorRenderTargets;
	FVector<DepthStencilRenderTarget *> DepthStencilRenderTargets;

	Extent2D Extent;
public:
	Framebuffer(ColorRenderTarget** _CRT, uint8 _CRTCount, DepthStencilRenderTarget** _DSRT, uint8 _DSRTCount, Extent2D _Extent) :
		ColorRenderTargets(_CRT, _CRTCount),
		DepthStencilRenderTargets(_DSRT, _DSRTCount),
		Extent(_Extent)
	{
	}
	virtual ~Framebuffer();
};