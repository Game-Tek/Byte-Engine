#include "Renderer.h"

#include "Vulkan\Vulkan.h"
#include "Vulkan\VulkanRenderer.h"

RAPI Renderer::RenderAPI = RAPI::NONE;
Renderer* Renderer::RendererInstance = CreateRenderer();

Renderer* Renderer::CreateRenderer()
{
	switch (RenderAPI)
	{
	case RAPI::NONE:		return nullptr;
	case RAPI::VULKAN:		return new VulkanRenderer();
	}
}