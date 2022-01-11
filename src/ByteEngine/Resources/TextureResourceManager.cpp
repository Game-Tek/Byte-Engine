#include "TextureResourceManager.h"

#include <GTSL/Buffer.hpp>

#include <GTSL/File.h>
#include <GTSL/Filesystem.h>

#include "ByteEngine/Application/Application.h"

#define STB_IMAGE_IMPLEMENTATION
//#define STBI_NO_STDIO
//#define STBI_NO_GIF
#include "stb_image.h"

#include <GTSL/JSON.hpp>

#undef Extract

TextureResourceManager::TextureResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"TextureResourceManager")
{
	GTSL::StaticString<512> query_path, resources_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	resources_path += u8"/resources/";
	query_path += u8"/resources/*.png";

	GTSL::String indexFileString(5192, GetTransientAllocator());
	auto serializer = GTSL::MakeSerializer(indexFileString);
	GTSL::StartArray(serializer, indexFileString, u8"textures");

	resource_files_.Start(resources_path + u8"Textures");

	GTSL::FileQuery file_query;

	while (auto queryResult = file_query.DoQuery(query_path)) {
		auto file_path = resources_path;
		file_path += queryResult.Get();
		auto fileName = queryResult.Get(); DropLast(fileName, u8'.');
		const auto hashed_name = GTSL::Id64(fileName);

		if (!resource_files_.Exists(hashed_name)) {
			GTSL::File query_file;
			query_file.Open(file_path, GTSL::File::READ, false);
			
			GTSL::Buffer textureFileBuffer(GetTransientAllocator());
			query_file.Read(textureFileBuffer);

			int32 x = 0, y = 0, channel_count = 0;
			stbi_info_from_memory(textureFileBuffer.GetData(), textureFileBuffer.GetLength(), &x, &y, &channel_count);
			auto finalChannelCount = GTSL::NextPowerOfTwo(static_cast<uint32>(channel_count));
			byte* data = nullptr;

			{
				GTSL::StaticVector<GTSL::StaticString<128>, 8> substrings;

				GTSL::Substrings(queryResult.Get(), substrings, U'.');

				for(auto& e : substrings) {
					switch (GTSL::Hash(e)) {
					break; case GTSL::Hash(u8"png"): {
						data = stbi_load_from_memory(textureFileBuffer.GetData(), textureFileBuffer.GetLength(), &x, &y, &channel_count, finalChannelCount);						
					}
					break; case GTSL::Hash(u8"hdr"): {
						data = reinterpret_cast<byte*>(stbi_loadf_from_memory(textureFileBuffer.GetData(), textureFileBuffer.GetLength(), &x, &y, &channel_count, finalChannelCount));
					}
					}
				}
			}

			TextureInfo texture_info;

			texture_info.Format = GAL::FormatDescriptor(GAL::ComponentType::INT, finalChannelCount, 8, GAL::TextureType::COLOR, 0, 1, 2, 3);
			const uint32 size = static_cast<uint32>(x) * y * finalChannelCount;
			texture_info.Extent = { static_cast<uint16>(x), static_cast<uint16>(y), 1 };

			GTSL::StartObject(serializer, indexFileString);
				GTSL::Insert(serializer, indexFileString, u8"name", fileName);
				GTSL::Insert(serializer, indexFileString, u8"format", GTSL::StringView(u8"INT_4_8_C_0123"));
				GTSL::StartArray(serializer, indexFileString, u8"extent");
					GTSL::Insert(serializer, indexFileString, x);
					GTSL::Insert(serializer, indexFileString, y);
					GTSL::Insert(serializer, indexFileString, 1);
				GTSL::EndArray(serializer, indexFileString);
			GTSL::EndObject(serializer, indexFileString);

			resource_files_.AddEntry(fileName, &texture_info, { size, static_cast<const byte*>(data) });

			stbi_image_free(data);
		}
	}
}

TextureResourceManager::~TextureResourceManager()
{
}