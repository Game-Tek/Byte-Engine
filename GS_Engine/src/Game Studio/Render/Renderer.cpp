#include "Renderer.h"

#include "Vulkan\VulkanRenderer.h"

#include "Platform\Windows\WindowsWindow.h"

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

Window* Renderer::CreateWindow(const WindowCreateInfo& _WCI)
{
	return new WindowsWindow(_WCI.Extent, _WCI.Name);
}
