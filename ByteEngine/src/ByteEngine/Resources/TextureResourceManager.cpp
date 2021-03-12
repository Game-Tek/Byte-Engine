#include "TextureResourceManager.h"

#include <GTSL/Buffer.hpp>
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
	GTSL::StaticString<512> query_path, resources_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	resources_path += "/resources/";
	query_path += "/resources/*.png";
	auto index_path = GetResourcePath(GTSL::ShortString<32>("Textures.beidx"));
	auto package_path = GetResourcePath(GTSL::ShortString<32>("Textures.bepkg"));

	indexFile.OpenFile(index_path, GTSL::File::AccessMode::WRITE | GTSL::File::AccessMode::READ);
	
	GTSL::Buffer<BE::TAR> indexFileBuffer; indexFileBuffer.Allocate(2048 * 2048 * 3, 32, GetTransientAllocator());

	if (indexFile.ReadFile(indexFileBuffer.GetBufferInterface()))
	{
		GTSL::Extract(textureInfos, indexFileBuffer);
	}
	else
	{
		GTSL::File packageFile; packageFile.OpenFile(package_path, GTSL::File::AccessMode::WRITE | GTSL::File::AccessMode::READ);

		auto load = [&](const GTSL::FileQuery::QueryResult& queryResult)
		{
			auto file_path = resources_path;
			file_path += queryResult.FileNameWithExtension;
			auto name = queryResult.FileNameWithExtension; name.Drop(name.FindLast('.').Get());
			const auto hashed_name = GTSL::Id64(name);

			if (!textureInfos.Find(hashed_name))
			{
				GTSL::File query_file;
				query_file.OpenFile(file_path, GTSL::File::AccessMode::READ); GTSL::Buffer<BE::TAR> textureBuffer; textureBuffer.Allocate(query_file.GetFileSize(), 8, GetTransientAllocator());

				query_file.ReadFile(textureBuffer.GetBufferInterface());

				int32 x, y, channel_count = 0;
				stbi_info_from_memory(textureBuffer.GetData(), textureBuffer.GetLength(), &x, &y, &channel_count);
				auto finalChannelCount = GTSL::NextPowerOfTwo(static_cast<uint32>(channel_count));
				auto* const data = stbi_load_from_memory(textureBuffer.GetData(), textureBuffer.GetLength(), &x, &y, &channel_count, finalChannelCount);

				TextureInfo texture_info;

				texture_info.Format = GAL::FormatDescriptor(GAL::ComponentType::INT, finalChannelCount, 8, GAL::TextureType::COLOR, 0, 1, 2, 3);

				texture_info.ByteOffset = static_cast<uint32>(packageFile.GetFileSize());

				const uint32 size = static_cast<uint32>(x) * y * finalChannelCount;

				texture_info.Dimensions = GAL::Dimension::SQUARE;
				texture_info.Extent = { static_cast<uint16>(x), static_cast<uint16>(y), 1 };

				packageFile.WriteToFile(GTSL::Range<byte*>(size, data));

				textureInfos.Emplace(hashed_name, texture_info);

				stbi_image_free(data);
			}
		};

		GTSL::FileQuery file_query(query_path);
		GTSL::ForEach(file_query, load);

		indexFileBuffer.Resize(0);
		Insert(textureInfos, indexFileBuffer);
		indexFile.WriteToFile(indexFileBuffer);
	}
		
	initializePackageFiles(package_path);
}

TextureResourceManager::~TextureResourceManager()
{
}