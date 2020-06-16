#pragma once

#include "System.h"
#include <GTSL/Ranger.h>

#include <GAL/Vulkan/VulkanRenderDevice.h>
#include <GAL/Vulkan/VulkanRenderContext.h>

namespace GTSL {
	class Window;
}

using RenderDevice = GAL::VulkanRenderDevice;
using RenderContext = GAL::VulkanRenderContext;
using Queue = GAL::VulkanQueue;

class RenderSystem : public System
{
public:
	RenderSystem() = default;

	struct InitializeRendererInfo
	{
		GTSL::Window* Window{ 0 };
	};
	void InitializeRenderer(const InitializeRendererInfo& initializeRenderer);
	
	void Process(const GTSL::Ranger<World*>& worlds) override
	{
	}
	
	void UpdateWindow(GTSL::Window& window);
	
	void Initialize() override;
	void Shutdown() override;
private:
	RenderDevice renderDevice;
	RenderContext renderContext;

	Queue graphicsQueue;
};
