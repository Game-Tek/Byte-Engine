#include "ShaderResourceManager.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/Serialize.h>
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Render/RenderTypes.h"
#include "ByteEngine/Render/ShaderGenerator.h"

ShaderResourceManager::ShaderResourceManager() : ResourceManager(u8"ShaderResourceManager"), shaderGroups(8, GetPersistentAllocator()), shaderInfos(8, GetPersistentAllocator())
{
	GTSL::Buffer<BE::TAR> shaderGroupBuffer;
	shaderGroupBuffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());

	initializePackageFiles(shaderPackageFiles, GetResourcePath(GTSL::ShortString<32>(u8"ShaderGroups"), GTSL::ShortString<32>(u8"bepkg")));	

	switch (shadersIndex.Open(GetResourcePath(GTSL::ShortString<32>(u8"Shaders"), GTSL::ShortString<32>(u8"beidx")), GTSL::File::READ | GTSL::File::WRITE, true)) {
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	}

	switch (shaderGroupsIndex.Open(GetResourcePath(GTSL::ShortString<32>(u8"ShaderGroups"), GTSL::ShortString<32>(u8"beidx")), GTSL::File::READ | GTSL::File::WRITE, true)) {
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	}
	
	shaderGroupsIndex.Read(shaderGroupBuffer.GetBufferInterface());

	if (shaderGroupBuffer.GetLength()) {
		Extract(shaderGroups, shaderGroupBuffer);
	}
	
	shaderGroupBuffer.Resize(0);
	
	shadersIndex.Read(shaderGroupBuffer.GetBufferInterface());
	
	if (shaderGroupBuffer.GetLength()) {
		Extract(shaderInfos, shaderGroupBuffer);
	}
}

ShaderResourceManager::~ShaderResourceManager()
{
}