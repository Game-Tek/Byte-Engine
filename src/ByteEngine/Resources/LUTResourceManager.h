#pragma once
#include <GTSL/Buffer.hpp>
#include <GTSL/Filesystem.h>
#include <GTSL/RGB.h>
#include <GTSL/StaticString.hpp>
#include <GTSL/Math/Vectors.h>

#include "ResourceManager.h"
#include "ByteEngine/Core.h"
#include "ByteEngine/Id.h"

class LUTResourceManager : public ResourceManager {
public:
	LUTResourceManager() {
		GTSL::FileQuery fileQuery(GetResourcePath(GTSL::MakeRange("*.cube")));

		while(fileQuery) {
			GTSL::File file; file.Open(fileQuery.GetFileNameWithExtension(), GTSL::File::READ);

			GTSL::Buffer<BE::TAR> buffer; buffer.Allocate(file.GetSize(), 16, GetTransientAllocator());
			
			file.Read(buffer.GetBufferInterface());

			LUTData lutData;
			
			if(!read(buffer, lutData)) {
				BE_LOG_WARNING("Error loading LUT file.")
			}
		}
	}

private:
	struct LUTData {
		GTSL::Vector3 Min, Max;
		uint32 Size;
	};
	
	bool read(GTSL::Range<const byte*> file, LUTData& lutData) {
		uint32 pos = 0, fileSize = file.Bytes();
		//GTSL::RGB lut[1 << 15];

		auto token = GTSL::StaticString<256>();
		
		auto nextToken = [&]() {
			token.Resize(0);
			
			while (file[pos] == ' ' || file[pos] == '\n' && pos < fileSize) {
				token += file[pos];
				++pos;
			}
		};

		auto nextLine = [&]() {			
			while (file[pos++] != '\n') {}
		};

		//skip start comments
		while(file[pos] == '#') { nextLine(); }

		nextToken();
		
		switch (Id(token)()) {
		case GTSL::Hash("LUT_3D_SIZE"): {
			nextToken();
			auto number = GTSL::ToNumber<uint32>(token);

			if(!number.State()) { return false; }
			
			lutData.Size = number.Get();
			
			break;
		}
		case GTSL::Hash("DOMAIN_MIN"): {
			nextToken(); auto x = GTSL::ToNumber<float32>(token);
			if(!x.State()) { return false; }
			nextToken(); auto y = GTSL::ToNumber<float32>(token);
			if(!y.State()) { return false; }
			nextToken(); auto z = GTSL::ToNumber<float32>(token);
			if(!z.State()) { return false; }
			
			lutData.Min.X() = x.Get(); lutData.Min.Y() = y.Get(); lutData.Min.Z() = z.Get();
			
			break;
		}

		case GTSL::Hash("DOMAIN_MAX"): {
			nextToken(); auto x = GTSL::ToNumber<float32>(token);
			if(!x.State()) { return false; }
			nextToken(); auto y = GTSL::ToNumber<float32>(token);
			if(!y.State()) { return false; }
			nextToken(); auto z = GTSL::ToNumber<float32>(token);
			if(!z.State()) { return false; }

			lutData.Max.X() = x.Get(); lutData.Max.Y() = y.Get(); lutData.Max.Z() = z.Get();
			
			break;
		}
		}

		uint32 i = 0;
		
		while (!token.IsEmpty()) { //until end
			nextToken();
			
			auto r = GTSL::ToNumber<float32>(token);
			if(!r.State()) { return false; }
			auto g = GTSL::ToNumber<float32>(token);
			if(!g.State()) { return false; }
			auto b = GTSL::ToNumber<float32>(token);
			if(!b.State()) { return false; }

			//lut[i].R() = r.Get(); lut[i].G() = g.Get(); lut[i].B() = b.Get();
			
			++i;
		}

		return true;
	}
};
