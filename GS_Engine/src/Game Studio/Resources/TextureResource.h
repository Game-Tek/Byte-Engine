#pragma once

#include "Core.h"

#include "Resource.h"

#include "RAPI/RenderCore.h"
#include "Utility/Extent.h"

class GS_API TextureResource : public Resource
{
public:
	class TextureResourceData : public ResourceData
	{
	public:
		char* ImageData = nullptr;

		void** WriteTo(size_t _Index, size_t _Bytes) override
		{
			switch (_Index)
			{
			case 0: return reinterpret_cast<void**>(&ImageData);
			default: return nullptr;
			}
		}
	};

	TextureResource() = default;
	~TextureResource();

	[[nodiscard]] size_t GetDataSize() const override { return TextureFormat == Format::RGBA_I8 ? 4 : 3 * (TextureDimensions.Width * TextureDimensions.Height); }

	bool LoadResource(const FString& _Path) override;
	void LoadFallbackResource(const FString& _Path) override;

protected:
	//Used to hold the texture's dimensions once it's been loaded.
	Extent2D TextureDimensions;

	//Used to hold the number of channels this texture has.
	Format TextureFormat;
};

