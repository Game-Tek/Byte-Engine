#include "FontResourceManager.h"

/*
* ttf-parser
*  A single header ttf parser
*  Reads the minimum information needed to render antialiased glyph geometry as fast as possible
*  Browser support using emscripten
*
*  A glyph is represented as a set of triangles (p_x, p1, p2) where p_x is the center of the glyph and
*  p1 and p2 are sequential points on the curve. Quadratic splines will have 2 tiangles associated with them,
*  (p_x, p1, p2) as before and (p1, p_c, p2) where p_c is the spline control point.
*
*  author: Kaushik Viswanathan <kaushik@ocutex.com>
*  https://github.com/kv01/ttf-parser
*/

#include <map>
#include <unordered_map>
#include <vector>

#include "ByteEngine/Core.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.h>
#include <GTSL/Math/Vectors.h>

#include "TextRendering.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

void get8b(void* dst, const char* src)
{
	if constexpr (_WIN64) {
		static_cast<uint8_t*>(dst)[0] = src[7];
		static_cast<uint8_t*>(dst)[1] = src[6];
		static_cast<uint8_t*>(dst)[2] = src[5];
		static_cast<uint8_t*>(dst)[3] = src[4];
		static_cast<uint8_t*>(dst)[4] = src[3];
		static_cast<uint8_t*>(dst)[5] = src[2];
		static_cast<uint8_t*>(dst)[6] = src[1];
		static_cast<uint8_t*>(dst)[7] = src[0];
	} else {
		static_cast<uint8_t*>(dst)[0] = src[0];
		static_cast<uint8_t*>(dst)[1] = src[1];
		static_cast<uint8_t*>(dst)[2] = src[2];
		static_cast<uint8_t*>(dst)[3] = src[3];
		static_cast<uint8_t*>(dst)[4] = src[4];
		static_cast<uint8_t*>(dst)[5] = src[5];
		static_cast<uint8_t*>(dst)[6] = src[6];
		static_cast<uint8_t*>(dst)[7] = src[7];
	}
}

void get4b(void* dst, const char* src) {
	if constexpr (_WIN64) {
		static_cast<uint8_t*>(dst)[0] = src[3];
		static_cast<uint8_t*>(dst)[1] = src[2];
		static_cast<uint8_t*>(dst)[2] = src[1];
		static_cast<uint8_t*>(dst)[3] = src[0];
	} else {
		static_cast<uint8_t*>(dst)[0] = src[0];
		static_cast<uint8_t*>(dst)[1] = src[1];
		static_cast<uint8_t*>(dst)[2] = src[2];
		static_cast<uint8_t*>(dst)[3] = src[3];
	}
}

void get2b(void* dst, const char* src)
{
	if constexpr (_WIN64) {
		static_cast<uint8_t*>(dst)[0] = src[1];
		static_cast<uint8_t*>(dst)[1] = src[0];
	} else {
		static_cast<uint8_t*>(dst)[0] = src[0];
		static_cast<uint8_t*>(dst)[1] = src[1];
	}
}

void get1b(void* dst, const char* src) {
	static_cast<uint8_t*>(dst)[0] = src[0];
}

void get8b(void* dst, const char* src, uint32& offset) {
	offset += 8;	
	get8b(dst, src);
}

void get4b(void* dst, const char* src, uint32& offset) {
	offset += 4;	
	get4b(dst, src);
}

void get2b(void* dst, const char* src, uint32& offset) {
	offset += 2;
	get2b(dst, src);
}

void get1b(void* dst, const char* src, uint32& offset) {
	++offset;	
	get1b(dst, src);
}

float32 to_2_14_float(const int16 value) {
	return static_cast<float32>(value & 0x3fff) / static_cast<float32>(1 << 14) + (-2 * ((value >> 15) & 0x1) + ((value >> 14) & 0x1));
}

struct Flags {
	bool xDual : 1;
	bool yDual : 1;
	bool xShort : 1;
	bool yShort : 1;
	bool repeat : 1;
	bool isControlPoint : 1;
};

enum COMPOUND_GLYPH_FLAGS {
	ARG_1_AND_2_ARE_WORDS = 0x0001,
	ARGS_ARE_XY_VALUES = 0x0002,
	ROUND_XY_TO_GRID = 0x0004,
	WE_HAVE_A_SCALE = 0x0008,
	MORE_COMPONENTS = 0x0020,
	WE_HAVE_AN_X_AND_Y_SCALE = 0x0040,
	WE_HAVE_A_TWO_BY_TWO = 0x0080,
	WE_HAVE_INSTRUCTIONS = 0x0100,
	USE_MY_METRICS = 0x0200,
	OVERLAP_COMPOUND = 0x0400,
	SCALED_COMPONENT_OFFSET = 0x0800,
	UNSCALED_COMPONENT_OFFSET = 0x1000
};

struct TTFHeader {
	uint32 Version;
	uint16 NumberOfTables;
	uint16 SearchRange;
	uint16 EntrySelector;
	uint16 RangeShift;

	uint32 Parse(const char* data, uint32 offset)
	{
		get4b(&Version, data + offset); offset += sizeof(uint32);
		get2b(&NumberOfTables, data + offset); offset += sizeof(uint16);
		/*get2b(&searchRange, data + offset); offset += sizeof(uint16);
		get2b(&entrySelector, data + offset); offset += sizeof(uint16);
		get2b(&rangeShift, data + offset); offset += sizeof(uint16);*/
		offset += sizeof(uint16) * 3;
		return offset;
	}
};

struct TableEntry
{
	uint32 tag;
	char tagstr[5];
	uint32 checkSum;
	uint32 offsetPos;
	uint32 length;

	uint32 Parse(const char* data, uint32 offset)
	{
		get4b(&tag, data + offset); memcpy(tagstr, data + offset, sizeof(uint32)); tagstr[4] = 0; offset += sizeof(uint32);
		get4b(&checkSum, data + offset); offset += sizeof(uint32);
		get4b(&offsetPos, data + offset); offset += sizeof(uint32);
		get4b(&length, data + offset); offset += sizeof(uint32);
		return offset;
	}
};

struct HeadTable
{
	float32 tableVersion;
	float32 fontRevision;
	uint32 checkSumAdjustment;
	uint32 magicNumber;//0x5F0F3CF5
	uint16 flags;
	uint16 unitsPerEm;
	long long createdDate;
	long long modifiedData;
	short xMin;
	short yMin;
	short xMax;
	short yMax;
	uint16 macStyle;
	uint16 lowestRecPPEM;
	short fontDirectionHintl;
	short indexToLocFormat;
	short glyphDataFormat;

	uint32 Parse(const char* data, uint32 offset)
	{
		get4b(&tableVersion, data + offset); offset += sizeof(uint32);
		get4b(&fontRevision, data + offset); offset += sizeof(uint32);
		get4b(&checkSumAdjustment, data + offset); offset += sizeof(uint32);
		get4b(&magicNumber, data + offset); offset += sizeof(uint32);
		get2b(&flags, data + offset); offset += sizeof(uint16);
		get2b(&unitsPerEm, data + offset); offset += sizeof(uint16);
		get8b(&createdDate, data + offset); offset += sizeof(uint64_t);
		get8b(&modifiedData, data + offset); offset += sizeof(uint64_t);
		get2b(&xMin, data + offset); offset += sizeof(short);
		get2b(&yMin, data + offset); offset += sizeof(short);
		get2b(&xMax, data + offset); offset += sizeof(short);
		get2b(&yMax, data + offset); offset += sizeof(short);
		get2b(&macStyle, data + offset); offset += sizeof(uint16);
		get2b(&lowestRecPPEM, data + offset); offset += sizeof(uint16);
		get2b(&fontDirectionHintl, data + offset); offset += sizeof(short);
		get2b(&indexToLocFormat, data + offset); offset += sizeof(short);
		get2b(&glyphDataFormat, data + offset); offset += sizeof(short);
		return offset;
	}
};

struct MaximumProfile
{
	float32 version;
	uint16 numGlyphs;
	uint16 maxPoints;
	uint16 maxContours;
	uint16 maxCompositePoints;
	uint16 maxCompositeContours;
	uint16 maxZones;
	uint16 maxTwilightPoints;
	uint16 maxStorage;
	uint16 maxFunctionDefs;
	uint16 maxInstructionDefs;
	uint16 maxStackElements;
	uint16 maxSizeOfInstructions;
	uint16 maxComponentElements;
	uint16 maxComponentDepth;

	uint32 Parse(const char* data, uint32 offset) {
		get4b(&version, data + offset); offset += sizeof(uint32);
		get2b(&numGlyphs, data + offset); offset += sizeof(uint16);
		get2b(&maxPoints, data + offset); offset += sizeof(uint16);
		get2b(&maxContours, data + offset); offset += sizeof(uint16);
		get2b(&maxCompositePoints, data + offset); offset += sizeof(uint16);
		get2b(&maxCompositeContours, data + offset); offset += sizeof(uint16);
		get2b(&maxZones, data + offset); offset += sizeof(uint16);
		get2b(&maxTwilightPoints, data + offset); offset += sizeof(uint16);
		get2b(&maxStorage, data + offset); offset += sizeof(uint16);
		get2b(&maxFunctionDefs, data + offset); offset += sizeof(uint16);
		get2b(&maxInstructionDefs, data + offset); offset += sizeof(uint16);
		get2b(&maxStackElements, data + offset); offset += sizeof(uint16);
		get2b(&maxSizeOfInstructions, data + offset); offset += sizeof(uint16);
		get2b(&maxComponentElements, data + offset); offset += sizeof(uint16);
		get2b(&maxComponentDepth, data + offset); offset += sizeof(uint16);
		return offset;
	}
};

struct NameValue
{
	uint16 platformID;
	uint16 encodingID;
	uint16 languageID;
	uint16 nameID;
	uint16 length;
	uint16 offset_value;

	uint32 Parse(const char* data, uint32 offset)
	{
		get2b(&platformID, data + offset); offset += sizeof(uint16);
		get2b(&encodingID, data + offset); offset += sizeof(uint16);
		get2b(&languageID, data + offset); offset += sizeof(uint16);
		get2b(&nameID, data + offset); offset += sizeof(uint16);
		get2b(&length, data + offset); offset += sizeof(uint16);
		get2b(&offset_value, data + offset); offset += sizeof(uint16);
		return offset;
	}
};

struct NameTable
{
	uint16 Format;
	uint16 count;
	uint16 stringOffset;
	std::vector<NameValue> NameRecords;

	uint32 Parse(const char* data, uint32 offset, std::string* names, uint16 maxNumberOfNames = 25)
	{
		uint32 offset_start = offset;
		get2b(&Format, data + offset); offset += sizeof(uint16);
		get2b(&count, data + offset); offset += sizeof(uint16);
		get2b(&stringOffset, data + offset); offset += sizeof(uint16);
		NameRecords.resize(count);
		for (uint32 i = 0; i < count; i++)
		{
			if (NameRecords[i].nameID > maxNumberOfNames) { continue; }
			
			offset = NameRecords[i].Parse(data, offset);
			char* newNameString = new char[NameRecords[i].length];
			memcpy(newNameString, data + offset_start + stringOffset + NameRecords[i].offset_value, sizeof(char) * NameRecords[i].length);
			uint16 string_length = NameRecords[i].length;
			
			if (newNameString[0] == 0) {
				string_length = string_length >> 1;
				
				for (uint16 j = 0; j < string_length; j++) {
					newNameString[j] = newNameString[j * 2 + 1];
				}
			}
			
			names[NameRecords[i].nameID].assign(newNameString, string_length);
			
			delete[] newNameString;
		}
		return offset;
	}
};

struct HHEATable
{
	uint16 majorVersion;
	uint16 minorVersion;
	int16 Ascender;
	int16 Descender;
	int16 LineGap;
	uint16 advanceWidthMax;
	int16 minLeftSideBearing;
	int16 minRightSideBearing;
	int16 xMaxExtent;
	int16 caretSlopeRise;
	int16 caretSlopeRun;
	int16 caretOffset;
	int16 metricDataFormat;
	uint16 numberOfHMetrics;

	uint32 Parse(const char* data, uint32 offset)
	{
		get2b(&majorVersion, data + offset); offset += sizeof(uint16);
		get2b(&minorVersion, data + offset); offset += sizeof(uint16);
		get2b(&Ascender, data + offset); offset += sizeof(int16);
		get2b(&Descender, data + offset); offset += sizeof(int16);
		get2b(&LineGap, data + offset); offset += sizeof(int16);
		get2b(&advanceWidthMax, data + offset); offset += sizeof(uint16);
		get2b(&minLeftSideBearing, data + offset); offset += sizeof(int16);
		get2b(&minRightSideBearing, data + offset); offset += sizeof(int16);
		get2b(&xMaxExtent, data + offset); offset += sizeof(int16);
		get2b(&caretSlopeRise, data + offset); offset += sizeof(int16);
		get2b(&caretSlopeRun, data + offset); offset += sizeof(int16);
		get2b(&caretOffset, data + offset); offset += sizeof(int16);
		offset += sizeof(int16) * 4;
		get2b(&metricDataFormat, data + offset); offset += sizeof(int16);
		get2b(&numberOfHMetrics, data + offset); offset += sizeof(uint16);
		return offset;
	}
};

struct float_v4
{
	float32 data[4];
};

struct FontLineInfoData
{
	uint32 StringStartIndex;
	uint32 StringEndIndex;
	GTSL::Vector2 OffsetStart;
	GTSL::Vector2 OffsetEnd;
	GTSL::Vector<FontResourceManager::Glyph*, BE::PersistentAllocatorReference> GlyphIndex;
};

struct FontPositioningOutput
{
	GTSL::Vector<FontLineInfoData, BE::PersistentAllocatorReference> LinePositions;
	uint32 NumTriangles;
	//PixelPositioning alignment;
	//BoundingRect bounding_rect;
	uint32 Geometry;
	uint16 FontSize;
};

struct FontPositioningOptions
{
	bool IsMultiline;
	bool IsWordPreserve;
	float32 LineHeight;
	//PixelPositioning alignment;
	//BoundingRect bounding_rect;
	FontPositioningOptions()
	{
		IsMultiline = true;
		IsWordPreserve = true;
		LineHeight = 1.0f;
	}
};

int16 GetKerningOffset(FontResourceManager::Font* font_data, uint16 left_glyph, uint16 right_glyph)
{
	auto kern_data = font_data->KerningTable.TryGet((left_glyph << 16) | right_glyph);
	return kern_data.State() ? kern_data.Get() : 0;
}

FontResourceManager::FontResourceManager(): ResourceManager(u8"FontResourceManager")
{
	auto path = GetResourcePath(GTSL::StaticString<64>(u8"Fonts"), GTSL::ShortString<32>(u8"bepkg"));
	
	GTSL::File beFontFile; beFontFile.Open(path, GTSL::File::WRITE, true);

	auto GetFont = [&](const GTSL::Range<const utf8*> fontName) {
		GTSL::StaticString<255> path(BE::Application::Get()->GetPathToApplication()); path += u8"/resources/"; path += fontName; path += u8".ttf";

		GTSL::File fontFile; fontFile.Open(path, GTSL::File::READ, false);
		GTSL::Buffer fileBuffer(fontFile.GetSize(), 8, GetTransientAllocator());

		fontFile.Read(fileBuffer);

		Font fontData(GetPersistentAllocator());
		const auto result = parseData(reinterpret_cast<const char*>(fileBuffer.GetData()), &fontData);
		BE_ASSERT(result > -1, "Failed to parse!");

		return fontData;
	};
	
	auto font = GetFont(GTSL::StaticString<64>(u8"FTLTLT"));

	GTSL::Buffer<BE::TAR> data(1000000, 8, GetTransientAllocator());

	//glyf map
	//glyfs
	
	for(auto& e : font.Glyphs) {
		Face face(GetPersistentAllocator());
		MakeFromPaths(e, face, 4, GetPersistentAllocator());

		for(auto f : face.LinearBeziers) {
			Insert(f.Points[0], data);
			Insert(f.Points[1], data);
		}

		for(auto f : face.CubicBeziers) {
			Insert(f.Points[0], data);
			Insert(f.Points[1], data);
			Insert(f.Points[2], data);
		}

		for(auto f : face.Bands) {
			for (auto l : f.Lines) {
				Insert(l, data);
			}

			for (auto c : f.Curves) {
				Insert(c, data);
			}
		}
	}

	beFontFile.Write(data);
}

GTSL::Vector2 toVector(const ShortVector sh) { return GTSL::Vector2(sh.X, sh.Y); }

int8 FontResourceManager::parseData(const char* data, Font* fontData)
{
	uint32 ptr = 0;
	
	TTFHeader header;
	ptr = header.Parse(data, ptr);
	
	std::unordered_map<std::string, TableEntry> tables;
	for (uint16 i = 0; i < header.NumberOfTables; i++)
	{
		TableEntry te;
		ptr = te.Parse(data, ptr);
		tables[te.tagstr] = te;
	}

	auto headTableEntry = tables.find("head");
	if (headTableEntry == tables.end()) { return -2; }
	HeadTable headTable;
	ptr = headTable.Parse(data, headTableEntry->second.offsetPos);
	
	auto maxpTableEntry = tables.find("maxp");
	if (maxpTableEntry == tables.end()) { return -2; }
	MaximumProfile maximumProfile;
	maximumProfile.Parse(data, maxpTableEntry->second.offsetPos);
	
	auto nameTableEntry = tables.find("name");
	if (nameTableEntry == tables.end()) { return -2; }
	NameTable nameTable;
	nameTable.Parse(data, nameTableEntry->second.offsetPos, fontData->NameTable);

	fontData->FullFontName = fontData->NameTable[1] + " " + fontData->NameTable[2];

	auto locaTableEntry = tables.find("loca");
	if (locaTableEntry == tables.end()) { return -2; }

	if (!maximumProfile.numGlyphs) { return -1; }
	
	std::vector<uint32> glyphIndices(maximumProfile.numGlyphs);

	uint32 endOfGlyf = 0;

	if (headTable.indexToLocFormat == 0)
	{
		uint32 byte_offset = locaTableEntry->second.offsetPos;

		for (uint16 i = 0; i < maximumProfile.numGlyphs; i++)
		{
			get2b(&glyphIndices[i], data + byte_offset, byte_offset);
			glyphIndices[i] = glyphIndices[i] << 1;
		}

		get2b(&endOfGlyf, data + byte_offset, byte_offset);
		endOfGlyf = endOfGlyf << 1;
	}
	else
	{
		uint32 byte_offset = locaTableEntry->second.offsetPos;
		for (uint16 i = 0; i < maximumProfile.numGlyphs; i++)
		{
			get4b(&glyphIndices[i], data + byte_offset, byte_offset);
		}
		get4b(&endOfGlyf, data + byte_offset, byte_offset);
	}

	auto cmapTableEntry = tables.find("cmap");
	if (cmapTableEntry == tables.end()) { return -2; }

	uint32 cmap_offset = cmapTableEntry->second.offsetPos + sizeof(uint16); //Skip version
	uint16 cmap_num_tables;
	get2b(&cmap_num_tables, data + cmap_offset, cmap_offset);

	std::map<uint16, uint32> glyphReverseMap;

	bool valid_cmap_table = false;
	for (uint16 i = 0; i < cmap_num_tables; i++)
	{
		constexpr uint8 UNICODE_PLATFORM_INDEX = 0; constexpr uint8 WIN32_PLATFORM_INDEX = 3; constexpr uint8 WIN32_UNICODE_ENCODING = 1;
		
		uint16 platformID, encodingID;
		uint32 cmap_subtable_offset;
		get2b(&platformID, data + cmap_offset, cmap_offset);
		get2b(&encodingID, data + cmap_offset, cmap_offset);
		get4b(&cmap_subtable_offset, data + cmap_offset, cmap_offset);

		if (!((platformID == UNICODE_PLATFORM_INDEX && encodingID == 3/*\(··)/*/) || (platformID == WIN32_PLATFORM_INDEX && encodingID == WIN32_UNICODE_ENCODING))) { continue; }

		cmap_subtable_offset += cmapTableEntry->second.offsetPos;
		uint16 format, length;
		get2b(&format, data + cmap_subtable_offset, cmap_subtable_offset);
		get2b(&length, data + cmap_subtable_offset, cmap_subtable_offset);

		if (format != 4) { continue; }

		uint16 language, segCountX2;// , searchRange, entrySelector, rangeShift;
		get2b(&language, data + cmap_subtable_offset, cmap_subtable_offset);
		get2b(&segCountX2, data + cmap_subtable_offset, cmap_subtable_offset);
		//get2b(&searchRange, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		//get2b(&entrySelector, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		//get2b(&rangeShift, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		cmap_subtable_offset += sizeof(uint16) * 3;

		uint16 segCount = segCountX2 >> 1;
		std::vector<uint16> endCount(segCount), startCount(segCount), idRangeOffset(segCount);
		std::vector<int16> idDelta(segCount);
		for (uint16 j = 0; j < segCount; j++)
		{
			get2b(&endCount[j], data + cmap_subtable_offset, cmap_subtable_offset);
		}

		cmap_subtable_offset += sizeof(uint16);
		
		for (uint16 j = 0; j < segCount; j++)
		{
			get2b(&startCount[j], data + cmap_subtable_offset);
			get2b(&idDelta[j], data + cmap_subtable_offset + sizeof(uint16) * segCount);
			get2b(&idRangeOffset[j], data + cmap_subtable_offset + sizeof(uint16) * segCount * 2);
			if (idRangeOffset[j] == 0)
			{
				for (uint32 k = startCount[j]; k <= endCount[j]; k++)
				{
					fontData->GlyphMap.Emplace(k, k + idDelta[j]);
					glyphReverseMap[k + idDelta[j]] = k;
				}
			}
			else
			{
				uint32 glyph_address_offset = cmap_subtable_offset + sizeof(uint16) * segCount * 2; //idRangeOffset_ptr
				for (uint32 k = startCount[j]; k <= endCount[j]; k++)
				{
					uint32 glyph_address_index_offset = idRangeOffset[j] + 2 * (k - startCount[j]) + glyph_address_offset;
					uint16& glyph_map_value = fontData->GlyphMap[k];
					get2b(&glyph_map_value, data + glyph_address_index_offset);
					glyphReverseMap[glyph_map_value] = k;
					glyph_map_value += idDelta[j];
				}
			}

			cmap_subtable_offset += sizeof(uint16);
		}

		valid_cmap_table = true;
		break;
	}
	if (!valid_cmap_table) { return -1; }

	auto hheaTableEntry = tables.find("hhea");
	if (hheaTableEntry == tables.end()) { return -2; }
	HHEATable hheaTable;
	uint32 hhea_offset = hheaTable.Parse(data, hheaTableEntry->second.offsetPos);

	auto glyfTableEntry = tables.find("glyf");
	if (glyfTableEntry == tables.end()) { return -2; }
	uint32 glyf_offset = glyfTableEntry->second.offsetPos;

	auto kernTableEntry = tables.find("kern");
	uint32 kernOffset = 0;
	if (kernTableEntry != tables.end())
	{
		kernOffset = kernTableEntry->second.offsetPos;
	}

	auto hmtxTableEntry = tables.find("hmtx");
	if (hmtxTableEntry == tables.end()) { return -2; }
	uint32 hmtx_offset = hmtxTableEntry->second.offsetPos;
	uint16 last_glyph_advance_width = 0;

	std::vector<std::vector<uint16>> pointsPerContour(GTSL::Math::Limit(maximumProfile.maxContours, (uint16)4096));
	std::vector<uint16> pointsInContour(GTSL::Math::Limit(maximumProfile.maxContours,  (uint16)4096));
	
	GTSL::Vector<bool, BE::TAR> glyphLoaded(maximumProfile.numGlyphs, GetTransientAllocator());
	for (uint32 i = 0; i < maximumProfile.numGlyphs; ++i) {
		glyphLoaded.EmplaceBack(false);
	}
	
	auto parseGlyph = [&](uint32 glyphIndex, auto&& self) -> int8 {
		if (glyphLoaded[glyphIndex]) { return 1; }

		Glyph& currentGlyph = fontData->Glyphs.Emplace(glyphIndex, GetPersistentAllocator());
		currentGlyph.GlyphIndex = static_cast<int16>(glyphIndex);
		currentGlyph.Character = glyphReverseMap[static_cast<int16>(glyphIndex)];

		if (glyphIndex < hheaTable.numberOfHMetrics) {
			get2b(&currentGlyph.AdvanceWidth, data + hmtx_offset + glyphIndex * sizeof(uint32));
			last_glyph_advance_width = currentGlyph.AdvanceWidth;
			get2b(&currentGlyph.LeftSideBearing, data + hmtx_offset + glyphIndex * sizeof(uint32) + sizeof(uint16));
		} else {
			currentGlyph.AdvanceWidth = last_glyph_advance_width;
		}

		if (glyphIndex != maximumProfile.numGlyphs - 1 && glyphIndices[glyphIndex] == glyphIndices[glyphIndex + 1]) {
			glyphLoaded[glyphIndex] = true;
			return -1;
		}

		if (glyphIndices[glyphIndex] >= endOfGlyf) { return -1; }

		uint32 currentOffset = glyf_offset + glyphIndices[glyphIndex];

		get2b(&currentGlyph.NumContours, data + currentOffset, currentOffset);
		
		{
			int16 bbox[4];
			get2b(&bbox[0], data + currentOffset, currentOffset); /*xMin*/ get2b(&bbox[1], data + currentOffset, currentOffset); //yMin
			get2b(&bbox[2], data + currentOffset, currentOffset); /*xMax*/ get2b(&bbox[3], data + currentOffset, currentOffset); //yMax

			currentGlyph.BoundingBox[0].X() = bbox[0]; currentGlyph.BoundingBox[0].Y() = bbox[1];
			currentGlyph.BoundingBox[1].X() = bbox[2]; currentGlyph.BoundingBox[1].Y() = bbox[3];
			
			currentGlyph.Center.X() = (currentGlyph.BoundingBox[0].X() + currentGlyph.BoundingBox[1].X()) / 2.0f;
			currentGlyph.Center.Y() = (currentGlyph.BoundingBox[0].Y() + currentGlyph.BoundingBox[1].Y()) / 2.0f;
		}
		
		if (currentGlyph.NumContours > 0) //simple glyph
		{
			currentGlyph.Paths.Resize(currentGlyph.NumContours);
			
			std::vector<uint16> contourEnd(currentGlyph.NumContours);
			
			for (uint16 j = 0; j < currentGlyph.NumContours; j++) { get2b(&contourEnd[j], data + currentOffset, currentOffset); }
			
			for (uint16 contourIndex = 0; contourIndex < currentGlyph.NumContours; contourIndex++) {
				uint16 num_points = contourEnd[contourIndex] - (contourIndex ? contourEnd[contourIndex - 1] : -1);
				
				if (pointsPerContour[contourIndex].size() < num_points) {
					pointsPerContour[contourIndex].resize(num_points);
				}
				
				pointsInContour[contourIndex] = num_points;
			}

			//Skip instructions
			uint16 num_instructions;
			get2b(&num_instructions, data + currentOffset); currentOffset += sizeof(uint16);
			currentOffset += sizeof(uint8) * num_instructions;

			uint16 numPoints = contourEnd[static_cast<int64>(currentGlyph.NumContours) - 1] + 1;
			std::vector<uint8> flags(numPoints);
			std::vector<Flags> flagsEnum(numPoints);
			std::vector<uint16> contour_index(numPoints);
			uint16 current_contour_index = 0;
			int16 repeat = 0;
			uint16 contour_count_first_point = 0;
			
			for (uint16 j = 0; j < numPoints; j++, ++contour_count_first_point) {
				if (!repeat) {
					get1b(&flags[j], data + currentOffset, currentOffset);
					
					if (flags[j] & 0x8) { get1b(&repeat, data + currentOffset, currentOffset); }
				} else {
					flags[j] = flags[j - 1];
					repeat--;
				}
				
				flagsEnum[j].isControlPoint = (!(flags[j] & 0b00000001)) != 0;
				flagsEnum[j].xShort = (flags[j] & 0b00000010) != 0;
				flagsEnum[j].yShort = (flags[j] & 0b00000100) != 0;
				flagsEnum[j].repeat = (flags[j] & 0b00001000) != 0;
				flagsEnum[j].xDual = (flags[j] & 0b00010000) != 0;
				flagsEnum[j].yDual = (flags[j] & 0b00100000) != 0;
				
				if (j > contourEnd[current_contour_index])
				{
					current_contour_index++;
					contour_count_first_point = 0;
				}
				
				contour_index[j] = current_contour_index;
				pointsPerContour[current_contour_index][contour_count_first_point] = j;
			}

			std::vector<ShortVector> glyphPoints(numPoints);

			for(uint16 j = 0; j < numPoints; ++j) {
				if (flagsEnum[j].xDual && !flagsEnum[j].xShort) {
					glyphPoints[j].X = j ? glyphPoints[j - 1].X : 0;
				} else {
					if (flagsEnum[j].xShort) {
						get1b(&glyphPoints[j].X, data + currentOffset, currentOffset);
					} else {
						get2b(&glyphPoints[j].X, data + currentOffset, currentOffset);
					}

					if (flagsEnum[j].xShort && !flagsEnum[j].xDual) { glyphPoints[j].X *= -1; }

					if (j != 0) { glyphPoints[j].X += glyphPoints[j - 1].X; }
				}
			}
			
			for (uint16 j = 0; j < numPoints; j++) {				
				if (flagsEnum[j].yDual && !flagsEnum[j].yShort) {
					glyphPoints[j].Y = j ? glyphPoints[j - 1].Y : 0;
				} else {
					if (flagsEnum[j].yShort) {
						get1b(&glyphPoints[j].Y, data + currentOffset, currentOffset);
					} else {
						get2b(&glyphPoints[j].Y, data + currentOffset, currentOffset);
					}
					
					if (flagsEnum[j].yShort && !flagsEnum[j].yDual) { glyphPoints[j].Y *= -1; }
					
					if (j != 0) { glyphPoints[j].Y += glyphPoints[j - 1].Y; }
				}
			}

			//Generate contours
			for (uint16 contourIndex = 0; contourIndex < currentGlyph.NumContours; ++contourIndex) {
				currentGlyph.Paths.EmplaceBack(pointsInContour[contourIndex], GetPersistentAllocator());
				//can be broken
				
				const uint16 numPointsInContour = pointsInContour[contourIndex];

				auto& contourPointsFlags = flagsEnum;
				
				uint16 pointInIndices = 0;
				
				//If the first point is control point
				while(contourPointsFlags[pointsPerContour[contourIndex][pointInIndices]].isControlPoint) { ++pointInIndices; }
				
				bool lastPointWasControlPoint = false, thisPointIsControlPoint = false;

				Segment currentCurve;
				currentCurve.Points[0] = toVector(glyphPoints[pointsPerContour[contourIndex][pointInIndices]]); //what if no more points

				++pointInIndices;
				
				for (uint32 p = 0; p < numPointsInContour; ++p, ++pointInIndices) {
					uint32 safeIndexToData = pointsPerContour[contourIndex][pointInIndices % numPointsInContour];
					
					thisPointIsControlPoint = contourPointsFlags[safeIndexToData].isControlPoint;

					if(thisPointIsControlPoint) {
						if(lastPointWasControlPoint) {
							auto thisPoint = toVector(glyphPoints[safeIndexToData]);
							auto newPoint = (thisPoint + currentCurve.Points[1]) * 0.5f;
							currentCurve.Points[2] = newPoint;
							currentCurve.IsCurve = true;
							currentGlyph.Paths[contourIndex].EmplaceBack(currentCurve);

							currentCurve.Points[0] = newPoint;
							
							currentCurve.Points[1] = thisPoint;
						} else {
							currentCurve.Points[1] = toVector(glyphPoints[safeIndexToData]);
						}
					} else {
						if(lastPointWasControlPoint) {
							auto thisPoint = toVector(glyphPoints[safeIndexToData]);
							currentCurve.Points[2] = thisPoint;
							currentCurve.IsCurve = true;
							currentGlyph.Paths[contourIndex].EmplaceBack(currentCurve);

							currentCurve.Points[0] = thisPoint;
						} else {
							auto thisPoint = toVector(glyphPoints[safeIndexToData]);
							currentCurve.Points[1] = GTSL::Vector2(0, 0);
							currentCurve.Points[2] = thisPoint;
							currentCurve.IsCurve = false;
							currentGlyph.Paths[contourIndex].EmplaceBack(currentCurve);

							currentCurve.Points[0] = thisPoint;
						}
					}
					
					lastPointWasControlPoint = thisPointIsControlPoint;
				}
			} //for contour
		} else { //Composite glyph
			for (auto compound_glyph_index = 0; compound_glyph_index < -currentGlyph.NumContours; compound_glyph_index++) {
				uint16 glyfFlags, glyphIndex;

				do {
					get2b(&glyfFlags, data + currentOffset); currentOffset += sizeof(uint16);
					get2b(&glyphIndex, data + currentOffset); currentOffset += sizeof(uint16);

					int16 glyfArgs1 = 0, glyfArgs2 = 0;
					int8_t glyfArgs1U8 = 0, glyfArgs2U8 = 0;
					bool is_word = false;
					if (glyfFlags & ARG_1_AND_2_ARE_WORDS)
					{
						get2b(&glyfArgs1, data + currentOffset); currentOffset += sizeof(int16);
						get2b(&glyfArgs2, data + currentOffset); currentOffset += sizeof(int16);
						is_word = true;
					}
					else
					{
						get1b(&glyfArgs1U8, data + currentOffset); currentOffset += sizeof(int8_t);
						get1b(&glyfArgs2U8, data + currentOffset); currentOffset += sizeof(int8_t);
					}

					float32 compositeGlyphElementTransformation[6] = { 1.0f, 0.0f, 0.0f, 1.0f, 0.0f, 0.0f };

					if (glyfFlags & WE_HAVE_A_SCALE)
					{
						int16 xy_value = 0;
						get2b(&xy_value, data + currentOffset); currentOffset += sizeof(int16);
						compositeGlyphElementTransformation[0] = to_2_14_float(xy_value);
						compositeGlyphElementTransformation[3] = to_2_14_float(xy_value);
					}
					else if (glyfFlags & WE_HAVE_AN_X_AND_Y_SCALE)
					{
						int16 xy_values[2];
						get2b(&xy_values[0], data + currentOffset); currentOffset += sizeof(int16);
						get2b(&xy_values[1], data + currentOffset); currentOffset += sizeof(int16);
						compositeGlyphElementTransformation[0] = to_2_14_float(xy_values[0]);
						compositeGlyphElementTransformation[3] = to_2_14_float(xy_values[1]);
					}
					else if (glyfFlags & WE_HAVE_A_TWO_BY_TWO)
					{
						int16 xy_values[4];
						get2b(&xy_values[0], data + currentOffset); currentOffset += sizeof(int16);
						get2b(&xy_values[1], data + currentOffset); currentOffset += sizeof(int16);
						get2b(&xy_values[2], data + currentOffset); currentOffset += sizeof(int16);
						get2b(&xy_values[3], data + currentOffset); currentOffset += sizeof(int16);
						compositeGlyphElementTransformation[0] = to_2_14_float(xy_values[0]);
						compositeGlyphElementTransformation[1] = to_2_14_float(xy_values[1]);
						compositeGlyphElementTransformation[2] = to_2_14_float(xy_values[2]);
						compositeGlyphElementTransformation[3] = to_2_14_float(xy_values[3]);
					}

					bool matched_points = false;
					if (glyfFlags & ARGS_ARE_XY_VALUES)
					{
						compositeGlyphElementTransformation[4] = is_word ? glyfArgs1 : glyfArgs1U8;
						compositeGlyphElementTransformation[5] = is_word ? glyfArgs2 : glyfArgs2U8;
						if (glyfFlags & SCALED_COMPONENT_OFFSET)
						{
							compositeGlyphElementTransformation[4] *= compositeGlyphElementTransformation[0];
							compositeGlyphElementTransformation[5] *= compositeGlyphElementTransformation[3];
						}
					}
					else
					{
						matched_points = true;
					}

					//Skip instructions
					if (glyfFlags & WE_HAVE_INSTRUCTIONS)
					{
						uint16 num_instructions = 0;
						get2b(&num_instructions, data + currentOffset); currentOffset += sizeof(uint16);
						currentOffset += sizeof(uint8_t) * num_instructions;
					}

					if (glyphLoaded[glyphIndex] == false)
					{
						if (self(glyphIndex, self) < 0) {
							BE_LOG_WARNING("ttf-parser: bad glyph index ", glyphIndex, " in composite glyph");
							continue;
						}
					}
					
					Glyph& compositeGlyphElement = fontData->Glyphs[glyphIndex];

					auto transformCurve = [&compositeGlyphElementTransformation](Segment& curve) -> Segment
					{
						Segment out;
						out.Points[0].X() = curve.Points[0].X() * compositeGlyphElementTransformation[0] + curve.Points[0].X() * compositeGlyphElementTransformation[1] + compositeGlyphElementTransformation[4];
						out.Points[0].Y() = curve.Points[0].Y() * compositeGlyphElementTransformation[2] + curve.Points[0].Y() * compositeGlyphElementTransformation[3] + compositeGlyphElementTransformation[5];
						out.Points[1].X() = curve.Points[1].X() * compositeGlyphElementTransformation[0] + curve.Points[1].Y() * compositeGlyphElementTransformation[1] + compositeGlyphElementTransformation[4];
						out.Points[1].Y() = curve.Points[1].X() * compositeGlyphElementTransformation[2] + curve.Points[1].Y() * compositeGlyphElementTransformation[3] + compositeGlyphElementTransformation[5];
						out.Points[2].X() = curve.Points[2].X() * compositeGlyphElementTransformation[0] + curve.Points[2].Y() * compositeGlyphElementTransformation[1] + compositeGlyphElementTransformation[4];
						out.Points[2].Y() = curve.Points[2].X() * compositeGlyphElementTransformation[2] + curve.Points[2].Y() * compositeGlyphElementTransformation[3] + compositeGlyphElementTransformation[5];
						return out;
					};

					uint32 compositeGlyphPathCount = compositeGlyphElement.Paths.GetLength();
					for (uint32 glyphPointIndex = 0; glyphPointIndex < compositeGlyphPathCount; glyphPointIndex++) {
						auto& currentCurvesList = compositeGlyphElement.Paths[glyphPointIndex];

						uint32 compositeGlyphPathCurvesCount = currentCurvesList.GetLength();

						Path newPath(GetPersistentAllocator());
						if (!matched_points) {
							newPath.Resize(compositeGlyphPathCurvesCount);

							for (uint32 glyphCurvesPointIndex = 0; glyphCurvesPointIndex < compositeGlyphPathCurvesCount; glyphCurvesPointIndex++) {
								newPath.EmplaceBack(transformCurve(currentCurvesList[glyphCurvesPointIndex]));
							}
						} else {
							BE_LOG_WARNING("ttf-parser: unsupported matched points in ttf composite glyph");
							continue;
						}

						currentGlyph.Paths.EmplaceBack(newPath);
					}
				} while (glyfFlags & MORE_COMPONENTS);
			}
		}

		glyphLoaded[glyphIndex] = true;

		return 0;
	};

	for (uint16 i = 0; i < maximumProfile.numGlyphs; i++) { parseGlyph(i, parseGlyph); }

	//Kerning table
	if (kernOffset)
	{
		uint32 currentOffset = kernOffset;
		uint16 kern_table_version, num_kern_subtables;
		get2b(&kern_table_version, data + currentOffset, currentOffset);
		get2b(&num_kern_subtables, data + currentOffset, currentOffset);
		uint16 kern_length = 0;
		uint32 kernStartOffset = currentOffset;

		for (uint16 kerningSubTableIndex = 0; kerningSubTableIndex < num_kern_subtables; kerningSubTableIndex++)
		{
			uint16 kerningVersion, kerningCoverage;
			currentOffset = kernStartOffset + kern_length;
			kernStartOffset = currentOffset;
			get2b(&kerningVersion, data + currentOffset, currentOffset);
			get2b(&kern_length, data + currentOffset, currentOffset);
			if (kerningVersion != 0)
			{
				currentOffset += kern_length - sizeof(uint16) * 3;
				continue;
			}
			get2b(&kerningCoverage, data + currentOffset, currentOffset);

			uint16 num_kern_pairs;
			get2b(&num_kern_pairs, data + currentOffset, currentOffset);
			currentOffset += sizeof(uint16) * 3;
			for (uint16 kern_index = 0; kern_index < num_kern_pairs; kern_index++)
			{
				uint16 kern_left, kern_right;
				int16 kern_value;
				get2b(&kern_left, data + currentOffset, currentOffset);
				get2b(&kern_right, data + currentOffset, currentOffset);
				get2b(&kern_value, data + currentOffset, currentOffset);

				fontData->KerningTable.Emplace((kern_left << 16) | kern_right, kern_value);
			}
		}
	}

	fontData->Metadata.UnitsPerEm = headTable.unitsPerEm;
	fontData->Metadata.Ascender = hheaTable.Ascender;
	fontData->Metadata.Descender = hheaTable.Descender;
	fontData->Metadata.LineGap = hheaTable.LineGap;

	return 0;
}