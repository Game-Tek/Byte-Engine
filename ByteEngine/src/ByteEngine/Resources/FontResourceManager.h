#pragma once

#include <map>
#include <string>
#include <unordered_map>
#include <GAL/RenderCore.h>
#include <GTSL/Buffer.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/Extent.h>
#include <GTSL/FlatHashMap.h>
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
	class Buffer;
}

struct ShortVector
{
	int16 X, Y;
};

class FontResourceManager : public ResourceManager
{
public:
	FontResourceManager() : ResourceManager("FontResourceManager"), fonts(4, GetPersistentAllocator()) {}
	
	struct Segment
	{
		//0 is on curve
		//1 is control point or nan
		//2 is on curve
		GTSL::Vector2 Points[3];

		bool IsCurve = false;
		
		bool IsBezierCurve() const { return IsCurve; }
		
	private:
		bool b = false, c = false, d = false;
		uint32 e = 0;
	};

	struct FontMetaData
	{
		uint16 UnitsPerEm;
		int16 Ascender;
		int16 Descender;
		int16 LineGap;
	};
	
	struct Path
	{
		GTSL::Vector<Segment, BE::PersistentAllocatorReference> Segments;
	};

	struct Glyph
	{
		uint32 Character;
		int16 GlyphIndex;
		int16 NumContours;
		GTSL::Vector<Path, BE::PersistentAllocatorReference> Paths;
		GTSL::Vector<GTSL::Vector<ShortVector, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> RawPaths;
		uint16 AdvanceWidth;
		int16 LeftSideBearing;
		int16 BoundingBox[4];
		GTSL::Vector2 Center;
	};

	//MAIN STRUCT
	struct Font
	{
		uint32 FileNameHash;
		std::string FullFontName;
		std::string NameTable[25];
		std::unordered_map<uint32, int16_t> KerningTable;
		std::unordered_map<uint16, Glyph> Glyphs;
		std::map<uint32, uint16> GlyphMap;
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
	
	struct ImageFont
	{
		std::map<char, Character> Characters;
		GTSL::Buffer ImageData;
		GTSL::Extent2D Extent;
	};
	
	Font GetFont(const GTSL::Range<const UTF8*> fontName);

	struct OnFontLoadInfo : OnResourceLoad
	{
		ImageFont* Font;
		GAL::TextureFormat TextureFormat;
		GTSL::Extent3D Extent;
	};
	
	struct FontLoadInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(TaskInfo, OnFontLoadInfo)> OnFontLoadDelegate;
	};
	void LoadImageFont(const FontLoadInfo& fontLoadInfo);

	~FontResourceManager()
	{
		auto deallocate = [&](ImageFont& imageFont)
		{
			imageFont.ImageData.Free(8, GetPersistentAllocator());
		};
		
		GTSL::ForEach(fonts, deallocate);
	}

	void GetFontAtlasSizeFormatExtent(Id id, uint32* textureSize, GAL::TextureFormat* textureFormat, GTSL::Extent3D* extent3D);
	
	void doThing();
	
private:
	int8 parseData(const char* data, Font* fontData);

	
	GTSL::FlatHashMap<ImageFont, BE::PersistentAllocatorReference> fonts;
};
