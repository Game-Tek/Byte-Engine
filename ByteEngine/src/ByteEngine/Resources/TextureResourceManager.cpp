#include "TextureResourceManager.h"

#include <GTSL/Buffer.h>
#include <stb image/stb_image.h>

#include <GTSL/File.h>
#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"
#include "ByteEngine/Game/GameInstance.h"

#undef Extract

TextureResourceManager::TextureResourceManager() : ResourceManager("TextureResourceManager"), textureInfos(8, 0.25, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, package_path, resources_path, index_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	index_path += BE::Application::Get()->GetPathToApplication();
	package_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	resources_path += "/resources/";
	query_path += "/resources/*.png";
	index_path += "/resources/Textures.beidx";
	package_path += "/resources/Textures.bepkg";

	indexFile.OpenFile(index_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	packageFile.OpenFile(package_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	
	GTSL::Buffer file_buffer; file_buffer.Allocate(2048 * 2048 * 2, 32, GetTransientAllocator());

	if (indexFile.ReadFile(file_buffer))
	{
		GTSL::Extract(textureInfos, file_buffer);
		file_buffer.Free(32, GetTransientAllocator());
		return;
	}
	
	auto load = [&](const GTSL::FileQuery::QueryResult& queryResult)
	{
		auto file_path = resources_path;
		file_path += queryResult.FileNameWithExtension;
		auto name = queryResult.FileNameWithExtension; name.Drop(name.FindLast('.'));
		const auto hashed_name = GTSL::Id64(name);

		if (!textureInfos.Find(hashed_name))
		{
			GTSL::File query_file;
			query_file.OpenFile(file_path, static_cast<uint8>(GTSL::File::AccessMode::READ), GTSL::File::OpenMode::LEAVE_CONTENTS);

			query_file.ReadFile(file_buffer);

			int32 x, y, channel_count = 0;
			auto* const data = stbi_load_from_memory(file_buffer.GetData(), file_buffer.GetLength(), &x, &y, &channel_count, 0);

			TextureInfo texture_info;

			switch (channel_count)
			{
			case 1: texture_info.Format = static_cast<uint8>(GAL::TextureFormat::R_I8); break;
			case 2: texture_info.Format = static_cast<uint8>(GAL::TextureFormat::RG_I8); break;
			case 3: texture_info.Format = static_cast<uint8>(GAL::TextureFormat::RGB_I8); break;
			case 4: texture_info.Format = static_cast<uint8>(GAL::TextureFormat::RGBA_I8); break;
			default: BE_ASSERT(false, "Non valid texture format count!");
			}

			texture_info.ByteOffset = static_cast<uint32>(packageFile.GetFileSize());

			const uint32 size = static_cast<uint32>(x) * y * channel_count;

			texture_info.ImageSize = size;
			texture_info.Dimensions = GAL::Dimension::SQUARE;
			texture_info.Extent = { static_cast<uint16>(x), static_cast<uint16>(y), 1 };

			packageFile.WriteToFile(GTSL::Ranger<byte>(size, data));

			textureInfos.Emplace(hashed_name, texture_info);

			stbi_image_free(data);

			query_file.CloseFile();
		}
	};
	
	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, load);

	indexFile.CloseFile();
	indexFile.OpenFile(index_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::CLEAR);

	file_buffer.Resize(0);
	Insert(textureInfos, file_buffer);
	indexFile.WriteToFile(file_buffer);
	
	file_buffer.Free(32, GetTransientAllocator());
}

TextureResourceManager::~TextureResourceManager()
{
	packageFile.CloseFile(); indexFile.CloseFile();
}

void TextureResourceManager::LoadTexture(const TextureLoadInfo& textureLoadInfo)
{
	auto& texture_info = textureInfos.At(textureLoadInfo.Name);

	indexFile.SetPointer(texture_info.ByteOffset, GTSL::File::MoveFrom::BEGIN);
	packageFile.ReadFromFile(GTSL::Ranger<byte>(texture_info.ImageSize, textureLoadInfo.DataBuffer.begin()));

	OnTextureLoadInfo onTextureLoadInfo;
	onTextureLoadInfo.ResourceName = textureLoadInfo.Name;
	onTextureLoadInfo.UserData = textureLoadInfo.UserData;
	onTextureLoadInfo.DataBuffer = textureLoadInfo.DataBuffer;
	
	onTextureLoadInfo.Extent = texture_info.Extent;
	onTextureLoadInfo.Dimensions = texture_info.Dimensions;
	onTextureLoadInfo.LODPercentage = 1.0f;
	onTextureLoadInfo.TextureFormat = static_cast<GAL::TextureFormat>(texture_info.Format);
	
	textureLoadInfo.GameInstance->AddAsyncTask(textureLoadInfo.OnTextureLoadInfo, GTSL::MoveRef(onTextureLoadInfo));
}

void Insert(const TextureResourceManager::TextureInfo& textureInfo, GTSL::Buffer& buffer)
{
	Insert(textureInfo.ByteOffset, buffer);
	Insert(textureInfo.ImageSize, buffer);
	Insert(textureInfo.Format, buffer);
	Insert(textureInfo.Dimensions, buffer);
	Insert(textureInfo.Extent, buffer);
}

void Extract(TextureResourceManager::TextureInfo& textureInfo, GTSL::Buffer& buffer)
{
	Extract(textureInfo.ByteOffset, buffer);
	Extract(textureInfo.ImageSize, buffer);
	Extract(textureInfo.Format, buffer);
	Extract(textureInfo.Dimensions, buffer);
	Extract(textureInfo.Extent, buffer);
}
