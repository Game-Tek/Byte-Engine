#pragma once

#include "ByteEngine/Game/System.h"
#include <GTSL/Ranger.h>

#include <GAL/Vulkan/VulkanRenderDevice.h>
#include <GAL/Vulkan/VulkanRenderContext.h>

namespace GTSL {
	class Window;
}

class RenderSystem : public System
{
public:
	RenderSystem() = default;

	struct InitializeRendererInfo
	{
		GTSL::Window* Window{ 0 };
	};
	void InitializeRenderer(const InitializeRendererInfo& initializeRenderer);
	
	void UpdateWindow(GTSL::Window& window);
	
	void Initialize() override;
	void Shutdown() override;

	using RenderDevice = GAL::VulkanRenderDevice;
	using RenderContext = GAL::VulkanRenderContext;
	using Queue = GAL::VulkanQueue;
	
private:
	RenderDevice renderDevice;
	RenderContext renderContext;

	Queue graphicsQueue;
};
