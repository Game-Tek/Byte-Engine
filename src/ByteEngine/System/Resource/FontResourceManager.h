#pragma once

#include <GTSL/Buffer.hpp>
#include <GTSL/Extent.h>
#include <GTSL/HashMap.hpp>

#include "ResourceManager.h"
#include "ByteEngine/Core.h"
#include "ByteEngine/Game/ApplicationManager.h"

struct IVector2D
{
	IVector2D() = default;

	IVector2D(const GTSL::int32 x, const GTSL::int32 y) : X(x), Y(y) {}

	GTSL::int32 X = 0, Y = 0;
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
		IVector2D Bearing;    // Offset from baseline to left/top of glyph
		GTSL::Extent2D Position;
		GTSL::uint32 Advance;    // Offset to advance to next glyph
	};

	static constexpr char8_t ALPHABET[] = u8"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
	static constexpr GTSL::uint64 SIZE = sizeof(ALPHABET) - 1ull;

	struct FontData : SData {
		DEFINE_ARRAY_MEMBER(Character, Characters, SIZE)
	};

	template<typename... ARGS>
	void LoadFont(const GTSL::StringView font_name, const TaskHandle<FontData, GTSL::Buffer<BE::PAR>, ARGS...> task_handle) {
		FontData fontData;
		resource_files_.LoadEntry(font_name, fontData); // TODO: handle non existant resources
		GTSL::Buffer buffer(GetPersistentAllocator());
		resource_files_.LoadData(fontData, buffer);
		GetApplicationManager()->EnqueueTask(task_handle, GTSL::MoveRef(fontData), MoveRef(buffer));
	}

private:
	ResourceFiles resource_files_;
	//int8 parseData(const char* data, Font* fontData);
};
