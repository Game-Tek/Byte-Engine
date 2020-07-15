#include "TextureResourceManager.h"

#include <GTSL/Buffer.h>
#include <stb image/stb_image.h>

#include <GTSL/File.h>
#include <GTSL/Filesystem.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

TextureResourceManager::TextureResourceManager() : SubResourceManager("Texture"), textureInfos(4, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, package_path, resources_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	package_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	query_path += "/resources/";
	package_path += "/resources/";
	resources_path += "/resources/";
	query_path += "*.png";
	package_path += "Textures.bepkg";

	packageFile.OpenFile(package_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::CLEAR);
	
	GTSL::Buffer file_buffer; file_buffer.Allocate(2048 * 2048 * 2, 32, GetTransientAllocator());
	
	auto load = [&](const GTSL::FileQuery::QueryResult& queryResult)
	{
		auto file_path = resources_path;
		file_path += queryResult.FileNameWithExtension;
		auto name = queryResult.FileNameWithExtension; name.Drop(name.FindLast('.'));
		const auto hashed_name = GTSL::Id64(name.operator GTSL::Ranger<const char>());

		GTSL::File query_file;
		query_file.OpenFile(file_path, static_cast<uint8>(GTSL::File::AccessMode::READ), GTSL::File::OpenMode::LEAVE_CONTENTS);

		query_file.ReadFile(file_buffer);

		int32 x, y, channel_count = 0;
		auto* const data = stbi_load_from_memory(file_buffer.GetData(), file_buffer.GetLength(), &x, &y, &channel_count, 0);

		TextureInfo texture_info;

		switch (channel_count)
		{
		case 1: texture_info.Format = static_cast<uint8>(GAL::ImageFormat::R_I8); break;
		case 2: texture_info.Format = static_cast<uint8>(GAL::ImageFormat::RG_I8); break;
		case 3: texture_info.Format = static_cast<uint8>(GAL::ImageFormat::RGB_I8); break;
		case 4: texture_info.Format = static_cast<uint8>(GAL::ImageFormat::RGBA_I8); break;
		default: BE_ASSERT(false, "Non valid texture format count!");
		}

		texture_info.ByteOffset = static_cast<uint32>(packageFile.GetFileSize());

		const uint64 size = x * y * channel_count;
		
		packageFile.WriteToFile(GTSL::Ranger<byte>(size, data));

		textureInfos.Emplace(GetPersistentAllocator(), hashed_name, texture_info);

		stbi_image_free(data);
		
		query_file.CloseFile();
	};
	
	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, load);

	file_buffer.Free(32, GetTransientAllocator());
}

TextureResourceManager::~TextureResourceManager()
{
	textureInfos.Free(GetPersistentAllocator());
	packageFile.CloseFile();
}

void TextureResourceManager::LoadTexture(const TextureLoadInfo& textureLoadInfo)
{
	auto& audio_resource_info = textureInfos.At(textureLoadInfo.Name);

	if (!textureAssets.Find(textureLoadInfo.Name))
	{
		indexFile.SetPointer(audio_resource_info.ByteOffset, GTSL::File::MoveFrom::BEGIN);
		//packageFile.ReadFromFile()
	}

	//handle resource is loaded
}
