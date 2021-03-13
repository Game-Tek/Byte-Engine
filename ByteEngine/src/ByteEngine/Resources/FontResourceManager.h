#pragma once

#include <map>
#include <string>
#include <unordered_map>
#include <GAL/RenderCore.h>
#include <GTSL/Buffer.hpp>
#include <GTSL/Delegate.hpp>
#include <GTSL/Extent.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Serialize.h>
#include <GTSL/Vector.hpp>
#include <GTSL/Math/Vector2.h>


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

struct ShortVector
{
	int16 X, Y;
};

class FontResourceManager : public ResourceManager
{
public:
	FontResourceManager();

	struct Segment
	{
		//0 is on curve
		//1 is control point or nan
		//2 is on curve
		GTSL::Vector2 Points[3];

		bool IsCurve = false;
		
		bool IsBezierCurve() const { return IsCurve; }
	};

	struct FontMetaData
	{
		uint16 UnitsPerEm;
		int16 Ascender;
		int16 Descender;
		int16 LineGap;
	};

	using Path = GTSL::Vector<Segment, BE::PersistentAllocatorReference>;
	
	struct Glyph
	{
		uint32 Character;
		int16 GlyphIndex;
		int16 NumContours;
		GTSL::Vector<Path, BE::PersistentAllocatorReference> Paths;
		uint16 AdvanceWidth;
		int16 LeftSideBearing;
		GTSL::Vector2 BoundingBox[2]; //min, max
		GTSL::Vector2 Center;
	};

	//MAIN STRUCT
	struct Font
	{
		uint32 FileNameHash;
		std::string FullFontName;
		std::string NameTable[25];
		GTSL::FlatHashMap<uint32, int16, BE::PAR> KerningTable;
		GTSL::FlatHashMap<uint32, Glyph, BE::PAR> Glyphs;
		GTSL::FlatHashMap<uint32, uint16, BE::PAR> GlyphMap;
		FontMetaData Metadata;
		uint64 LastUsed;
	};
	
	struct Character
	{
		GTSL::Extent2D Size;       // Size of glyph
		IVector2D Bearing;    // Offset from baseline to left/top of glyph
		GTSL::Extent2D Position;
		uint32 Advance;    // Offset to advance to next glyph
	};

	struct FontData : Data
	{
		GTSL::FlatHashMap<uint32, Character, BE::PAR> Characters;
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
	int8 parseData(const char* data, Font* fontData);
};
