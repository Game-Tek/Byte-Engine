#include "RAPI.h"

#include "Vulkan/VulkanRenderer.h"

RAPIs RAPI::RenderAPI = GetRAPIs();
RAPI* RAPI::RAPIInstance = CreateRAPI();

RAPI* RAPI::CreateRAPI()
{
	switch (RenderAPI)
	{
	case RAPIs::NONE:		return nullptr;
	case RAPIs::VULKAN:		return new VulkanRAPI();
	default:				return nullptr;
	}
}

RAPIs RAPI::GetRAPIs()
{
#ifdef GS_RAPI_VULKAN
	return RAPIs::VULKAN;
#endif
}
