#include "MaterialResourceManager.h"

#include <GTSL/Buffer.h>
#include <GTSL/DataSizes.h>
#include <GTSL/Serialize.h>
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Render/RenderTypes.h"

static_assert((uint8)GAL::ShaderType::VERTEX_SHADER == 0, "Enum changed!");
static_assert((uint8)GAL::ShaderType::COMPUTE_SHADER == 5, "Enum changed!");

static constexpr const char* TYPE_TO_EXTENSION[12] = { ".vs", ".tcs", ".tes", ".gs", ".fs", ".cs", ".rgs", ".ahs", ".chs", ".ms", ".is", ".cs" };

using VertexElementsType = GTSL::UnderlyingType<GAL::ShaderDataType>;
using ShaderTypeType = GTSL::UnderlyingType<GAL::ShaderType>;
using BindingTypeType = GTSL::UnderlyingType<GAL::BindingType>;

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
		GTSL::Buffer shader_source_buffer; shader_source_buffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer index_buffer; index_buffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer shader_buffer; shader_buffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer shader_error_buffer; shader_error_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(512)), 8, GetTransientAllocator());
		
		MaterialInfo materialInfo;
		
		GTSL::StaticString<256> resources_path;
		resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";

		GTSL::File shader;

		materialInfo.MaterialOffset = package.GetFileSize();
		
		for (uint8 i = 0; i < materialCreateInfo.ShaderTypes.ElementCount(); ++i)
		{
			resources_path += materialCreateInfo.ShaderName; resources_path += TYPE_TO_EXTENSION[static_cast<uint8>(materialCreateInfo.ShaderTypes[i])];

			shader.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
			shader.ReadFile(shader_source_buffer);

			auto f = GTSL::Ranger<const UTF8>(shader_source_buffer.GetLength(), reinterpret_cast<const UTF8*>(shader_source_buffer.GetData()));
			const auto comp_res = Shader::CompileShader(f, materialCreateInfo.ShaderName, static_cast<GAL::ShaderType>(materialCreateInfo.ShaderTypes[i]), GAL::ShaderLanguage::GLSL, shader_buffer, shader_error_buffer);
			*(shader_error_buffer.GetData() + (shader_error_buffer.GetLength() - 1)) = '\0';
			if(comp_res == false)
			{
				BE_LOG_ERROR(reinterpret_cast<const char*>(shader_error_buffer.GetData()));
			}
			BE_ASSERT(comp_res != false, shader_error_buffer.GetData());

			materialInfo.ShaderSizes.EmplaceBack(shader_buffer.GetLength());
			package.WriteToFile(shader_buffer);
			
			resources_path.Drop(resources_path.FindLast('/') + 1);

			shader_source_buffer.Resize(0);
			shader_error_buffer.Resize(0);
			shader_buffer.Resize(0);
			shader.CloseFile();
		}

		materialInfo.VertexElements = GTSL::Ranger<const VertexElementsType>(materialCreateInfo.VertexFormat.ElementCount(), reinterpret_cast<const VertexElementsType*>(materialCreateInfo.VertexFormat.begin()));
		materialInfo.ShaderTypes = GTSL::Ranger<const ShaderTypeType>(materialCreateInfo.ShaderTypes.ElementCount(), reinterpret_cast<const ShaderTypeType*>(materialCreateInfo.ShaderTypes.begin()));
		materialInfo.RenderGroup = GTSL::Id64(materialCreateInfo.RenderGroup);

		for(uint32 i = 0; i < materialCreateInfo.Bindings.ElementCount(); ++i)
		{
			for(uint32 j = 0; j < materialCreateInfo.Bindings[i].ElementCount(); ++j)
			{
				materialInfo.BindingSets[i].EmplaceBack(materialCreateInfo.Bindings[i][j]);
			}
		}

		for(uint32 i = 0; i < materialCreateInfo.Uniforms.ElementCount(); ++i)
		{
			for(uint32 j = 0; j < materialCreateInfo.Uniforms[i].ElementCount(); ++j)
			{
				materialInfo.Uniforms[i].EmplaceBack(materialCreateInfo.Uniforms[i][j]);
			}
		}
		
		materialInfos.Emplace(hashed_name, materialInfo);
		index.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
		Insert(materialInfos, index_buffer);
		index.WriteToFile(index_buffer);

		shader.CloseFile();
		shader_source_buffer.Free(8, GetTransientAllocator());
		index_buffer.Free(8, GetTransientAllocator());
		shader_buffer.Free(8, GetTransientAllocator());
		shader_error_buffer.Free(8, GetTransientAllocator());
	}
}

void MaterialResourceManager::GetMaterialSize(const GTSL::Id64 name, uint32& size)
{
	GTSL::ReadLock lock(mutex);
	for(auto& e : materialInfos.At(name).ShaderSizes) { size += e; }
}

void MaterialResourceManager::LoadMaterial(const MaterialLoadInfo& loadInfo)
{
	auto materialInfo = materialInfos.At(loadInfo.Name);

	uint32 mat_size = 0;
	for (auto e : materialInfo.ShaderSizes) { mat_size += e; }
	BE_ASSERT(mat_size <= loadInfo.DataBuffer.Bytes(), "Buffer can't hold required data!");

	BE_ASSERT(materialInfo.MaterialOffset != materialInfo.ShaderSizes[0], ":|");
	
	package.SetPointer(materialInfo.MaterialOffset, GTSL::File::MoveFrom::BEGIN);

	[[maybe_unused]] const auto read = package.ReadFromFile(loadInfo.DataBuffer);
	BE_ASSERT(read != 0, "Read 0 bytes!");
	
	OnMaterialLoadInfo onMaterialLoadInfo;
	onMaterialLoadInfo.ResourceName = loadInfo.Name;
	onMaterialLoadInfo.UserData = loadInfo.UserData;
	onMaterialLoadInfo.DataBuffer = loadInfo.DataBuffer;
	onMaterialLoadInfo.ShaderTypes = GTSL::Ranger<GAL::ShaderType>(materialInfo.ShaderTypes.GetLength(), reinterpret_cast<GAL::ShaderType*>(materialInfo.ShaderTypes.begin()));
	onMaterialLoadInfo.ShaderSizes = materialInfo.ShaderSizes;
	onMaterialLoadInfo.RenderGroup = materialInfo.RenderGroup;
	
	for (uint32 i = 0; i < materialInfo.BindingSets.GetLength(); ++i)
	{
		for (uint32 j = 0; j < materialInfo.BindingSets[i].GetLength(); ++j)
		{
			onMaterialLoadInfo.BindingSets[i].EmplaceBack(materialInfo.BindingSets[i][j]);
		}
	}

	for (uint32 i = 0; i < materialInfo.Uniforms.GetLength(); ++i)
	{
		for (uint32 j = 0; j < materialInfo.Uniforms[i].GetLength(); ++j)
		{
			onMaterialLoadInfo.Uniforms[i].EmplaceBack(materialInfo.Uniforms[i][j]);
		}
	}
	
	onMaterialLoadInfo.VertexElements = GTSL::Ranger<GAL::ShaderDataType>(materialInfo.VertexElements.GetLength(), reinterpret_cast<GAL::ShaderDataType*>(materialInfo.VertexElements.begin()));
	
	loadInfo.GameInstance->AddDynamicTask(loadInfo.Name, loadInfo.OnMaterialLoad, loadInfo.ActsOn, loadInfo.StartOn, loadInfo.DoneFor, GTSL::MoveRef(onMaterialLoadInfo));
}

void Insert(const MaterialResourceManager::MaterialInfo::Binding& materialInfo, GTSL::Buffer& buffer)
{
	Insert(materialInfo.Type, buffer);
}

void Extract(MaterialResourceManager::MaterialInfo::Binding& materialInfo, GTSL::Buffer& buffer)
{
	Extract(materialInfo.Type, buffer);
}

void Insert(const MaterialResourceManager::MaterialInfo::Uniform& materialInfo, GTSL::Buffer& buffer)
{
	Insert(materialInfo.Name, buffer); Insert(materialInfo.Type, buffer);
}

void Extract(MaterialResourceManager::MaterialInfo::Uniform& materialInfo, GTSL::Buffer& buffer)
{
	Extract(reinterpret_cast<uint64&>(materialInfo.Name), buffer); Extract(materialInfo.Type, buffer);
}

void Insert(const MaterialResourceManager::MaterialInfo& materialInfo, GTSL::Buffer& buffer)
{
	Insert(materialInfo.MaterialOffset, buffer);
	Insert(materialInfo.RenderGroup, buffer);
	Insert(materialInfo.ShaderSizes, buffer);
	Insert(materialInfo.VertexElements, buffer);
	Insert(materialInfo.BindingSets, buffer);
	Insert(materialInfo.Uniforms, buffer);
	Insert(materialInfo.ShaderTypes, buffer);
}

void Extract(MaterialResourceManager::MaterialInfo& materialInfo, GTSL::Buffer& buffer)
{
	Extract(materialInfo.MaterialOffset, buffer);
	Extract(reinterpret_cast<uint64&>(materialInfo.RenderGroup), buffer);
	Extract(materialInfo.ShaderSizes, buffer);
	Extract(materialInfo.VertexElements, buffer);
	Extract(materialInfo.BindingSets, buffer);
	Extract(materialInfo.Uniforms, buffer);
	Extract(materialInfo.ShaderTypes, buffer);
}