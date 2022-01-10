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
	resource_files_.Start({});

	{
		GTSL::FileQuery file_query;

		GTSL::Buffer pathBuffer(512 * 512, 16, GetTransientAllocator());

		while(auto r = file_query.DoQuery(GetResourcePath(u8"*.ttf"))) {
			GTSL::Font font({});
			GTSL::MakeFont({}, &font); //process ttf file

			FontData font_data;

			for(auto e : ALPHABET) {
				auto& glyph = font.GetGlyph(e);

				GTSL::Vector<GTSL::Vector<GTSL::Segment<3>, GTSL::DefaultAllocatorReference>, GTSL::DefaultAllocatorReference> processedGlyph;

				GTSL::MakePath(glyph, &processedGlyph); //generate N point bezier curves for glyph

				for(auto& c : processedGlyph) {
					//pathBuffer.Write(c.Points.GetLengthSize(), reinterpret_cast<const byte*>(c.Points.GetData()));
				}
			}

			resource_files_.AddEntry({}, &font_data, pathBuffer.GetRange());
		}
	}

	//glyf map
	//glyfs
	
	//for(auto& e : font.Glyphs) {
	//	Face face(GetPersistentAllocator());
	//	MakeFromPaths(e, face, 4, GetPersistentAllocator());
	//
	//	for(auto f : face.LinearBeziers) {
	//		Insert(f.Points[0], data);
	//		Insert(f.Points[1], data);
	//	}
	//
	//	for(auto f : face.CubicBeziers) {
	//		Insert(f.Points[0], data);
	//		Insert(f.Points[1], data);
	//		Insert(f.Points[2], data);
	//	}
	//
	//	for(auto f : face.Bands) {
	//		for (auto l : f.Lines) {
	//			Insert(l, data);
	//		}
	//
	//		for (auto c : f.Curves) {
	//			Insert(c, data);
	//		}
	//	}
	//}
	//
	//beFontFile.Write(data);
}