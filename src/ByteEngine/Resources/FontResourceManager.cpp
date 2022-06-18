#include "FontResourceManager.h"

#include "ByteEngine/Core.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/Filesystem.h>
#include <GTSL/TTF.hpp>
#include <GTSL/Math/Vectors.hpp>

#include "TextRendering.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

FontResourceManager::FontResourceManager(const InitializeInfo& info) : ResourceManager(info, u8"FontResourceManager") {
	resource_files_.Start(GetResourcePath(u8"Fonts"));

	{
		GTSL::FileQuery file_query;

		while(auto r = file_query.DoQuery(GetUserResourcePath(u8"*.ttf"))) {
			auto name = r.Get();

			RTrimLast(name, U'.');

			if(resource_files_.Exists(Id(name))) { continue; }

			GTSL::Buffer fontFileContentsBuffer(GetTransientAllocator());
			GTSL::File file; file.Open(GetUserResourcePath(r.Get())); file.Read(fontFileContentsBuffer);

			GTSL::Font font{ GTSL::DefaultAllocatorReference() };
			GTSL::MakeFont(fontFileContentsBuffer.GetRange(), &font); //process ttf file

			GTSL::Buffer pathBuffer(512 * 512, 16, GetTransientAllocator());
			FontData font_data;

			pathBuffer << static_cast<uint32>(SIZE); // Number of glyphs

			for(uint32 i = 0; i < SIZE; ++i) {
				auto& glyph = font.GetGlyph(ALPHABET[i]);

				GTSL::Vector<GTSL::Vector<GTSL::Segment<3>, GTSL::DefaultAllocatorReference>, GTSL::DefaultAllocatorReference> processedGlyph;

				GTSL::MakePath(glyph, &processedGlyph); //generate N point bezier curves for glyph

				pathBuffer << uint32(processedGlyph.GetLength()); // Contours

				auto& m = font_data.Characters.array[i];
				m.Advance = glyph.AdvanceWidth;

				for (auto& c : processedGlyph) {
					pathBuffer << uint32(c.GetLength()); // Points

					for (auto& d : c) {
						for(auto& point : d.Points) { // Normalize point coordinates
							point /= glyph.Max;
						}

						if(d.IsBezierCurve()) {
							pathBuffer << uint8(3);
							pathBuffer.Write(24, reinterpret_cast<const byte*>(d.Points));
						} else {
							pathBuffer << uint8(2);
							pathBuffer.Write(8, reinterpret_cast<const byte*>(&d.Points[0]));
							pathBuffer.Write(8, reinterpret_cast<const byte*>(&d.Points[2]));							
						}
					}
				}
			}

			resource_files_.AddEntry(name, &font_data, pathBuffer.GetRange());
		}
	}
}