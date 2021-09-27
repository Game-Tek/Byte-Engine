#include "FontResourceManager.h"

#include "ByteEngine/Core.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.hpp>
#include <GTSL/Math/Vectors.h>

#include "TextRendering.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

FontResourceManager::FontResourceManager(): ResourceManager(u8"FontResourceManager")
{
	auto path = GetResourcePath(GTSL::StaticString<64>(u8"Fonts"), GTSL::ShortString<32>(u8"bepkg"));
	
	GTSL::File beFontFile; beFontFile.Open(path, GTSL::File::WRITE, true);

	auto GetFont = [&](const GTSL::Range<const utf8*> fontName) {
		GTSL::StaticString<255> path(BE::Application::Get()->GetPathToApplication()); path += u8"/resources/"; path += fontName; path += u8".ttf";

		GTSL::File fontFile; fontFile.Open(path, GTSL::File::READ, false);
		GTSL::Buffer fileBuffer(fontFile.GetSize(), 8, GetTransientAllocator());

		fontFile.Read(fileBuffer);

		//Font fontData(GetPersistentAllocator());
	//	const auto result = parseData(reinterpret_cast<const char*>(fileBuffer.GetData()), &fontData);

		//return fontData;
	};
	
	//auto font = GetFont(GTSL::StaticString<64>(u8"FTLTLT"));

	GTSL::Buffer<BE::TAR> data(1000000, 8, GetTransientAllocator());

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