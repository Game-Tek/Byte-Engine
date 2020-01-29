#include "RenderDevice.h"

#include "Vulkan/VulkanRenderDevice.h"

RAPIs RenderDevice::RenderAPI = GetRAPIs();
RenderDevice* RenderDevice::RenderDeviceInstance = CreateRAPI();

RenderDevice* RenderDevice::CreateRAPI()
{
	switch (RenderAPI)
	{
	case RAPIs::NONE: return nullptr;
	case RAPIs::VULKAN: return new VulkanRenderDevice();
	default: return nullptr;
	}
}

RAPIs RenderDevice::GetRAPIs()
{
#ifdef GS_RAPI_VULKAN
	return RAPIs::VULKAN;
#endif
}
