#pragma once

#include "Core.h"

#include "RenderContext.h"
#include "Mesh.h"
#include "Pipelines.h"
#include "RenderPass.h"
#include "Framebuffer.h"

enum class RAPI : uint8
{
	NONE, VULKAN
};

GS_CLASS Renderer
{
	static RAPI RenderAPI;
	static Renderer* RendererInstance;
	
	static Renderer* CreateRenderer();
	static RAPI GetRAPI();
protected:
	Renderer() = default;
	virtual ~Renderer() = default;
public:
	static INLINE RAPI GetRenderAPI() { return RenderAPI; }
	static INLINE Renderer* GetRenderer() { return RendererInstance; }

	virtual Mesh* CreateMesh(const MeshCreateInfo& _MCI) = 0;
	virtual GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) = 0;
	virtual ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) = 0;
	virtual RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) = 0;
	virtual Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) = 0;
	virtual RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) = 0;
};