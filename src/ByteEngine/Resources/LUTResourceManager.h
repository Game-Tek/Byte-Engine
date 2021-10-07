#pragma once
#include <GTSL/Buffer.hpp>
#include <GTSL/Filesystem.h>
#include <GTSL/RGB.h>
#include <GTSL/StaticString.hpp>
#include <GTSL/Math/Vectors.h>
#include <GTSL/LUT.hpp>

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

			GTSL::LUTData lutData;
			//GTSL::ParseLUT(buffer, lutData);
		}
	}

private:
};
