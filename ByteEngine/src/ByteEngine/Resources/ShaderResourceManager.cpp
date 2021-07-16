#include "ShaderResourceManager.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/Serialize.h>
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Render/RenderTypes.h"

ShaderResourceManager::ShaderResourceManager() : ResourceManager(u8"ShaderResourceManager"), shaderGroupsMap(8, GetPersistentAllocator()), shaderInfosMap(8, GetPersistentAllocator())
{
	initializePackageFiles(shaderPackageFiles, GetResourcePath(GTSL::ShortString<32>(u8"Shaders"), GTSL::ShortString<32>(u8"bepkg")));

	switch (shaderInfosFile.Open(GetResourcePath(GTSL::ShortString<32>(u8"Shaders"), GTSL::ShortString<32>(u8"beidx")), GTSL::File::READ | GTSL::File::WRITE, true)) {
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	}

	switch (shaderGroupsInfoFile.Open(GetResourcePath(GTSL::ShortString<32>(u8"ShaderGroups"), GTSL::ShortString<32>(u8"beidx")), GTSL::File::READ | GTSL::File::WRITE, true)) {
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	}

	{
		GTSL::Buffer fileBuffer(GetTransientAllocator());
		
		shaderGroupsInfoFile.Read(fileBuffer);

		if (fileBuffer.GetLength()) {
			Extract(shaderGroupsMap, fileBuffer);
		}
	}
	
	{
		GTSL::Buffer fileBuffer(GetTransientAllocator());
		
		shaderInfosFile.Read(fileBuffer);
		
		if (fileBuffer.GetLength()) {
			Extract(shaderInfosMap, fileBuffer);
		}
	}
}

ShaderResourceManager::~ShaderResourceManager()
{
}