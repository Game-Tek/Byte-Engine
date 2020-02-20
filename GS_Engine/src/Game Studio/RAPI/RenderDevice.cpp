#include "RenderDevice.h"

#include "Vulkan/VulkanRenderDevice.h"

using namespace RAPI;

void RenderDevice::GetAvailableRenderAPIs(FVector<RenderAPI>& renderApis)
{
#ifdef GS_PLATFORM_WIN
	renderApis.emplace_back(RenderAPI::VULKAN);
#endif
}

RenderDevice* RenderDevice::CreateRenderDevice(const RenderAPI renderApi)
{
	GS_ASSERT(renderApi == RenderAPI::NONE, "renderApi is RenderAPI::NONE, which is not a valid API, please select another option preferably one of those returned by RenderDevice::GetAvailableRenderAPIs()")
	
#ifdef GS_DEBUG

	FVector<RenderAPI> available_render_apis;
	GetAvailableRenderAPIs(available_render_apis);

	auto supported = false;
	for (auto& e : available_render_apis)
	{
		if (e == renderApi)
		{
			supported = true;
			break;
		}
	}

	GS_ASSERT(supported, "Chosen Render API is not available. Please query supported APIs with RenderDevice::GetAvailableRenderAPIs()")

#endif

	
	switch (renderApi)
	{
	case RenderAPI::NONE: return nullptr;
	case RenderAPI::VULKAN: return new VulkanRenderDevice();
	default: return nullptr;
	}
}

void RenderDevice::DestroyRenderDevice(const RenderDevice* renderDevice)
{
	delete renderDevice;
}
