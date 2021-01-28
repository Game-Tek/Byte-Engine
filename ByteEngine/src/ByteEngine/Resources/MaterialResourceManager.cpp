#include "MaterialResourceManager.h"

#include <GTSL/Buffer.hpp>
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

MaterialResourceManager::MaterialResourceManager() : ResourceManager("MaterialResourceManager"), rasterMaterialInfos(16, GetPersistentAllocator()),
rtMaterialInfos(16, GetPersistentAllocator()), rtHandles(16, GetPersistentAllocator())
{
	GTSL::Buffer<BE::TAR> rasterFileBuffer; rasterFileBuffer.Allocate((uint32)GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
	
	GTSL::StaticString<256> resources_path;
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";

	resources_path += "Materials.bepkg";
	package.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ | (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::LEAVE_CONTENTS);

	resources_path.Drop(resources_path.FindLast('/') + 1);
	resources_path += "Materials.beidx";

	index.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ | (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::LEAVE_CONTENTS);
	index.ReadFile(rasterFileBuffer.GetBufferInterface());

	if (rasterFileBuffer.GetLength())
	{
		Extract(rasterMaterialInfos, rasterFileBuffer);
		Extract(rtMaterialInfos, rasterFileBuffer);
	}
	else
	{
		Insert(rasterMaterialInfos, rasterFileBuffer);
		Insert(rtMaterialInfos, rasterFileBuffer);
	}
}

MaterialResourceManager::~MaterialResourceManager()
{
}

void MaterialResourceManager::CreateRasterMaterial(const RasterMaterialCreateInfo& materialCreateInfo)
{
	const auto hashed_name = GTSL::Id64(materialCreateInfo.ShaderName);
	
	if (!rasterMaterialInfos.Find(hashed_name))
	{
		GTSL::Buffer<BE::TAR> shader_source_buffer; shader_source_buffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> index_buffer; index_buffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shader_buffer; shader_buffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shader_error_buffer; shader_error_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(512)), 8, GetTransientAllocator());
		
		RasterMaterialInfo materialInfo;
		
		GTSL::StaticString<256> resources_path;
		resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";

		GTSL::File shader;

		materialInfo.MaterialOffset = package.GetFileSize();
		
		for (uint8 i = 0; i < materialCreateInfo.ShaderTypes.ElementCount(); ++i)
		{
			resources_path += materialCreateInfo.ShaderName; resources_path += TYPE_TO_EXTENSION[static_cast<uint8>(materialCreateInfo.ShaderTypes[i])];

			shader.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
			shader.ReadFile(shader_source_buffer.GetBufferInterface());

			auto f = GTSL::Range<const UTF8*>(shader_source_buffer.GetLength(), reinterpret_cast<const UTF8*>(shader_source_buffer.GetData()));
			const auto comp_res = GAL::CompileShader(f, resources_path, static_cast<GAL::ShaderType>(materialCreateInfo.ShaderTypes[i]), GAL::ShaderLanguage::GLSL, shader_buffer.GetBufferInterface(), shader_error_buffer.GetBufferInterface());
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
		
		rasterMaterialInfos.Emplace(hashed_name, materialInfo);
		index.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
		Insert(rasterMaterialInfos, index_buffer);
		Insert(rtMaterialInfos, index_buffer);
		index.WriteToFile(index_buffer);
	}
}

void MaterialResourceManager::CreateRayTraceMaterial(const RayTraceMaterialCreateInfo& materialCreateInfo)
{
	const auto hashed_name = GTSL::Id64(materialCreateInfo.ShaderName);

	if (!rtMaterialInfos.Find(hashed_name))
	{
		GTSL::Buffer<BE::TAR> shader_source_buffer; shader_source_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(512)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> index_buffer; index_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(512)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shader_buffer; shader_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(512)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shader_error_buffer; shader_error_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(8)), 8, GetTransientAllocator());

		RayTraceMaterialInfo materialInfo;
		
		GTSL::StaticString<256> resources_path;
		resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";

		GTSL::File shader;

		materialInfo.OffsetToBinary = package.GetFileSize();

		resources_path += materialCreateInfo.ShaderName; resources_path += TYPE_TO_EXTENSION[static_cast<uint8>(materialCreateInfo.Type)];

		shader.OpenFile(resources_path, (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
		shader.ReadFile(shader_source_buffer.GetBufferInterface());

		auto f = GTSL::Range<const UTF8*>(shader_source_buffer.GetLength(), reinterpret_cast<const UTF8*>(shader_source_buffer.GetData()));
		const auto comp_res = GAL::CompileShader(f, resources_path, materialCreateInfo.Type, GAL::ShaderLanguage::GLSL, shader_buffer.GetBufferInterface(), shader_error_buffer.GetBufferInterface());
		*(shader_error_buffer.GetData() + (shader_error_buffer.GetLength() - 1)) = '\0';
		if (comp_res == false)
		{
			BE_LOG_ERROR(reinterpret_cast<const char*>(shader_error_buffer.GetData()));
		}
		BE_ASSERT(comp_res != false, shader_error_buffer.GetData());

		materialInfo.ShaderInfo.BinarySize = shader_buffer.GetLength();
		materialInfo.ShaderInfo.ColorBlendOperation = materialCreateInfo.ColorBlendOperation;
		materialInfo.ShaderInfo.ShaderType = materialCreateInfo.Type;
		package.WriteToFile(shader_buffer);

		resources_path.Drop(resources_path.FindLast('/') + 1);

		shader_source_buffer.Resize(0);
		shader_error_buffer.Resize(0);
		shader_buffer.Resize(0);

		rtMaterialInfos.Emplace(hashed_name, materialInfo);
		index.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
		Insert(rasterMaterialInfos, index_buffer);
		Insert(rtMaterialInfos, index_buffer);
		index.WriteToFile(index_buffer);
	}
	
	rtHandles.EmplaceBack(hashed_name);
}

void MaterialResourceManager::GetMaterialSize(const GTSL::Id64 name, uint32& size)
{
	GTSL::ReadLock lock(mutex);
	for(auto& e : rasterMaterialInfos.At(name).ShaderSizes) { size += e; }
}

void MaterialResourceManager::LoadMaterial(const MaterialLoadInfo& loadInfo)
{
	auto materialInfo = rasterMaterialInfos.At(loadInfo.Name);

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
	
	onMaterialLoadInfo.VertexElements = GTSL::Range<GAL::ShaderDataType*>(materialInfo.VertexElements.GetLength(), reinterpret_cast<GAL::ShaderDataType*>(materialInfo.VertexElements.begin()));
	
	auto functionName = GTSL::StaticString<64>("Load Material: "); functionName += loadInfo.Name;
	loadInfo.GameInstance->AddDynamicTask(Id(functionName.begin()), loadInfo.OnMaterialLoad, loadInfo.ActsOn, GTSL::MoveRef(onMaterialLoadInfo));
}

MaterialResourceManager::OnMaterialLoadInfo MaterialResourceManager::LoadMaterialSynchronous(uint64 id, GTSL::Range<byte*> buffer)
{
	auto materialInfo = rasterMaterialInfos.At(id);

	uint32 mat_size = 0;
	for (auto e : materialInfo.ShaderSizes) { mat_size += e; }
	BE_ASSERT(mat_size <= buffer.Bytes(), "Buffer can't hold required data!");

	BE_ASSERT(materialInfo.MaterialOffset != materialInfo.ShaderSizes[0], ":|");

	package.SetPointer(materialInfo.MaterialOffset, GTSL::File::MoveFrom::BEGIN);

	[[maybe_unused]] const auto read = package.ReadFromFile(buffer);
	BE_ASSERT(read != 0, "Read 0 bytes!");

	OnMaterialLoadInfo onMaterialLoadInfo;
	onMaterialLoadInfo.ResourceName = GTSL::Id64();
	onMaterialLoadInfo.DataBuffer = buffer;
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

	onMaterialLoadInfo.VertexElements = GTSL::Range<GAL::ShaderDataType*>(materialInfo.VertexElements.GetLength(), reinterpret_cast<GAL::ShaderDataType*>(materialInfo.VertexElements.begin()));

	return onMaterialLoadInfo;
}

MaterialResourceManager::RayTracingShaderInfo MaterialResourceManager::LoadRayTraceShaderSynchronous(Id id, GTSL::Range<byte*> buffer)
{
	auto materialInfo = rtMaterialInfos.At(id());

	uint32 mat_size = materialInfo.ShaderInfo.BinarySize;
	BE_ASSERT(mat_size <= buffer.Bytes(), "Buffer can't hold required data!");

	package.SetPointer(materialInfo.OffsetToBinary, GTSL::File::MoveFrom::BEGIN);

	[[maybe_unused]] const auto read = package.ReadFromFile(buffer);
	BE_ASSERT(read != 0, "Read 0 bytes!");

	return materialInfo.ShaderInfo;
}