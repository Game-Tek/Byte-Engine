#pragma once

#include "Core.h"

#include "RenderContext.h"
#include "Image.h"
#include "RenderMesh.h"
#include "GraphicsPipeline.h"
#include "ComputePipeline.h"
#include "RenderPass.h"
#include "Framebuffer.h"
#include "UniformBuffer.h"
#include "UniformLayout.h"
#include "Texture.h"

enum class RAPIs : uint8
{
	NONE, VULKAN
};

class GS_API RenderDevice
{
	static RAPIs RenderAPI;
	static RenderDevice* RenderDeviceInstance;
	
	static RenderDevice* CreateRAPI();
	static RAPIs GetRAPIs();
protected:
	RenderDevice() = default;
	virtual ~RenderDevice()
	{
		delete RenderDeviceInstance;
	}
	
public:
	static INLINE RAPIs GetRenderAPI() { return RenderAPI; }
	static INLINE RenderDevice* Get() { return RenderDeviceInstance; }

	virtual RenderMesh* CreateMesh(const MeshCreateInfo& _MCI) = 0;
	virtual UniformBuffer* CreateUniformBuffer(const UniformBufferCreateInfo& _BCI) = 0;
	virtual UniformLayout* CreateUniformLayout(const UniformLayoutCreateInfo& _ULCI) = 0;
	virtual Image* CreateImage(const ImageCreateInfo& _ICI) = 0;
	virtual Texture* CreateTexture(const TextureCreateInfo& TCI_) = 0;
	virtual GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) = 0;
	virtual ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) = 0;
	virtual RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) = 0;
	virtual Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) = 0;
	virtual RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) = 0;
};