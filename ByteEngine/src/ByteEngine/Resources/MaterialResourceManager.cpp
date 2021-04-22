#include "MaterialResourceManager.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/Filesystem.h>
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

	package.OpenFile(GetResourcePath(GTSL::ShortString<32>("Materials"), GTSL::ShortString<32>("bepkg")), GTSL::File::AccessMode::READ | GTSL::File::AccessMode::WRITE);

	index.OpenFile(GetResourcePath(GTSL::ShortString<32>("Materials"), GTSL::ShortString<32>("beidx")), GTSL::File::AccessMode::READ | GTSL::File::AccessMode::WRITE);
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
		GTSL::Buffer<BE::TAR> shaderSourceBuffer; shaderSourceBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(8)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> indexBuffer; indexBuffer.Allocate(GTSL::Byte(GTSL::MegaByte(1)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shaderBuffer; shaderBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(128)), 8, GetTransientAllocator());
		GTSL::Buffer<BE::TAR> shaderErrorBuffer; shaderErrorBuffer.Allocate(GTSL::Byte(GTSL::KiloByte(4)), 8, GetTransientAllocator());
		
		RasterMaterialDataSerialize materialData;

		materialData.ByteOffset = package.GetFileSize();

		GTSL::FileQuery fileQuery(GetResourcePath(materialCreateInfo.ShaderName, GTSL::ShortString<2>("*")));

		while(fileQuery.DoQuery())
		{
			auto dotSearch = fileQuery.GetFileNameWithExtension().FindLast('.');
			
			auto extension = GTSL::Range<const utf8*>(dotSearch.Get().First, fileQuery.GetFileNameWithExtension().begin() + dotSearch.Get().Second + 1/*dot*/);

			GAL::ShaderType shaderType;

			switch (GTSL::Id64(extension)())
			{
			case GTSL::Hash("vert"): shaderType = GAL::ShaderType::VERTEX_SHADER; break;
			case GTSL::Hash("frag"): shaderType = GAL::ShaderType::FRAGMENT_SHADER; break;
			case GTSL::Hash("rchit"): shaderType = GAL::ShaderType::CLOSEST_HIT; break;
			case GTSL::Hash("rgen"): shaderType = GAL::ShaderType::RAY_GEN; break;
			case GTSL::Hash("rmiss"): shaderType = GAL::ShaderType::MISS; break;
			default: return;
			}
			
			GTSL::File shaderSourceFile;
			shaderSourceFile.OpenFile(GetResourcePath(fileQuery.GetFileNameWithExtension()), GTSL::File::AccessMode::READ);
			
			GTSL::String<BE::TAR> string(1024, GetTransientAllocator());
			GenerateShader(string, shaderType);

			switch (shaderType)
			{
			case GAL::ShaderType::VERTEX_SHADER:
				AddVertexShaderLayout(string, materialCreateInfo.Permutations[0]);
				break;
			}

			shaderSourceBuffer.CopyBytes(string.GetLength() - 1, (const byte*)string.c_str());

			shaderSourceFile.ReadFile(shaderSourceBuffer.GetBufferInterface());
			
			auto f = GTSL::Range<const utf8*>(shaderSourceBuffer.GetLength(), reinterpret_cast<const utf8*>(shaderSourceBuffer.GetData()));
			const auto compilationResult = GAL::CompileShader(f, materialCreateInfo.ShaderName, shaderType, GAL::ShaderLanguage::GLSL, shaderBuffer.GetBufferInterface(), shaderErrorBuffer.GetBufferInterface());

			//BE_LOG_MESSAGE(reinterpret_cast<const char*>(shaderSourceBuffer.GetData()));
			
			if (!compilationResult) {
				BE_LOG_ERROR(reinterpret_cast<const char*>(shaderErrorBuffer.GetData()));
				BE_ASSERT(false, shaderErrorBuffer.GetData());
			}

			package.WriteToFile(shaderBuffer);

			auto& shader = materialData.Shaders.EmplaceBack();
			shader.Size = shaderBuffer.GetLength();
			shader.Type = shaderType;
			
			shaderSourceBuffer.Resize(0);
			shaderErrorBuffer.Resize(0);
			shaderBuffer.Resize(0);
		}
		
		for (uint8 i = 0; i < materialCreateInfo.MaterialInstances.GetLength(); ++i)
		{
			const auto& materialInstanceInfo = materialCreateInfo.MaterialInstances[i];

			materialData.MaterialInstances.EmplaceBack();
			materialData.MaterialInstances.back().Name = materialInstanceInfo.Name;
			materialData.MaterialInstances.back().Parameters = materialInstanceInfo.Parameters;
		}

		materialData.RenderGroup = GTSL::Id64(materialCreateInfo.RenderGroup);

		materialData.Parameters = materialCreateInfo.Parameters;
		materialData.RenderPass = materialCreateInfo.RenderPass;
		
		materialData.ColorBlendOperation = materialCreateInfo.ColorBlendOperation;
		materialData.DepthTest = materialCreateInfo.DepthTest;
		materialData.DepthWrite = materialCreateInfo.DepthWrite;
		materialData.CullMode = materialCreateInfo.CullMode;
		materialData.StencilTest = materialCreateInfo.StencilTest;
		materialData.BlendEnable = materialCreateInfo.BlendEnable;

		materialData.Front = materialCreateInfo.Front;
		materialData.Back = materialCreateInfo.Back;

		materialData.Parameters = materialCreateInfo.Parameters;

		materialData.MaterialInstances = materialCreateInfo.MaterialInstances;

		for(const auto& p : materialCreateInfo.Permutations)
		{
			auto& permutation = materialData.Permutations.EmplaceBack(p);
			//permutation.VertexElements.EmplaceBack();
		}
		
		rasterMaterialInfos.Emplace(hashed_name, materialData);
		index.SetPointer(0, GTSL::File::MoveFrom::BEGIN);
		Insert(rasterMaterialInfos, indexBuffer);
		Insert(rtPipelineInfos, indexBuffer);
		index.WriteToFile(indexBuffer);
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
	for(auto& e : rasterMaterialInfos.At(name).Shaders) {
		size += e.Size;
	}
}

MaterialResourceManager::OnMaterialLoadInfo MaterialResourceManager::LoadMaterial(const MaterialLoadInfo& loadInfo)
{
	auto materialInfo = rasterMaterialInfos.At(loadInfo.Name);

	uint32 mat_size = 0;
	for (auto e : materialInfo.Shaders)
	{
		mat_size += e.Size;
	}
	
	BE_ASSERT(mat_size <= loadInfo.DataBuffer.Bytes(), "Buffer can't hold required data!");
	
	package.SetPointer(materialInfo.ByteOffset, GTSL::File::MoveFrom::BEGIN);

	[[maybe_unused]] const auto read = package.ReadFromFile(loadInfo.DataBuffer);
	BE_ASSERT(read != 0, "Read 0 bytes!");
	
	OnMaterialLoadInfo onMaterialLoadInfo;
	onMaterialLoadInfo.ResourceName = loadInfo.Name;
	onMaterialLoadInfo.UserData = loadInfo.UserData;
	onMaterialLoadInfo.DataBuffer = loadInfo.DataBuffer;
	onMaterialLoadInfo.RenderGroup = materialInfo.RenderGroup;
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
	onMaterialLoadInfo.Permutations = materialInfo.Permutations;
	onMaterialLoadInfo.Shaders = materialInfo.Shaders;

	auto tt = onMaterialLoadInfo; //copy because MoveRef into dynamic task removes contents
	
	auto functionName = GTSL::StaticString<64>("Load Material: "); functionName += loadInfo.Name();
	loadInfo.GameInstance->AddDynamicTask(Id(functionName.begin()), loadInfo.OnMaterialLoad, loadInfo.ActsOn, GTSL::MoveRef(onMaterialLoadInfo));

	return tt;
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
