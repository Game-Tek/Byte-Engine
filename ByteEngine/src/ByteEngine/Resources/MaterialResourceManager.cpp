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

			auto f = GTSL::Range<const UTF8*>(shader_source_buffer.GetLength(), reinterpret_cast<const UTF8*>(shader_source_buffer.GetData()));
			const auto comp_res = Shader::CompileShader(f, resources_path, static_cast<GAL::ShaderType>(materialCreateInfo.ShaderTypes[i]), GAL::ShaderLanguage::GLSL, shader_buffer, shader_error_buffer);
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

		materialInfo.VertexElements = GTSL::Range<const VertexElementsType*>(materialCreateInfo.VertexFormat.ElementCount(), reinterpret_cast<const VertexElementsType*>(materialCreateInfo.VertexFormat.begin()));
		materialInfo.ShaderTypes = GTSL::Range<const ShaderTypeType*>(materialCreateInfo.ShaderTypes.ElementCount(), reinterpret_cast<const ShaderTypeType*>(materialCreateInfo.ShaderTypes.begin()));
		materialInfo.RenderGroup = GTSL::Id64(materialCreateInfo.RenderGroup);
		
		materialInfo.RenderPass = materialCreateInfo.RenderPass;
		
		materialInfo.ColorBlendOperation = materialCreateInfo.ColorBlendOperation;
		materialInfo.DepthTest = materialCreateInfo.DepthTest;
		materialInfo.DepthWrite = materialCreateInfo.DepthWrite;
		materialInfo.CullMode = materialCreateInfo.CullMode;
		materialInfo.StencilTest = materialCreateInfo.StencilTest;
		materialInfo.BlendEnable = materialCreateInfo.BlendEnable;

		materialInfo.Front = materialCreateInfo.Front;
		materialInfo.Back = materialCreateInfo.Back;

		materialInfo.MaterialParameters = materialCreateInfo.MaterialParameters;
		materialInfo.Textures = materialCreateInfo.Textures;
		materialInfo.PerInstanceParameters = materialCreateInfo.PerInstanceParameters;
		
		materialInfo.BindingSets = materialCreateInfo.Bindings;
		
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
	onMaterialLoadInfo.ShaderTypes = GTSL::Range<GAL::ShaderType*>(materialInfo.ShaderTypes.GetLength(), reinterpret_cast<GAL::ShaderType*>(materialInfo.ShaderTypes.begin()));
	onMaterialLoadInfo.ShaderSizes = materialInfo.ShaderSizes;
	onMaterialLoadInfo.RenderGroup = materialInfo.RenderGroup;
	
	onMaterialLoadInfo.RenderPass = materialInfo.RenderPass;

	onMaterialLoadInfo.MaterialParameters = materialInfo.MaterialParameters;
	onMaterialLoadInfo.Textures = materialInfo.Textures;
	onMaterialLoadInfo.PerInstanceParameters = materialInfo.PerInstanceParameters;
	
	onMaterialLoadInfo.ColorBlendOperation = materialInfo.ColorBlendOperation;
	onMaterialLoadInfo.DepthTest = materialInfo.DepthTest;
	onMaterialLoadInfo.DepthWrite = materialInfo.DepthWrite;
	onMaterialLoadInfo.StencilTest = materialInfo.StencilTest;
	onMaterialLoadInfo.CullMode = materialInfo.CullMode;
	onMaterialLoadInfo.BlendEnable = materialInfo.BlendEnable;
	onMaterialLoadInfo.Front = materialInfo.Front;
	onMaterialLoadInfo.Back = materialInfo.Back;
	
	onMaterialLoadInfo.BindingSets = materialInfo.BindingSets;
	
	onMaterialLoadInfo.VertexElements = GTSL::Range<GAL::ShaderDataType*>(materialInfo.VertexElements.GetLength(), reinterpret_cast<GAL::ShaderDataType*>(materialInfo.VertexElements.begin()));
	
	loadInfo.GameInstance->AddAsyncTask(loadInfo.OnMaterialLoad, GTSL::MoveRef(onMaterialLoadInfo));
}

void Insert(const MaterialResourceManager::Binding& materialInfo, GTSL::Buffer& buffer)
{
	Insert(materialInfo.Type, buffer);
	Insert(materialInfo.Stage, buffer);
}

void Extract(MaterialResourceManager::Binding& materialInfo, GTSL::Buffer& buffer)
{
	Extract(materialInfo.Type, buffer);
	Extract(materialInfo.Stage, buffer);
}

void Insert(const MaterialResourceManager::Uniform& materialInfo, GTSL::Buffer& buffer)
{
	Insert(materialInfo.Name, buffer);
	Insert(materialInfo.Type, buffer);
}

void Extract(MaterialResourceManager::Uniform& materialInfo, GTSL::Buffer& buffer)
{
	Extract(materialInfo.Name, buffer);
	Extract(materialInfo.Type, buffer);
}

void Insert(const MaterialResourceManager::MaterialInfo& materialInfo, GTSL::Buffer& buffer)
{
	Insert(materialInfo.MaterialOffset, buffer);
	Insert(materialInfo.RenderGroup, buffer);
	Insert(materialInfo.RenderPass, buffer);
	
	Insert(materialInfo.ShaderSizes, buffer);
	Insert(materialInfo.VertexElements, buffer);
	Insert(materialInfo.BindingSets, buffer);
	Insert(materialInfo.ShaderTypes, buffer);
	
	Insert(materialInfo.Textures, buffer);
	
	Insert(materialInfo.DepthTest, buffer);
	Insert(materialInfo.DepthWrite, buffer);
	Insert(materialInfo.StencilTest, buffer);
	Insert(materialInfo.CullMode, buffer);
	Insert(materialInfo.ColorBlendOperation, buffer);
	Insert(materialInfo.BlendEnable, buffer);

	Insert(materialInfo.MaterialParameters, buffer);
	Insert(materialInfo.PerInstanceParameters, buffer);

	Insert(materialInfo.Front, buffer);
	Insert(materialInfo.Back, buffer);
}

void Extract(MaterialResourceManager::MaterialInfo& materialInfo, GTSL::Buffer& buffer)
{
	Extract(materialInfo.MaterialOffset, buffer);
	Extract(materialInfo.RenderGroup, buffer);
	Extract(materialInfo.RenderPass, buffer);
	
	Extract(materialInfo.ShaderSizes, buffer);
	Extract(materialInfo.VertexElements, buffer);
	Extract(materialInfo.BindingSets, buffer);
	Extract(materialInfo.ShaderTypes, buffer);

	Extract(materialInfo.Textures, buffer);
	
	Extract(materialInfo.DepthTest, buffer);
	Extract(materialInfo.DepthWrite, buffer);
	Extract(materialInfo.StencilTest, buffer);
	Extract(materialInfo.CullMode, buffer);
	Extract(materialInfo.ColorBlendOperation, buffer);
	Extract(materialInfo.BlendEnable, buffer);

	Extract(materialInfo.MaterialParameters, buffer);
	Extract(materialInfo.PerInstanceParameters, buffer);
	
	Extract(materialInfo.Front, buffer);
	Extract(materialInfo.Back, buffer);
}

void Insert(const MaterialResourceManager::StencilState& stencilState, GTSL::Buffer& buffer)
{
	Insert(stencilState.FailOperation, buffer);
	Insert(stencilState.PassOperation, buffer);
	Insert(stencilState.DepthFailOperation, buffer);
	Insert(stencilState.CompareOperation, buffer);
	Insert(stencilState.CompareMask, buffer);
	Insert(stencilState.WriteMask, buffer);
	Insert(stencilState.Reference, buffer);
}

void Extract(MaterialResourceManager::StencilState& stencilState, GTSL::Buffer& buffer)
{
	Extract(stencilState.FailOperation, buffer);
	Extract(stencilState.PassOperation, buffer);
	Extract(stencilState.DepthFailOperation, buffer);
	Extract(stencilState.CompareOperation, buffer);
	Extract(stencilState.CompareMask, buffer);
	Extract(stencilState.WriteMask, buffer);
	Extract(stencilState.Reference, buffer);
}
