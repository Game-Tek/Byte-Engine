#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Utility/Extent.h"

namespace RAPI
{
	class RenderTarget
	{
	protected:
		Extent3D Extent{ 0, 0, 0 };
		Format Format{ Format::RGBA_I8 };
		ImageType Type{ ImageType::COLOR };
		ImageDimensions Dimensions{ ImageDimensions::IMAGE_1D };

	public:

		struct RenderTargetCreateInfo
		{
			Extent3D		Extent		{ 0, 0, 0 };
			RAPI::Format	Format		{ Format::RGBA_I8 };
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

		INLINE Extent3D			GetExtent()		const { return Extent; }
		INLINE RAPI::Format		GetFormat()		const { return Format; }
		INLINE ImageType		GetType()		const { return Type; }
		INLINE ImageDimensions	GetDimensions() const { return Dimensions; }
	};
}
