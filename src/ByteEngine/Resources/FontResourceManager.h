#pragma once

#include <string>
#include <unordered_map>
#include <GTSL/Buffer.hpp>
#include <GTSL/Extent.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/Serialize.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Vectors.hpp>


#include "ResourceManager.h"
#include "ByteEngine/Core.h"
#include "ByteEngine/Application/AllocatorReferences.h"

struct IVector2D
{
	IVector2D() = default;

	IVector2D(const int32 x, const int32 y) : X(x), Y(y) {}
	
	int32 X = 0, Y = 0;
};

namespace GTSL {
	template<class ALLOCATOR>
	class Buffer;
}

class FontResourceManager : public ResourceManager
{
public:
	FontResourceManager(const InitializeInfo&);
	
	struct Character
	{
		GTSL::Extent2D Size;       // Size of glyph
		IVector2D Bearing;    // Address from baseline to left/top of glyph
		GTSL::Extent2D Position;
		uint32 Advance;    // Address to advance to next glyph
	};

	struct FontData : Data
	{
		GTSL::HashMap<uint32, Character, BE::PAR> Characters;
	};

	struct FontDataSerialize : DataSerialize<FontData>
	{
		INSERT_START(FontDataSerialize)
		{
			INSERT_BODY;
			GTSL::Insert(insertInfo.Characters, buffer);
		}

		EXTRACT_START(FontDataSerialize)
		{
			EXTRACT_BODY;
			GTSL::Extract(extractInfo.Characters, buffer);
		}
	};

	struct FontInfo : Info<FontDataSerialize>
	{
		//DECL_INFO_CONSTRUCTOR(FontInfo, Info<FontDataSerialize>);
	};
	
private:
	//int8 parseData(const char* data, Font* fontData);
};
