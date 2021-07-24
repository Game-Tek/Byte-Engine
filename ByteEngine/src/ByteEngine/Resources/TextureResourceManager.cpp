#include "TextureResourceManager.h"

#include <GTSL/Buffer.hpp>
#include <stb image/stb_image.h>

#include <GTSL/File.h>
#include <GTSL/Filesystem.h>

#include "ByteEngine/Application/Application.h"

#undef Extract

TextureResourceManager::TextureResourceManager() : ResourceManager(u8"TextureResourceManager"), textureInfos(8, 0.25, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, resources_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	resources_path += u8"/resources/";
	query_path += u8"/resources/*.png";
	auto index_path = GetResourcePath(GTSL::ShortString<32>(u8"Textures"), GTSL::ShortString<32>(u8"beidx"));
	auto package_path = GetResourcePath(GTSL::ShortString<32>(u8"Textures"), GTSL::ShortString<32>(u8"bepkg"));

	switch (indexFile.Open(index_path, GTSL::File::WRITE | GTSL::File::READ, true)) {
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: {
		GTSL::File packageFile; packageFile.Open(package_path, GTSL::File::WRITE, false);

		GTSL::FileQuery file_query(query_path);

		while (file_query.DoQuery()) {
			auto file_path = resources_path;
			file_path += file_query.GetFileNameWithExtension();
			auto name = file_query.GetFileNameWithExtension(); name.Drop(FindLast(name,u8'.').Get());
			const auto hashed_name = GTSL::Id64(name);

			if (!textureInfos.Find(hashed_name))
			{
				GTSL::File query_file;
				query_file.Open(file_path, GTSL::File::READ, false);
				
				GTSL::Buffer textureBuffer(GetTransientAllocator());
				query_file.Read(textureBuffer);

				int32 x, y, channel_count = 0;
				stbi_info_from_memory(textureBuffer.GetData(), textureBuffer.GetLength(), &x, &y, &channel_count);
				auto finalChannelCount = GTSL::NextPowerOfTwo(static_cast<uint32>(channel_count));
				auto* const data = stbi_load_from_memory(textureBuffer.GetData(), textureBuffer.GetLength(), &x, &y, &channel_count, finalChannelCount);

				TextureInfo texture_info;

				texture_info.Format = GAL::FormatDescriptor(GAL::ComponentType::INT, finalChannelCount, 8, GAL::TextureType::COLOR, 0, 1, 2, 3);

				texture_info.ByteOffset = static_cast<uint32>(packageFile.GetSize());

				const uint32 size = static_cast<uint32>(x) * y * finalChannelCount;

				texture_info.Extent = { static_cast<uint16>(x), static_cast<uint16>(y), 1 };

				packageFile.Write(GTSL::Range<byte*>(size, data));

				textureInfos.Emplace(hashed_name, texture_info);

				stbi_image_free(data);
			}
		}

		GTSL::Buffer indexFileBuffer(2048 , 32, GetTransientAllocator());
		Insert(textureInfos, indexFileBuffer);
		indexFile.Write(indexFileBuffer);

		textureInfos.Clear();
		indexFile.SetPointer(0);
		
		break;
	}
	case GTSL::File::OpenResult::ERROR: break;
	default: ;
	}

	GTSL::Buffer<BE::TAR> indexFileBuffer(2048, 32, GetTransientAllocator());
	indexFile.Read(indexFileBuffer);
	Extract(textureInfos, indexFileBuffer);

	mappedFile.Open(GetResourcePath(GTSL::MakeRange(u8"Textures.bepkg")));
}

TextureResourceManager::~TextureResourceManager()
{
}