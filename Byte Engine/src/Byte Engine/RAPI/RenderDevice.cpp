#include "RenderDevice.h"

#include "Vulkan/VulkanRenderDevice.h"

using namespace RAPI;

void RenderDevice::GetAvailableRenderAPIs(FVector<RenderAPI>& renderApis)
{
#if (BE_PLATFORM_WIN) // PLATFORMS WHICH SUPPORT VULKAN
	if (VulkanRenderDevice::IsVulkanSupported())
	{
		renderApis.emplace_back(RenderAPI::VULKAN);
	}
#endif // (BE_PLATFORM_WIN) // PLATFORMS WHICH SUPPORT VULKAN

}

RenderDevice* RAPI::RenderDevice::CreateRenderDevice(const RenderDeviceCreateInfo& renderDeviceCreateInfo)
{
	BE_ASSERT(renderDeviceCreateInfo.RenderingAPI == RenderAPI::NONE, "renderApi is RenderAPI::NONE, which is not a valid API, please select another option preferably one of those returned by RenderDevice::GetAvailableRenderAPIs()")

#ifdef BE_DEBUG
	FVector<RenderAPI> available_render_apis;
	GetAvailableRenderAPIs(available_render_apis);

	auto supported = false;
	for (auto& e : available_render_apis)
	{
		if (e == renderDeviceCreateInfo.RenderingAPI)
		{
			supported = true;
			break;
		}
	}

	BE_ASSERT(supported, "Chosen Render API is not available. Please query supported APIs with RenderDevice::GetAvailableRenderAPIs()")
#endif


		switch (renderDeviceCreateInfo.RenderingAPI)
		{
		case RenderAPI::NONE: return nullptr;
		case RenderAPI::VULKAN: return new VulkanRenderDevice(renderDeviceCreateInfo);
		default: return nullptr;
		}
}

void RenderDevice::DestroyRenderDevice(const RenderDevice* renderDevice)
{
	delete renderDevice;
}