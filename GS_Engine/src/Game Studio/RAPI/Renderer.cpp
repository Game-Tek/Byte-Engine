#include "Renderer.h"

#include "Vulkan/VulkanRenderer.h"

RAPI Renderer::RenderAPI = GetRAPI();
Renderer* Renderer::RendererInstance = CreateRenderer();

Renderer* Renderer::CreateRenderer()
{
	switch (RenderAPI)
	{
	case RAPI::NONE:		return nullptr;
	case RAPI::VULKAN:		return new VulkanRenderer();
	}
}

RAPI Renderer::GetRAPI()
{
#ifdef GS_RAPI_VULKAN
	return RAPI::VULKAN;
#endif
}
