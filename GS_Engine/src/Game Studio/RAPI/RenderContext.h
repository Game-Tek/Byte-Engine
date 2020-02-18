#pragma once

#include "Core.h"
#include "Utility/Extent.h"
#include "Containers/FVector.hpp"

namespace RAPI
{
	class Image;

	struct ResizeInfo
	{
		Extent2D NewWindowSize;
	};

	struct RenderContextCreateInfo
	{
	};
	
	class RenderContext
	{
	protected:
		uint8 CurrentImage = 0;
		uint8 MAX_FRAMES_IN_FLIGHT = 0;

	public:
		virtual ~RenderContext()
		{
		};

		virtual void OnResize(const ResizeInfo& _RI) = 0;

		[[nodiscard]] virtual FVector<Image*> GetSwapchainImages() const = 0;

		[[nodiscard]] uint8 GetCurrentImage() const { return CurrentImage; }
		[[nodiscard]] uint8 GetMaxFramesInFlight() const { return MAX_FRAMES_IN_FLIGHT; }
	};
}
