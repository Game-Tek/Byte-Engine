#pragma once

#include "Core.h"

#include "RenderContext.h"
#include "RenderTarget.h"
#include "RenderMesh.h"
#include "GraphicsPipeline.h"
#include "ComputePipeline.h"
#include "RenderPass.h"
#include "Framebuffer.h"
#include "UniformBuffer.h"
#include "Bindings.h"
#include "Texture.h"
#include "CommandBuffer.h"

namespace RAPI
{
	enum class RenderAPI : uint8
	{
		NONE,
		VULKAN,
		DIRECTX12
	};

	struct GPUInfo
	{
		FString GPUName;
		uint32 DriverVersion;
		uint32 APIVersion;
	};

	class Queue
	{
	public:
		enum class QueueCapabilities : uint8
		{
			GRAPHICS = 1, COMPUTE = 2, TRANSFER = 4
		};

	private:
		QueueCapabilities capabilities;

	public:
		struct QueueCreateInfo
		{
			QueueCapabilities Capabilities;
			float QueuePriority = 1.0f;
			Queue** QueueToSet = nullptr;
		};

		struct DispatchInfo : RenderInfo
		{
			class CommandBuffer* CommandBuffer = nullptr;
		};
		virtual void Dispatch(const DispatchInfo& dispatchInfo) = 0;
	};

	class RenderDevice
	{
	protected:
		RenderDevice() = default;

		virtual ~RenderDevice() = default;
		
	public:
		static void GetAvailableRenderAPIs(FVector<RenderAPI>& renderApis);

		struct RenderDeviceCreateInfo
		{
			RenderAPI RenderingAPI;
			FString ApplicationName;
			uint16 ApplicationVersion[3];
			FVector<Queue::QueueCreateInfo>* QueueCreateInfos = nullptr;
			//FVector<RAPIExtensions>* Extensions = nullptr;
		};
		static RenderDevice* CreateRenderDevice(const RenderDeviceCreateInfo& renderDeviceCreateInfo);
		static void DestroyRenderDevice(const RenderDevice* renderDevice);

		virtual GPUInfo GetGPUInfo() = 0;

		virtual RenderMesh* CreateRenderMesh(const RenderMesh::RenderMeshCreateInfo& _MCI) = 0;
		virtual UniformBuffer* CreateUniformBuffer(const UniformBufferCreateInfo& _BCI) = 0;
		virtual RenderTarget* CreateRenderTarget(const RenderTarget::RenderTargetCreateInfo& _ICI) = 0;
		virtual Texture* CreateTexture(const TextureCreateInfo& TCI_) = 0;
		virtual BindingsPool* CreateBindingsPool(const BindingsPoolCreateInfo& bindingsPoolCreateInfo) = 0;
		virtual BindingsSet* CreateBindingsSet(const BindingsSetCreateInfo& bindingsSetCreateInfo) = 0;
		virtual GraphicsPipeline* CreateGraphicsPipeline(const GraphicsPipelineCreateInfo& _GPCI) = 0;
		virtual ComputePipeline* CreateComputePipeline(const ComputePipelineCreateInfo& _CPCI) = 0;
		virtual RenderPass* CreateRenderPass(const RenderPassCreateInfo& _RPCI) = 0;
		virtual Framebuffer* CreateFramebuffer(const FramebufferCreateInfo& _FCI) = 0;
		virtual RenderContext* CreateRenderContext(const RenderContextCreateInfo& _RCCI) = 0;
		virtual CommandBuffer* CreateCommandBuffer(const CommandBuffer::CommandBufferCreateInfo& commandBufferCreateInfo) = 0;
	};
}