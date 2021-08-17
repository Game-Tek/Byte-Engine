#pragma once

#include "RenderCore.h"

#include <GTSL/Extent.h>
#include <GTSL/Memory.h>
#include <GTSL/SIMD/SIMD128.hpp>

namespace GAL
{
	//Represents a resource utilized by the rendering API for storing and referencing textures. Which are images which hold some information loaded from memory.
	class Texture
	{
	public:
		Texture() = default;
		~Texture() = default;

		static GTSL::uint32 GetImageSize(const GTSL::uint8 textureFormatSize, const GTSL::Extent2D extent)
		{
			return static_cast<GTSL::uint32>(textureFormatSize) * extent.Width * extent.Height;
		}

		/**
		 * \brief Assumes source and target image formats are different(wont't fail if they are the same but it will perform the conversion and copying), assumes target format has a higher channel count that source.
		 * \param sourceImageFormat 
		 * \param targetImageFormat 
		 * \param imageExtent 
		 * \param buffer 
		 */
		static void ConvertTextureFormat(const FormatDescriptor sourceImageFormat, const FormatDescriptor targetImageFormat, const GTSL::Extent2D imageExtent, GTSL::AlignedPointer<GTSL::byte, 16> buffer, GTSL::uint8 alphaValue)
		{
			const GTSL::uint8 sourceFormatSize = sourceImageFormat.BitDepth * sourceImageFormat.ComponentCount, targetFormatSize = targetImageFormat.BitDepth * targetImageFormat.ComponentCount;
			const GTSL::uint32 targetTextureSize = GetImageSize(targetFormatSize, imageExtent), sourceTextureSize = GetImageSize(sourceFormatSize, imageExtent);

			auto rgb_i8_to_rgba_i8 = [&]()
			{
				using Vector = GTSL::SIMD128<GTSL::uint8>;
				
				GTSL::uint32 bytesToProcessWithScalar = sourceTextureSize % Vector::Bytes;
				GTSL::uint32 bytesToProcessWithVector = sourceTextureSize - bytesToProcessWithScalar;
				
				GTSL::uint32 pixelsToProcessWithScalar = bytesToProcessWithScalar / sourceFormatSize;
				GTSL::uint32 pixelsToProcessWithVector = bytesToProcessWithVector / sourceFormatSize;
				
				GTSL::uint32 srcPixelsPerVector = Vector::Bytes / sourceFormatSize;
				GTSL::uint32 dstPixelsPerVector = Vector::Bytes / targetFormatSize;
				
				GTSL::uint32 vectorsInSrc = sourceTextureSize / Vector::Bytes; //TODO pixels per vector = 10,6... ??

				GTSL::uint32 pixels = imageExtent.Width * imageExtent.Height;
				
				GTSL::byte* source = buffer.Get() + sourceTextureSize - sourceFormatSize,* target = buffer.Get() + targetTextureSize - targetFormatSize;
				
				//for(GTSL::uint32 i = 0; i < pixelsToProcessWithScalar; ++i) //loop for each pixel
				//{
				//	GTSL::MemCopy(sourceFormatSize, source, target);
				//	*(target + 3) = alphaValue;
				//
				//	source -= sourceFormatSize;
				//	target -= targetFormatSize;
				//}
				//
				//source = buffer.Get() + sourceTextureSize - Vector::Bytes; target = buffer.Get() + targetTextureSize - Vector::Bytes;
				//
				//while(source != buffer.Get() - Vector::Bytes) //loop for each group of bytes that fit in a vector
				//{
				//	GTSL::SIMD128<GTSL::uint8> data{ GTSL::AlignedPointer<GTSL::uint8, 16>(source) };
				//
				//	//data = Vector::Shuffle<0, 1, 2, 0, 3, 4, 5, 0, 6, 7, 8, 0, 9, 10, 11, 0>(data);
				//	
				//	data.CopyTo(GTSL::AlignedPointer<GTSL::uint8, 16>(target));
				//
				//	for (GTSL::uint32 j = 0; j < dstPixelsPerVector; ++j)
				//	{
				//		GTSL::MemCopy(sourceFormatSize, source, target);
				//		(*(target + ((j * 4) - 1))) = alphaValue;
				//	}
				//
				//	source -= Vector::Bytes;
				//	target -= Vector::Bytes;
				//}

				for(GTSL::uint32 i = 0; i < pixels; ++i) //loop for each pixel
				{
					GTSL::MemCopy(sourceFormatSize, source, target);
					*(target + 3) = alphaValue;
				
					source -= sourceFormatSize;
					target -= targetFormatSize;
				}
			};
			
			switch (MakeFormatFromFormatDescriptor(sourceImageFormat))
			{
				case Format::RGB_I8:
				{
					switch (MakeFormatFromFormatDescriptor(targetImageFormat))
					{
						case Format::RGBA_I8: rgb_i8_to_rgba_i8(); return;
						
						default: break;
					}
				}

				case Format::RGBA_I8:
				{
					switch (MakeFormatFromFormatDescriptor(targetImageFormat))
					{
						case Format::RGBA_I8: return;

						default: break;
					}
				}
				
				default: __debugbreak();
			}
		}
	};

	class ImageView
	{
	public:
		ImageView() = default;
		~ImageView() = default;
	};

	class Sampler
	{
	public:
	};
}
