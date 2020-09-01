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

#include <cstdint>
#include <map>
#include <unordered_map>
#include <vector>
#include <fstream>

#include "ByteEngine/Core.h"
#ifdef _DEBUG
#include <stdio.h>
#define TTFDEBUG_PRINT(...) printf(__VA_ARGS__)
#else
#define TTFDEBUG_PRINT(...) {}
#endif

#include <GTSL/Buffer.h>
#include <GTSL/Math/Vector2.h>


#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

using namespace GTSL;

typedef void(*TTF_FONT_MEM_CPY)(void*, const char*);
extern TTF_FONT_MEM_CPY get2b;
extern TTF_FONT_MEM_CPY get4b;
extern TTF_FONT_MEM_CPY get8b;
extern uint32 little_endian_test;
extern bool endian_tested;

extern void get4b_be(void* dst, const char* src);
extern void get4b_le(void* dst, const char* src);
extern void get8b_be(void* dst, const char* src);
extern void get8b_le(void* dst, const char* src);
extern void get2b_be(void* dst, const char* src);
extern void get2b_le(void* dst, const char* src);
extern void get1b(void* dst, const char* src);
extern float32 to_2_14_float(int16 value);

struct Flags
{
	bool xDual;
	bool yDual;
	bool xShort;
	bool yShort;
	bool repeat;
	bool offCurve;
};

enum COMPOUND_GLYPH_FLAGS
{
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

struct TTFHeader
{
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

	uint32 parse(const char* data, uint32 offset) {
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
			UTF8* newNameString = new UTF8[NameRecords[i].length];
			memcpy(newNameString, data + offset_start + stringOffset + NameRecords[i].offset_value, sizeof(UTF8) * NameRecords[i].length);
			uint16 string_length = NameRecords[i].length;
			
			if (newNameString[0] == 0)
			{
				string_length = string_length >> 1;
				
				for (uint16 j = 0; j < string_length; j++)
				{
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

struct ShortVector
{
	int16 X, Y;
};

struct FontLineInfoData
{
	uint32 StringStartIndex;
	uint32 StringEndIndex;
	Vector2 OffsetStart;
	Vector2 OffsetEnd;
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

extern int16 GetKerningOffset(FontResourceManager::Font* font_data, uint16 left_glyph, uint16 right_glyph);

TTF_FONT_MEM_CPY get2b = get2b_le;
TTF_FONT_MEM_CPY get4b = get4b_le;
TTF_FONT_MEM_CPY get8b = get8b_le;
uint32 little_endian_test = 0x01234567;
bool endian_tested = false;

//Copying functions for big and little endian
void get4b_be(void* dst, const char* src)
{
	static_cast<uint8_t*>(dst)[0] = src[0];
	static_cast<uint8_t*>(dst)[1] = src[1];
	static_cast<uint8_t*>(dst)[2] = src[2];
	static_cast<uint8_t*>(dst)[3] = src[3];
}
void get4b_le(void* dst, const char* src)
{
	static_cast<uint8_t*>(dst)[0] = src[3];
	static_cast<uint8_t*>(dst)[1] = src[2];
	static_cast<uint8_t*>(dst)[2] = src[1];
	static_cast<uint8_t*>(dst)[3] = src[0];
}
void get8b_be(void* dst, const char* src)
{
	static_cast<uint8_t*>(dst)[0] = src[0];
	static_cast<uint8_t*>(dst)[1] = src[1];
	static_cast<uint8_t*>(dst)[2] = src[2];
	static_cast<uint8_t*>(dst)[3] = src[3];
	static_cast<uint8_t*>(dst)[4] = src[4];
	static_cast<uint8_t*>(dst)[5] = src[5];
	static_cast<uint8_t*>(dst)[6] = src[6];
	static_cast<uint8_t*>(dst)[7] = src[7];
}
void get8b_le(void* dst, const char* src)
{
	static_cast<uint8_t*>(dst)[0] = src[7];
	static_cast<uint8_t*>(dst)[1] = src[6];
	static_cast<uint8_t*>(dst)[2] = src[5];
	static_cast<uint8_t*>(dst)[3] = src[4];
	static_cast<uint8_t*>(dst)[4] = src[3];
	static_cast<uint8_t*>(dst)[5] = src[2];
	static_cast<uint8_t*>(dst)[6] = src[1];
	static_cast<uint8_t*>(dst)[7] = src[0];
}
void get2b_be(void* dst, const char* src) {
	static_cast<uint8_t*>(dst)[0] = src[0];
	static_cast<uint8_t*>(dst)[1] = src[1];
}
void get2b_le(void* dst, const char* src) {
	static_cast<uint8_t*>(dst)[0] = src[1];
	static_cast<uint8_t*>(dst)[1] = src[0];
}
void get1b(void* dst, const char* src)
{
	static_cast<uint8_t*>(dst)[0] = src[0];
}
float32 to_2_14_float(const int16 value)
{
	return static_cast<float32>(value & 0x3fff) / static_cast<float32>(1 << 14) + (-2 * ((value >> 15) & 0x1) + ((value >> 14) & 0x1));
}

int16 GetKerningOffset(FontResourceManager::Font* font_data, uint16 left_glyph, uint16 right_glyph)
{
	auto kern_data = font_data->KerningTable.find((left_glyph << 16) | right_glyph);
	return (kern_data == font_data->KerningTable.end()) ? 0 : kern_data->second;
}

FontResourceManager::Font FontResourceManager::GetFont(const Ranger<const UTF8> fontName)
{
	StaticString<255> path(BE::Application::Get()->GetPathToApplication()); path += "/resources/"; path += fontName; path += ".ttf";

	File fontFile; fontFile.OpenFile(path, static_cast<uint8>(File::AccessMode::READ), File::OpenMode::LEAVE_CONTENTS);
	Buffer fileBuffer; fileBuffer.Allocate(fontFile.GetFileSize(), 8, GetTransientAllocator());

	fontFile.ReadFile(fileBuffer);
	
	Font fontData;
	const auto result = parseData(reinterpret_cast<const char*>(fileBuffer.GetData()), &fontData);
	BE_ASSERT(result > -1, "Failed to parse!")
	
	fileBuffer.Free(8, GetTransientAllocator());
	fontFile.CloseFile();
	
	return fontData;
}

int8 FontResourceManager::parseData(const char* data, Font* fontData)
{
	if (endian_tested == false)
	{
		if (*reinterpret_cast<uint8_t*>(&little_endian_test) == 0x67 == true) {
			get2b = get2b_le; get4b = get4b_le; get8b = get8b_le;
		}
		else
		{
			get2b = get2b_be; get4b = get4b_be; get8b = get8b_be;
		}
		endian_tested = true;
	}

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

	auto head_table_entry = tables.find("head");
	if (head_table_entry == tables.end()) { return -2; }

	HeadTable headTable;
	ptr = headTable.Parse(data, head_table_entry->second.offsetPos);
	auto maxp_table_entry = tables.find("maxp");
	if (maxp_table_entry == tables.end()) { return -2; }

	MaximumProfile max_profile;
	max_profile.parse(data, maxp_table_entry->second.offsetPos);
	auto name_table_entry = tables.find("name");
	if (name_table_entry == tables.end()) { return -2; }

	NameTable name_table;
	name_table.Parse(data, name_table_entry->second.offsetPos, fontData->NameTable);

	fontData->FullFontName = fontData->NameTable[1] + " " + fontData->NameTable[2];

	auto loca_table_entry = tables.find("loca");
	if (loca_table_entry == tables.end()) { return -2; }

	std::vector<uint32> glyph_index(max_profile.numGlyphs);

	uint32 end_of_glyf = 0;

	if (headTable.indexToLocFormat == 0)
	{
		uint32 byte_offset = loca_table_entry->second.offsetPos;

		for (uint16 i = 0; i < max_profile.numGlyphs; i++, byte_offset += sizeof(uint16))
		{
			get2b(&glyph_index[i], data + byte_offset);
			glyph_index[i] = glyph_index[i] << 1;
		}

		get2b(&end_of_glyf, data + byte_offset);
		end_of_glyf = end_of_glyf << 1;
	}
	else
	{
		uint32 byte_offset = loca_table_entry->second.offsetPos;
		for (uint16 i = 0; i < max_profile.numGlyphs; i++, byte_offset += sizeof(uint32))
		{
			get4b(&glyph_index[i], data + byte_offset);
		}
		get4b(&end_of_glyf, data + byte_offset);
	}

	auto cmap_table_entry = tables.find("cmap");
	if (cmap_table_entry == tables.end()) { return -2; }

	uint32 cmap_offset = cmap_table_entry->second.offsetPos + sizeof(uint16); //Skip version
	uint16 cmap_num_tables;
	get2b(&cmap_num_tables, data + cmap_offset); cmap_offset += sizeof(uint16);

	std::map<uint16, uint32> glyphReverseMap;

	bool valid_cmap_table = false;
	for (uint16 i = 0; i < cmap_num_tables; i++)
	{
		constexpr uint8 UNICODE_PLATFORM_INDEX = 0; constexpr uint8 WIN32_PLATFORM_INDEX = 3; constexpr uint8 WIN32_UNICODE_ENCODING = 1;
		
		uint16 platformID, encodingID;
		uint32 cmap_subtable_offset;
		get2b(&platformID, data + cmap_offset); cmap_offset += sizeof(uint16);
		get2b(&encodingID, data + cmap_offset); cmap_offset += sizeof(uint16);
		get4b(&cmap_subtable_offset, data + cmap_offset); cmap_offset += sizeof(uint32);

		if (!((platformID == UNICODE_PLATFORM_INDEX && encodingID == 3/*\(··)/*/) || (platformID == WIN32_PLATFORM_INDEX && encodingID == WIN32_UNICODE_ENCODING))) { continue; }

		cmap_subtable_offset += cmap_table_entry->second.offsetPos;
		uint16 format, length;
		get2b(&format, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		get2b(&length, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);

		if (format != 4) { continue; }

		uint16 language, segCountX2;// , searchRange, entrySelector, rangeShift;
		get2b(&language, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		get2b(&segCountX2, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		//get2b(&searchRange, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		//get2b(&entrySelector, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		//get2b(&rangeShift, data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
		cmap_subtable_offset += sizeof(uint16) * 3;

		uint16 segCount = segCountX2 >> 1;
		std::vector<uint16> endCount(segCount), startCount(segCount), idRangeOffset(segCount);
		std::vector<int16> idDelta(segCount);
		for (uint16 j = 0; j < segCount; j++)
		{
			get2b(&endCount[j], data + cmap_subtable_offset); cmap_subtable_offset += sizeof(uint16);
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
					fontData->GlyphMap[k] = k + idDelta[j];
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
	if (!valid_cmap_table) { TTFDEBUG_PRINT("ttf-parser: No valid cmap table found\n"); }

	HHEATable hheaTable;
	auto hhea_table_entry = tables.find("hhea");
	if (hhea_table_entry == tables.end()) { return -2; }
	uint32 hhea_offset = hheaTable.Parse(data, hhea_table_entry->second.offsetPos);

	auto glyf_table_entry = tables.find("glyf");
	{
		if (glyf_table_entry == tables.end()) { return -2; }
	}
	uint32 glyf_offset = glyf_table_entry->second.offsetPos;

	auto kern_table_entry = tables.find("kern");
	uint32 kernOffset = 0;
	if (kern_table_entry != tables.end())
	{
		kernOffset = kern_table_entry->second.offsetPos;
	}

	auto hmtx_table_entry = tables.find("hmtx");
	if (hmtx_table_entry == tables.end())
	{
		return -2;
	}
	uint32 hmtx_offset = hmtx_table_entry->second.offsetPos;
	uint16 last_glyph_advance_width = 0;

	std::vector<std::vector<uint16>> point_index((max_profile.maxContours < 4096) ? max_profile.maxContours : 4096);
	std::vector<uint16> points_per_contour((max_profile.maxContours < 4096) ? max_profile.maxContours : 4096);

	if (!max_profile.numGlyphs)
	{
		return -1;
	}

	bool* glyphLoaded = new bool[max_profile.numGlyphs];
	memset(glyphLoaded, 0, sizeof(bool) * max_profile.numGlyphs);

	auto parseGlyph = [&](uint16 i, auto&& self) -> int8
	{
		if (glyphLoaded[i] == true) { return 1; }

		Glyph& currentGlyph = fontData->Glyphs[i]; //when replacing for own map remember to emplace first, std []operator try_emplaces
		currentGlyph.PathList.Initialize(3, GetPersistentAllocator());
		currentGlyph.GlyphIndex = i;
		currentGlyph.Character = glyphReverseMap[i];
		currentGlyph.NumTriangles = 0;

		if (i < hheaTable.numberOfHMetrics)
		{
			get2b(&currentGlyph.AdvanceWidth, data + hmtx_offset + i * sizeof(uint32));
			last_glyph_advance_width = currentGlyph.AdvanceWidth;
			get2b(&currentGlyph.LeftSideBearing, data + hmtx_offset + i * sizeof(uint32) + sizeof(uint16));
		}
		else
		{
			currentGlyph.AdvanceWidth = last_glyph_advance_width;
		}

		if (i != max_profile.numGlyphs - 1 && glyph_index[i] == glyph_index[i + 1])
		{
			glyphLoaded[i] = true;
			return -1;
		}

		if (glyph_index[i] >= end_of_glyf) { return -1; }

		uint32 currentOffset = glyf_offset + glyph_index[i];

		get2b(&currentGlyph.NumContours, data + currentOffset); currentOffset += sizeof(int16);
		get2b(&currentGlyph.BoundingBox[0], data + currentOffset); currentOffset += sizeof(int16);
		get2b(&currentGlyph.BoundingBox[1], data + currentOffset); currentOffset += sizeof(int16);
		get2b(&currentGlyph.BoundingBox[2], data + currentOffset); currentOffset += sizeof(int16);
		get2b(&currentGlyph.BoundingBox[3], data + currentOffset); currentOffset += sizeof(int16);

		Vector2 glyphCenter;
		glyphCenter.X = (currentGlyph.BoundingBox[0] + currentGlyph.BoundingBox[2]) / 2.0f;
		glyphCenter.Y = (currentGlyph.BoundingBox[1] + currentGlyph.BoundingBox[3]) / 2.0f;

		if (currentGlyph.NumContours > 0)
		{ //Simple glyph
			std::vector<uint16> contourEnd(currentGlyph.NumContours);
			//currentGlyph.PathList.Resize(currentGlyph.NumContours);
			//don't resize
			//code expects resize to leave valid elements which our vector doesn't
			//emplace elements as needed later to ensure valid elements
			for (uint16 j = 0; j < currentGlyph.NumContours; j++)
			{
				get2b(&contourEnd[j], data + currentOffset); currentOffset += sizeof(uint16);
			}
			for (uint16 j = 0; j < currentGlyph.NumContours; j++)
			{
				uint16 num_points = contourEnd[j] - (j ? contourEnd[j - 1] : -1);
				if (point_index[j].size() < num_points)
				{
					point_index[j].resize(num_points);
				}
				points_per_contour[j] = num_points;
			}

			//Skip instructions
			uint16 num_instructions;
			get2b(&num_instructions, data + currentOffset); currentOffset += sizeof(uint16);
			currentOffset += sizeof(uint8_t) * num_instructions;

			uint16 num_points = contourEnd[currentGlyph.NumContours - 1] + 1;
			std::vector<uint8_t> flags(num_points);
			std::vector<::Flags> flagsEnum(num_points);
			std::vector<uint16> contour_index(num_points);
			uint16 current_contour_index = 0;
			int16 repeat = 0;
			uint16 coutour_count_first_point = 0;
			for (uint16 j = 0; j < num_points; j++, coutour_count_first_point++)
			{
				if (repeat == 0)
				{
					get1b(&flags[j], data + currentOffset); currentOffset += sizeof(uint8_t);
					if (flags[j] & 0x8)
					{
						get1b(&repeat, data + currentOffset); currentOffset += sizeof(uint8_t);
					}
				}
				else
				{
					flags[j] = flags[j - 1];
					repeat--;
				}
				flagsEnum[j].offCurve = (!(flags[j] & 0b00000001)) != 0;
				flagsEnum[j].xShort = (flags[j] & 0b00000010) != 0;
				flagsEnum[j].yShort = (flags[j] & 0b00000100) != 0;
				flagsEnum[j].repeat = (flags[j] & 0b00001000) != 0;
				flagsEnum[j].xDual = (flags[j] & 0b00010000) != 0;
				flagsEnum[j].yDual = (flags[j] & 0b00100000) != 0;
				if (j > contourEnd[current_contour_index])
				{
					current_contour_index++;
					coutour_count_first_point = 0;
				}
				contour_index[j] = current_contour_index;
				point_index[current_contour_index][coutour_count_first_point] = j;
			}

			std::vector<ShortVector> points;
			points.resize(num_points);
			for (uint16 j = 0; j < num_points; j++)
			{
				if (flagsEnum[j].xDual && !flagsEnum[j].xShort)
				{
					points[j].X = j ? points[j - 1].X : 0;
				}
				else
				{
					if (flagsEnum[j].xShort)
					{
						get1b(&points[j].X, data + currentOffset); currentOffset += 1;
					}
					else
					{
						get2b(&points[j].X, data + currentOffset); currentOffset += 2;
					}
					if (flagsEnum[j].xShort && !flagsEnum[j].xDual)
					{
						points[j].X *= -1;
					}
					if (j != 0)
					{
						points[j].X += points[j - 1].X;
					}
				}
			}
			for (uint16 j = 0; j < num_points; j++)
			{
				if (flagsEnum[j].yDual && !flagsEnum[j].yShort)
				{
					points[j].Y = j ? points[j - 1].Y : 0;
				}
				else
				{
					if (flagsEnum[j].yShort)
					{
						get1b(&points[j].Y, data + currentOffset); currentOffset += 1;
					}
					else
					{
						get2b(&points[j].Y, data + currentOffset); currentOffset += 2;
					}
					if (flagsEnum[j].yShort && !flagsEnum[j].yDual)
					{
						points[j].Y *= -1;
					}
					if (j != 0)
					{
						points[j].Y += points[j - 1].Y;
					}
				}
			}

			//Generate contours
			for (uint16 path = 0; path < currentGlyph.NumContours; ++path)
			{
				currentGlyph.PathList.EmplaceBack();
				currentGlyph.PathList[path].Curves.Initialize(64, GetPersistentAllocator());
				
				const uint16& numPointsPerContour = points_per_contour[path];
				Vector2 prev_point;
				const uint16& point_index_0 = point_index[path][0];
				const ::Flags& flags_0 = flagsEnum[point_index_0];
				
				//If the first point is off curve
				if (flags_0.offCurve)
				{
					const uint16& point_index_m1 = point_index[path][numPointsPerContour - 1];
					const ::Flags& flags_m1 = flagsEnum[point_index_m1];
					const ShortVector& p0 = points[point_index_0];
					const ShortVector& pm1 = points[point_index_m1];
					if (flags_m1.offCurve)
					{
						prev_point.X = (p0.X + pm1.X) / 2.0f;
						prev_point.Y = (p0.Y + pm1.Y) / 2.0f;
					}
					else
					{
						prev_point.X = pm1.X;
						prev_point.Y = pm1.Y;
					}
				}
				for (uint16 pointInContour = 0; pointInContour < numPointsPerContour; pointInContour++)
				{
					const uint16& point_index0 = point_index[path][pointInContour % numPointsPerContour];
					const uint16& point_index1 = point_index[path][(pointInContour + 1) % numPointsPerContour];
					const ::Flags& flags0 = flagsEnum[point_index0];
					const ::Flags& flags1 = flagsEnum[point_index1];
					const ShortVector& p0 = points[point_index0];
					const ShortVector& p1 = points[point_index1];

					Curve curve;

					if (flags0.offCurve)
					{
						curve.p0.X = prev_point.X;
						curve.p0.Y = prev_point.Y;
						curve.p1.X = p0.X;
						curve.p1.Y = p0.Y;

						if (flags1.offCurve)
						{
							curve.p2.X = (p0.X + p1.X) / 2.0f;
							curve.p2.Y = (p0.Y + p1.Y) / 2.0f;

							prev_point = curve.p2;
						}
						else
						{
							curve.p2.X = p1.X;
							curve.p2.Y = p1.Y;
							//No change to prev_point
						}
					}
					else if (!flags1.offCurve)
					{
						curve.p0.X = p0.X;
						curve.p0.Y = p0.Y;
						curve.p1.X = p1.X;
						curve.p1.Y = p1.Y;
						curve.p2.X = glyphCenter.X + 0.5f;
						curve.p2.Y = glyphCenter.Y + 0.5f;

						prev_point.X = p0.X;
						prev_point.Y = p0.Y;
					}
					else
					{
						const uint16& point_index2 = point_index[path][(pointInContour + 2) % numPointsPerContour];
						const ::Flags& flags2 = flagsEnum[point_index2];
						const ShortVector& p2 = points[point_index2];

						if (flags2.offCurve)
						{
							curve.p0.X = p0.X;
							curve.p0.Y = p0.Y;
							curve.p1.X = p1.X;
							curve.p1.Y = p1.Y;
							curve.p2.X = (p1.X + p2.X) / 2.0f;
							curve.p2.Y = (p1.Y + p2.Y) / 2.0f;

							prev_point = curve.p2;

						}
						else
						{
							curve.p0.X = p0.X;
							curve.p0.Y = p0.Y;
							curve.p1.X = p1.X;
							curve.p1.Y = p1.Y;
							curve.p2.X = p2.X;
							curve.p2.Y = p2.Y;

							prev_point.X = p0.X;
							prev_point.Y = p0.Y;
						}
					}
					if (flags0.offCurve || flags1.offCurve)
					{
						curve.IsCurve = true;
						Curve lineCurve;
						lineCurve.IsCurve = false;
						lineCurve.p0.X = curve.p0.X;
						lineCurve.p0.Y = curve.p0.Y;
						lineCurve.p1.X = curve.p2.X;
						lineCurve.p1.Y = curve.p2.Y;
						lineCurve.p2.X = glyphCenter.X + 0.5f;
						lineCurve.p2.Y = glyphCenter.Y + 0.5f;
						currentGlyph.PathList[path].Curves.PushBack(lineCurve);
						if (flags0.offCurve == false) { ++pointInContour; }
					}
					else
					{
						curve.IsCurve = false;
					}
					
					currentGlyph.PathList[path].Curves.PushBack(curve);
				}
				
				currentGlyph.NumTriangles += static_cast<uint32>(currentGlyph.PathList[path].Curves.GetLength());
			}
		}
		else
		{ //Composite glyph
			for (auto compound_glyph_index = 0; compound_glyph_index < -currentGlyph.NumContours; compound_glyph_index++)
			{
				uint16 glyfFlags, glyphIndex;

				do
				{
					get2b(&glyfFlags, data + currentOffset); currentOffset += sizeof(uint16);
					get2b(&glyphIndex, data + currentOffset); currentOffset += sizeof(uint16);

					int16 glyfArgs1, glyfArgs2;
					int8_t glyfArgs1U8, glyfArgs2U8;
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
						int16 xy_value;
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
							TTFDEBUG_PRINT("ttf-parser: bad glyph index %d in composite glyph\n", glyphIndex);
							continue;
						}
					}
					
					Glyph& compositeGlyphElement = fontData->Glyphs[glyphIndex];

					auto transformCurve = [&compositeGlyphElementTransformation](Curve& curve) -> Curve
					{
						Curve out;
						out.p0.X = curve.p0.X * compositeGlyphElementTransformation[0] + curve.p0.Y * compositeGlyphElementTransformation[1] + compositeGlyphElementTransformation[4];
						out.p0.Y = curve.p0.X * compositeGlyphElementTransformation[2] + curve.p0.Y * compositeGlyphElementTransformation[3] + compositeGlyphElementTransformation[5];
						out.p1.X = curve.p1.X * compositeGlyphElementTransformation[0] + curve.p1.Y * compositeGlyphElementTransformation[1] + compositeGlyphElementTransformation[4];
						out.p1.Y = curve.p1.X * compositeGlyphElementTransformation[2] + curve.p1.Y * compositeGlyphElementTransformation[3] + compositeGlyphElementTransformation[5];
						out.p2.X = curve.p2.X * compositeGlyphElementTransformation[0] + curve.p2.Y * compositeGlyphElementTransformation[1] + compositeGlyphElementTransformation[4];
						out.p2.Y = curve.p2.X * compositeGlyphElementTransformation[2] + curve.p2.Y * compositeGlyphElementTransformation[3] + compositeGlyphElementTransformation[5];
						return out;
					};

					uint32 compositeGlyphPathCount = compositeGlyphElement.PathList.GetLength();
					for (uint32 glyphPointIndex = 0; glyphPointIndex < compositeGlyphPathCount; glyphPointIndex++)
					{
						GTSL::Vector<Curve, BE::PersistentAllocatorReference>& currentCurvesList = compositeGlyphElement.PathList[glyphPointIndex].Curves;

						uint32 compositeGlyphPathCurvesCount = currentCurvesList.GetLength();

						Path newPath;
						if (matched_points == false)
						{
							newPath.Curves.Initialize(compositeGlyphPathCurvesCount, GetPersistentAllocator());

							for (uint32 glyphCurvesPointIndex = 0; glyphCurvesPointIndex < compositeGlyphPathCurvesCount; glyphCurvesPointIndex++)
							{
								newPath.Curves.EmplaceBack(transformCurve(currentCurvesList[glyphCurvesPointIndex]));
							}
						}
						else
						{
							TTFDEBUG_PRINT("ttf-parser: unsupported matched points in ttf composite glyph\n");
							continue;
						}

						currentGlyph.PathList.EmplaceBack(newPath);
					}

					currentGlyph.NumTriangles += compositeGlyphElement.NumTriangles;
				} while (glyfFlags & MORE_COMPONENTS);
			}
		}

		glyphLoaded[i] = true;

		return 0;
	};

	for (uint16 i = 0; i < max_profile.numGlyphs; i++)
	{
		parseGlyph(i, parseGlyph);
	}

	delete[] glyphLoaded;

	//Kerning table
	if (kernOffset)
	{
		uint32 currentOffset = kernOffset;
		uint16 kern_table_version, num_kern_subtables;
		get2b(&kern_table_version, data + currentOffset); currentOffset += sizeof(uint16);
		get2b(&num_kern_subtables, data + currentOffset); currentOffset += sizeof(uint16);
		uint16 kern_length = 0;
		uint32 kernStartOffset = currentOffset;

		for (uint16 kerningSubTableIndex = 0; kerningSubTableIndex < num_kern_subtables; kerningSubTableIndex++)
		{
			uint16 kerningVersion, kerningCoverage;
			currentOffset = kernStartOffset + kern_length;
			kernStartOffset = currentOffset;
			get2b(&kerningVersion, data + currentOffset); currentOffset += sizeof(uint16);
			get2b(&kern_length, data + currentOffset); currentOffset += sizeof(uint16);
			if (kerningVersion != 0)
			{
				currentOffset += kern_length - sizeof(uint16) * 3;
				continue;
			}
			get2b(&kerningCoverage, data + currentOffset); currentOffset += sizeof(uint16);

			uint16 num_kern_pairs;
			get2b(&num_kern_pairs, data + currentOffset); currentOffset += sizeof(uint16);
			currentOffset += sizeof(uint16) * 3;
			for (uint16 kern_index = 0; kern_index < num_kern_pairs; kern_index++)
			{
				uint16 kern_left, kern_right;
				int16 kern_value;
				get2b(&kern_left, data + currentOffset); currentOffset += sizeof(uint16);
				get2b(&kern_right, data + currentOffset); currentOffset += sizeof(uint16);
				get2b(&kern_value, data + currentOffset); currentOffset += sizeof(int16);

				fontData->KerningTable[(kern_left << 16) | kern_right] = kern_value;
			}
		}
	}

	fontData->Metadata.UnitsPerEm = headTable.unitsPerEm;
	fontData->Metadata.Ascender = hheaTable.Ascender;
	fontData->Metadata.Descender = hheaTable.Descender;
	fontData->Metadata.LineGap = hheaTable.LineGap;

	return 0;
}
