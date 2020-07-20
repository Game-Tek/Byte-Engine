#include "MaterialResourceManager.h"

#include <GTSL/Buffer.h>
#include <GTSL/DataSizes.h>
#include <GTSL/Serialize.h>
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Render/RenderTypes.h"

MaterialResourceManager::MaterialResourceManager() : ResourceManager("MaterialResourceManager"), materialInfos(16, GetPersistentAllocator())
{
	GTSL::Buffer file_buffer; file_buffer.Allocate((uint32)GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
	
	GTSL::StaticString<256> resources_path;
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";

	resources_path += "Materials.bepkg";
	package.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ | (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::LEAVE_CONTENTS);

	resources_path.Drop(resources_path.FindLast('/') + 1);
	resources_path += "Materials.beidx";

	index.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ | (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::LEAVE_CONTENTS);
	index.ReadFile(file_buffer);

	if (file_buffer.GetLength())
	{
		Extract(materialInfos, file_buffer);
	}
	
	file_buffer.Free(8, GetTransientAllocator());
}

MaterialResourceManager::~MaterialResourceManager()
{
	package.CloseFile(); index.CloseFile();
}

void MaterialResourceManager::CreateMaterial(const MaterialCreateInfo& materialCreateInfo)
{
	const auto hashed_name = GTSL::Id64(materialCreateInfo.ShaderName);
	
	if (!materialInfos.Find(hashed_name))
	{
		GTSL::Buffer shader_source_buffer; shader_source_buffer.Allocate((uint32)GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer index_buffer; index_buffer.Allocate((uint32)GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer shader_buffer; shader_buffer.Allocate((uint32)GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		
		MaterialInfo material_info;
		
		GTSL::StaticString<256> resources_path;
		resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";

		resources_path += materialCreateInfo.ShaderName; resources_path += ".vs";

		GTSL::File shader; shader.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
		shader.ReadFile(shader_source_buffer);

		auto f = GTSL::Ranger<const UTF8>(shader_source_buffer.GetLength(), reinterpret_cast<const UTF8*>(shader_source_buffer.GetData()));
		Shader::CompileShader(f, materialCreateInfo.ShaderName, GAL::ShaderType::VERTEX_SHADER, GAL::ShaderLanguage::GLSL, shader_buffer);

		material_info.VertexShaderOffset = package.GetFileSize();
		material_info.VertexShaderSize = shader_buffer.GetLength();
		package.WriteToFile(shader_buffer);
		
		resources_path.Drop(resources_path.FindLast('/') + 1);
		resources_path += materialCreateInfo.ShaderName; resources_path += ".fs";

		shader.CloseFile();
		shader.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
		
		shader_source_buffer.Resize(0);
		shader_buffer.Resize(0);
		shader.ReadFile(shader_source_buffer);

		f = GTSL::Ranger<const UTF8>(shader_source_buffer.GetLength(), reinterpret_cast<const UTF8*>(shader_source_buffer.GetData()));
		Shader::CompileShader(f, materialCreateInfo.ShaderName, GAL::ShaderType::FRAGMENT_SHADER, GAL::ShaderLanguage::GLSL, shader_buffer);

		material_info.FragmentShaderSize = shader_buffer.GetLength();
		package.WriteToFile(shader_buffer);

		materialInfos.Emplace(hashed_name, material_info);
		index.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
		Insert(materialInfos, index_buffer);
		index.WriteToFile(index_buffer);

		shader.CloseFile();
		shader_source_buffer.Free(8, GetTransientAllocator());
		index_buffer.Free(8, GetTransientAllocator());
		shader_buffer.Free(8, GetTransientAllocator());
	}
}

void Insert(const MaterialResourceManager::MaterialInfo& materialInfo, GTSL::Buffer& buffer)
{
	Insert(materialInfo.VertexShaderOffset, buffer);
	Insert(materialInfo.VertexShaderSize, buffer);
	Insert(materialInfo.FragmentShaderSize, buffer);
}

void Extract(MaterialResourceManager::MaterialInfo& materialInfo, GTSL::Buffer& buffer)
{
	Extract(materialInfo.VertexShaderOffset, buffer);
	Extract(materialInfo.VertexShaderSize, buffer);
	Extract(materialInfo.FragmentShaderSize, buffer);
}