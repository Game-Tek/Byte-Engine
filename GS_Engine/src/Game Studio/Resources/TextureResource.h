#pragma once

#include "Core.h"

#include "Resource.h"

#include "RAPI/RenderCore.h"
#include "Utility/Extent.h"

class GS_API TextureResource : public Resource
{
public:
	class TextureResourceData final : public ResourceData
	{
	public:
		char* ImageData = nullptr;
		size_t imageDataSize = 0;

		~TextureResourceData();
		
		void** WriteTo(size_t _Index, size_t _Bytes) override
		{
			switch (_Index)
			{
			case 0: return reinterpret_cast<void**>(&ImageData);
			default: return nullptr;
			}
		}

		friend OutStream& operator<<(OutStream& _OS, TextureResourceData& _TRD)
		{
			_OS.Write(_TRD.imageDataSize, _TRD.ImageData);
			return _OS;
		}

		friend InStream& operator>>(InStream& _IS, TextureResourceData& _TRD)
		{
			_IS.Read(_TRD.imageDataSize, _TRD.ImageData);
			return _IS;
		}
	};

private:
	TextureResourceData data;

	bool LoadResource(const FString& _Path) override;
	void LoadFallbackResource(const FString& _Path) override;
	
	[[nodiscard]] const char* GetResourceTypeExtension() const override { return "png"; }
public:
	TextureResource() = default;
	~TextureResource() = default;

	[[nodiscard]] const char* GetName() const override { return "TextureResource"; }

	[[nodiscard]] const TextureResourceData& GetTextureData() const { return data; }
protected:
	//Used to hold the texture's dimensions once it's been loaded.
	Extent2D TextureDimensions;

	//Used to hold the number of channels this texture has.
	Format TextureFormat;
};

