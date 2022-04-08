#pragma once

#include <GTSL/Buffer.hpp>
#include <GTSL/Extent.h>
#include <GTSL/HashMap.hpp>

#include "ResourceManager.h"
#include "ByteEngine/Core.h"

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
	
	struct Character {
		GTSL::Extent2D Size;       // Size of glyph
		IVector2D Bearing;    // Address from baseline to left/top of glyph
		GTSL::Extent2D Position;
		uint32 Advance;    // Address to advance to next glyph
	};

	static constexpr char8_t ALPHABET[] = u8"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
	static constexpr uint64 SIZE = sizeof(ALPHABET);

	struct FontData : SData {
		DEFINE_ARRAY_MEMBER(Character, Characters, SIZE)
	};

	GTSL::Pair<FontData, GTSL::Buffer<BE::PAR>> GetFont(const GTSL::StringView string_view) {
		FontData fontData;
		resource_files_.LoadEntry(string_view, fontData);
		GTSL::Buffer buffer(GetPersistentAllocator());
		resource_files_.LoadData(fontData, buffer);
		return GTSL::Pair(GTSL::MoveRef(fontData), GTSL::MoveRef(buffer));
	}

private:
	ResourceFiles resource_files_;
	//int8 parseData(const char* data, Font* fontData);
};
