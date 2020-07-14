#include "TextureResourceManager.h"

#include <stb image/stb_image.h>
#include <GTSL/Id.h>

#include <GTSL/File.h>

#include "ByteEngine/Application/Application.h"

void TextureResourceManager::LoadTexture(const TextureLoadInfo& textureLoadInfo)
{
	GTSL::StaticString<1024> path;
	//path += BE::Application::Get()->GetResourceManager()->GetResourcePath();
	path += '/';
	path += textureLoadInfo.Name;
	path += ".png";
	
	GTSL::File file;
	file.OpenFile(path, (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::LEAVE_CONTENTS);
	auto file_size = file.GetFileSize();
	auto range = GTSL::Ranger<byte>(file_size, textureLoadInfo.TextureDataBuffer.begin());
	file.ReadFromFile(range);

	int32 x, y, channel_count;
	stbi_load_from_memory(range.begin(), range.Bytes(), &x, &y, &channel_count, 0);

	OnTextureLoadInfo on_texture_load_info;
	on_texture_load_info.TextureDataBuffer = range;

	GAL::ImageFormat format;
	
	switch(channel_count)
	{
	case 1: format = GAL::ImageFormat::R_I8; break;
	case 2: format = GAL::ImageFormat::RG_I8; break;
	case 3: format = GAL::ImageFormat::RGB_I8; break;
	case 4: format = GAL::ImageFormat::RGBA_I8; break;
	}
	
	on_texture_load_info.TextureFormat = format;

	file.CloseFile();
	
	textureLoadInfo.OnTextureLoadInfo(on_texture_load_info);
}
