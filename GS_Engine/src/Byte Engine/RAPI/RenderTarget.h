#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Utility/Extent.h"

namespace RAPI
{
	class RenderTarget
	{
	protected:
		Extent3D Extent{ 0, 0, 1 };
		ImageFormat Format{ ImageFormat::RGBA_I8 };
		ImageType Type{ ImageType::COLOR };
		ImageDimensions Dimensions{ ImageDimensions::IMAGE_1D };

	public:

		struct RenderTargetCreateInfo
		{
			Extent3D		Extent		{ 0, 0, 0 };
			ImageFormat		Format		{ ImageFormat::RGBA_I8 };
			ImageType		Type		{ ImageType::COLOR };
			ImageDimensions Dimensions	{ ImageDimensions::IMAGE_2D };
			ImageUse		Use			{ ImageUse::INPUT_ATTACHMENT };
		};
		explicit RenderTarget(const RenderTargetCreateInfo& renderTargetCreateInfo) :
			Extent(renderTargetCreateInfo.Extent),
			Format(renderTargetCreateInfo.Format),
			Type(renderTargetCreateInfo.Type),
			Dimensions(renderTargetCreateInfo.Dimensions)
		{
		}

		RenderTarget() = default;

		[[nodiscard]] Extent3D GetExtent() const { return Extent; }
		[[nodiscard]] ImageFormat GetFormat() const { return Format; }
		[[nodiscard]] ImageType	GetType() const { return Type; }
		[[nodiscard]] ImageDimensions GetDimensions() const { return Dimensions; }
	};
}
