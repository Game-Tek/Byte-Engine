#pragma once

#include "Core.h"
#include "Utility/Extent.h"
#include "Containers/FVector.hpp"

namespace RAPI
{
	class Window;
	class RenderDevice;
	class RenderTarget;

	struct ResizeInfo
	{
		RenderDevice* RenderDevice = nullptr;
		Extent2D NewWindowSize;
	};

	struct RenderContextCreateInfo
	{
		Window* Window = nullptr;
		uint8 DesiredFramesInFlight = 0;
	};
	
	class RenderContext
	{
	protected:
		uint8 currentImage = 0;
		uint8 maxFramesInFlight = 0;

		Extent2D extent{ 0, 0 };

	public:
		virtual ~RenderContext()
		{
		};

		virtual void OnResize(const ResizeInfo& _RI) = 0;

		[[nodiscard]] virtual FVector<RenderTarget*> GetSwapchainImages() const = 0;

		[[nodiscard]] uint8 GetCurrentImage() const { return currentImage; }
		[[nodiscard]] uint8 GetMaxFramesInFlight() const { return maxFramesInFlight; }
	};
}
