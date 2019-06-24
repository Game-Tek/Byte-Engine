#include "Renderer.h"

#ifdef GS_RAPI_VULKAN
#include "Vulkan\VulkanRenderer.h"
#endif // GS_RAPI_VULKAN

Renderer* Renderer::CreateRenderer()
{
#ifdef GS_RAPI_VULKAN
	return new VulkanRenderer();
#endif // GS_RAPI_VULKAN

	return nullptr;
}
