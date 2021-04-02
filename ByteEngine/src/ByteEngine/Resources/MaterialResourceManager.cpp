#include "MaterialResourceManager.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/Serialize.h>
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Render/RenderTypes.h"
#include "ByteEngine/Render/ShaderGenerator.h"

constexpr GTSL::ShortString<12> ShaderTypeToFileExtension(GAL::ShaderType type)
{
	switch (type)
	{
	case GAL::ShaderType::VERTEX_SHADER: return "vert";
	case GAL::ShaderType::TESSELLATION_CONTROL_SHADER: return "tesc";
	case GAL::ShaderType::TESSELLATION_EVALUATION_SHADER: return "tese";
	case GAL::ShaderType::GEOMETRY_SHADER: return "geom";
	case GAL::ShaderType::FRAGMENT_SHADER: return "frag";
	case GAL::ShaderType::COMPUTE_SHADER: return "comp";
	case GAL::ShaderType::RAY_GEN: return "rgen";
	case GAL::ShaderType::ANY_HIT: return "rahit";
	case GAL::ShaderType::CLOSEST_HIT: return "rchit";
	case GAL::ShaderType::MISS: return "rmiss";
	case GAL::ShaderType::INTERSECTION: return "rint";
	case GAL::ShaderType::CALLABLE: return "rcall";
	}
}

MaterialResourceManager::MaterialResourceManager() : ResourceManager("MaterialResourceManager"), rasterMaterialInfos(16, GetPersistentAllocator()),
rtPipelineInfos(8, GetPersistentAllocator())
{
	GTSL::Buffer<BE::TAR> rasterFileBuffer; rasterFileBuffer.Allocate((uint32)GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
	
	GTSL::StaticString<256> resources_path;
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";

	resources_path += "Materials.bepkg";
	package.OpenFile(resources_path, GTSL::File::AccessMode::READ | GTSL::File::AccessMode::WRITE);

	resources_path.Drop(resources_path.FindLast('/').Get() + 1);
	resources_path += "Materials.beidx";

	index.OpenFile(resources_path, GTSL::File::AccessMode::READ | GTSL::File::AccessMode::WRITE);
	index.ReadFile(rasterFileBuffer.GetBufferInterface());

	if (rasterFileBuffer.GetLength())
	{
		Extract(rasterMaterialInfos, rasterFileBuffer);
		Extract(rtPipelineInfos, rasterFileBuffer);
	}
	else
	{
		Insert(rasterMaterialInfos, rasterFileBuffer);
		Insert(rtPipelineInfos, rasterFileBuffer);
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
		GTSL::Buffer<BE::TAR> shader_source_buffer; shader_source_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(8)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> index_buffer; index_buffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shader_buffer; shader_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(128)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shader_error_buffer; shader_error_buffer.Allocate(GTSL::Byte(GTSL::KiloByte(4)), 8, GetTransientAllocator());
		
		RasterMaterialDataSerialize materialInfo;

		GTSL::File shader;

		materialInfo.ByteOffset = package.GetFileSize();
		
		for (uint8 i = 0; i < materialCreateInfo.ShaderTypes.ElementCount(); ++i)
		{
			shader.OpenFile(GetResourcePath(materialCreateInfo.ShaderName, ShaderTypeToFileExtension(materialCreateInfo.ShaderTypes[i])), GTSL::File::AccessMode::READ);

			GTSL::String<BE::TAR> string(1024, GetTransientAllocator());
			GenerateShader(string, materialCreateInfo.ShaderTypes[i]);
			shader_source_buffer.CopyBytes(string.GetLength() - 1, (const byte*)string.c_str());
			shader.ReadFile(shader_source_buffer.GetBufferInterface());

			auto f = GTSL::Range<const utf8*>(shader_source_buffer.GetLength(), reinterpret_cast<const utf8*>(shader_source_buffer.GetData()));
			const auto compilationResult = GAL::CompileShader(f, materialCreateInfo.ShaderName, materialCreateInfo.ShaderTypes[i], GAL::ShaderLanguage::GLSL, shader_buffer.GetBufferInterface(), shader_error_buffer.GetBufferInterface());
			*(shader_error_buffer.GetData() + (shader_error_buffer.GetLength() - 1)) = '\0';
			
			if(!compilationResult) {
				BE_LOG_ERROR(reinterpret_cast<const char*>(shader_error_buffer.GetData()));
				BE_ASSERT(false, shader_error_buffer.GetData());
			}

			materialInfo.ShaderSizes.EmplaceBack(shader_buffer.GetLength());
			package.WriteToFile(shader_buffer);

			shader_source_buffer.Resize(0);
			shader_error_buffer.Resize(0);
			shader_buffer.Resize(0);
		}

		materialInfo.VertexElements = materialCreateInfo.VertexFormat;
		materialInfo.ShaderTypes = materialCreateInfo.ShaderTypes;
		materialInfo.RenderGroup = GTSL::Id64(materialCreateInfo.RenderGroup);

		materialInfo.Parameters = materialCreateInfo.Parameters;
		materialInfo.RenderPass = materialCreateInfo.RenderPass;
		
		materialInfo.ColorBlendOperation = materialCreateInfo.ColorBlendOperation;
		materialInfo.DepthTest = materialCreateInfo.DepthTest;
		materialInfo.DepthWrite = materialCreateInfo.DepthWrite;
		materialInfo.CullMode = materialCreateInfo.CullMode;
		materialInfo.StencilTest = materialCreateInfo.StencilTest;
		materialInfo.BlendEnable = materialCreateInfo.BlendEnable;

		materialInfo.Front = materialCreateInfo.Front;
		materialInfo.Back = materialCreateInfo.Back;

		materialInfo.Parameters = materialCreateInfo.Parameters;

		materialInfo.MaterialInstances = materialCreateInfo.MaterialInstances;
		
		rasterMaterialInfos.Emplace(hashed_name, materialInfo);
		index.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
		Insert(rasterMaterialInfos, index_buffer);
		Insert(rtPipelineInfos, index_buffer);
		index.WriteToFile(index_buffer);
	}
}

void MaterialResourceManager::CreateRayTracePipeline(const RayTracePipelineCreateInfo& pipelineCreateInfo)
{
	auto searchResult = rtPipelineInfos.TryEmplace(Id(pipelineCreateInfo.PipelineName));
	if(!searchResult.State()) { return; }
	
	auto& pipeline = searchResult.Get();

	pipeline.OffsetToBinary = package.GetFileSize();
	
	for(uint32 i = 0; i < pipelineCreateInfo.Shaders.GetLength(); ++i)
	{
		const auto& shaderInfo = pipelineCreateInfo.Shaders[i];
		auto& shader = pipeline.Shaders.EmplaceBack();

		shader.ShaderName = shaderInfo.ShaderName;
		shader.ShaderType = shaderInfo.Type;
		shader.MaterialInstances = shaderInfo.MaterialInstances;

		{
			GTSL::Buffer<BE::TAR> shaderBuffer; shaderBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(128)), 8, GetTransientAllocator());
			GTSL::Buffer<BE::TAR> shaderSourceBuffer; shaderSourceBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(8)), 8, GetTransientAllocator());
			GTSL::Buffer<BE::TAR> shaderErrorBuffer; shaderErrorBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(4)), 8, GetTransientAllocator());

			GTSL::File shaderFile;

			shaderFile.OpenFile(GetResourcePath(shaderInfo.ShaderName, ShaderTypeToFileExtension(shaderInfo.Type)), GTSL::File::AccessMode::READ);

			GTSL::String<BE::TAR> string(1024, GetTransientAllocator());
			GenerateShader(string, shaderInfo.Type);
			shaderSourceBuffer.CopyBytes(string.GetLength() - 1, (const byte*)string.c_str());
			shaderFile.ReadFile(shaderSourceBuffer.GetBufferInterface());

			auto f = GTSL::Range<const utf8*>(shaderSourceBuffer.GetLength(), reinterpret_cast<const utf8*>(shaderSourceBuffer.GetData()));
			const auto compilationResult = GAL::CompileShader(f, shaderInfo.ShaderName, shaderInfo.Type, GAL::ShaderLanguage::GLSL, shaderBuffer.GetBufferInterface(), shaderErrorBuffer.GetBufferInterface());
		
			if (!compilationResult) {
				BE_LOG_ERROR(reinterpret_cast<const char*>(shaderErrorBuffer.GetData()));
				BE_ASSERT(false, shaderErrorBuffer.GetData());
			}

			shader.BinarySize = shaderBuffer.GetLength();
			
			package.WriteToFile(shaderBuffer);
		}
	}

	{
		index.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
		GTSL::Buffer<BE::TAR> indexFileBuffer; indexFileBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(512)), 8, GetTransientAllocator());
		Insert(rasterMaterialInfos, indexFileBuffer);
		Insert(rtPipelineInfos, indexFileBuffer);
		index.WriteToFile(indexFileBuffer);
	}
}

void MaterialResourceManager::GetMaterialSize(const Id name, uint32& size)
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

	BE_ASSERT(materialInfo.ByteOffset != materialInfo.ShaderSizes[0], ":|");
	
	package.SetPointer(materialInfo.ByteOffset, GTSL::File::MoveFrom::BEGIN);

	[[maybe_unused]] const auto read = package.ReadFromFile(loadInfo.DataBuffer);
	BE_ASSERT(read != 0, "Read 0 bytes!");
	
	OnMaterialLoadInfo onMaterialLoadInfo;
	onMaterialLoadInfo.ResourceName = loadInfo.Name;
	onMaterialLoadInfo.UserData = loadInfo.UserData;
	onMaterialLoadInfo.DataBuffer = loadInfo.DataBuffer;
	onMaterialLoadInfo.ShaderTypes = materialInfo.ShaderTypes;
	onMaterialLoadInfo.ShaderSizes = materialInfo.ShaderSizes;
	onMaterialLoadInfo.RenderGroup = materialInfo.RenderGroup;
	onMaterialLoadInfo.VertexElements = materialInfo.VertexElements;
	onMaterialLoadInfo.RenderPass = materialInfo.RenderPass;
	onMaterialLoadInfo.Parameters = materialInfo.Parameters;
	onMaterialLoadInfo.ColorBlendOperation = materialInfo.ColorBlendOperation;
	onMaterialLoadInfo.DepthTest = materialInfo.DepthTest;
	onMaterialLoadInfo.DepthWrite = materialInfo.DepthWrite;
	onMaterialLoadInfo.StencilTest = materialInfo.StencilTest;
	onMaterialLoadInfo.CullMode = materialInfo.CullMode;
	onMaterialLoadInfo.BlendEnable = materialInfo.BlendEnable;
	onMaterialLoadInfo.Front = materialInfo.Front;
	onMaterialLoadInfo.Back = materialInfo.Back;
	onMaterialLoadInfo.MaterialInstances = materialInfo.MaterialInstances;
	
	auto functionName = GTSL::StaticString<64>("Load Material: "); functionName += loadInfo.Name();
	loadInfo.GameInstance->AddDynamicTask(Id(functionName.begin()), loadInfo.OnMaterialLoad, loadInfo.ActsOn, GTSL::MoveRef(onMaterialLoadInfo));
}

void MaterialResourceManager::LoadRayTraceShadersForPipeline(const RayTracePipelineInfo& info, GTSL::Range<byte*> buffer)
{
	uint32 pipelineSize = 0;

	for (uint32 s = 0; s < info.Shaders.GetLength(); ++s) {
		pipelineSize += info.Shaders[s].BinarySize;
	}

	BE_ASSERT(pipelineSize <= buffer.Bytes(), "Buffer can't hold required data!");

	package.SetPointer(info.OffsetToBinary, GTSL::File::MoveFrom::BEGIN);

	[[maybe_unused]] const auto read = package.ReadFromFile(buffer);
	BE_ASSERT(read != 0, "Read 0 bytes!");
}
