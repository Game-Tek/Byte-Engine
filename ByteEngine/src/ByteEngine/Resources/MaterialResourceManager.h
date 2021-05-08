#pragma once

#include <GAL/Pipelines.h>
#include <GAL/RenderCore.h>
#include <GTSL/Algorithm.h>
#include <GTSL/Array.hpp>
#include <GTSL/Delegate.hpp>
#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Math/Vectors.h>

#include "ResourceManager.h"

#include "ByteEngine/Game/GameInstance.h"

class MaterialResourceManager final : public ResourceManager
{
public:
	MaterialResourceManager();
	~MaterialResourceManager();
	void GetShaderSize(Id id, uint32* shaderSize);

	enum class ParameterType : uint8
	{
		UINT32, FVEC4,
		TEXTURE_REFERENCE, BUFFER_REFERENCE
	};
	
	struct Parameter
	{
		GTSL::Id64 Name;
		ParameterType Type;

		Parameter() = default;
		Parameter(const GTSL::Id64 name, const ParameterType type) : Name(name), Type(type) {}

		template<class ALLOC>
		friend void Insert(const Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(parameterInfo.Name, buffer);
			Insert(parameterInfo.Type, buffer);
		}
		
		template<class ALLOC>
		friend void Extract(Parameter& parameterInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(parameterInfo.Name, buffer);
			Extract(parameterInfo.Type, buffer);
		}
	};
	
	struct StencilState
	{
		GAL::StencilCompareOperation FailOperation = GAL::StencilCompareOperation::KEEP;
		GAL::StencilCompareOperation PassOperation = GAL::StencilCompareOperation::KEEP;
		GAL::StencilCompareOperation DepthFailOperation = GAL::StencilCompareOperation::KEEP;
		GAL::CompareOperation CompareOperation = GAL::CompareOperation::NEVER;
		GTSL::uint32 CompareMask = 0;
		GTSL::uint32 WriteMask = 0;
		GTSL::uint32 Reference = 0;

		template<class ALLOC>
		friend void Insert(const StencilState& stencilState, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(stencilState.FailOperation, buffer);
			Insert(stencilState.PassOperation, buffer);
			Insert(stencilState.DepthFailOperation, buffer);
			Insert(stencilState.CompareOperation, buffer);
			Insert(stencilState.CompareMask, buffer);
			Insert(stencilState.WriteMask, buffer);
			Insert(stencilState.Reference, buffer);
		}
		
		template<class ALLOC>
		friend void Extract(StencilState& stencilState, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(stencilState.FailOperation, buffer);
			Extract(stencilState.PassOperation, buffer);
			Extract(stencilState.DepthFailOperation, buffer);
			Extract(stencilState.CompareOperation, buffer);
			Extract(stencilState.CompareMask, buffer);
			Extract(stencilState.WriteMask, buffer);
			Extract(stencilState.Reference, buffer);
		}
	};
	
	struct MaterialInstance
	{
		MaterialInstance() = default;
		
		union ParameterData
		{
			ParameterData() = default;
			
			uint32 uint32 = 0;
			GTSL::Vector4 Vector4;
			GTSL::Id64 TextureReference;
			uint64 BufferReference;

			template<class ALLOCATOR>
			friend void Insert(const ParameterData& uni, GTSL::Buffer<ALLOCATOR>& buffer) //if trivially copyable
			{
				buffer.CopyBytes(sizeof(ParameterData), reinterpret_cast<const byte*>(&uni));
			}

			template<class ALLOCATOR>
			friend void Extract(ParameterData& uni, GTSL::Buffer<ALLOCATOR>& buffer)
			{
				buffer.ReadBytes(sizeof(ParameterData), reinterpret_cast<byte*>(&uni));
			}
		};

		GTSL::Id64 Name;
		GTSL::Array<GTSL::Pair<GTSL::Id64, ParameterData>, 16> Parameters;

		template<class ALLOC>
		friend void Insert(const MaterialInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(materialInstance.Name, buffer);
			Insert(materialInstance.Parameters, buffer);
		}

		template<class ALLOC>
		friend void Extract(MaterialInstance& materialInstance, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(materialInstance.Name, buffer);
			Extract(materialInstance.Parameters, buffer);
		}

	};

	struct Shader {
		GAL::ShaderType Type; uint32 Size;

		template<class ALLOC>
		friend void Insert(const Shader& shader, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(shader.Type, buffer);
			Insert(shader.Size, buffer);
		}

		template<class ALLOC>
		friend void Extract(Shader& shader, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(shader.Type, buffer);
			Extract(shader.Size, buffer);
		}
	};
	
	struct RasterMaterialData : Data
	{
		GTSL::Id64 RenderGroup;
		bool DepthWrite; bool DepthTest; bool StencilTest;
		GAL::CullMode CullMode;
		GTSL::Id64 RenderPass;

		GTSL::Array<Parameter, 16> Parameters;
		
		GAL::BlendOperation ColorBlendOperation;

		StencilState Front;
		StencilState Back;
		bool BlendEnable = false;

		GTSL::Array<MaterialInstance, 16> MaterialInstances;
		GTSL::Array<Shader, 16> Shaders;

		struct VertexElement
		{
			GTSL::ShortString<32> VertexAttribute;
			uint8 Index = 0;
			GAL::ShaderDataType Type;

			template<class ALLOC>
			friend void Insert(const VertexElement& vertexElement, GTSL::Buffer<ALLOC>& buffer)
			{
				Insert(vertexElement.VertexAttribute, buffer);
				Insert(vertexElement.Index, buffer);
				Insert(vertexElement.Type, buffer);
			}

			template<class ALLOC>
			friend void Extract(VertexElement& vertexElement, GTSL::Buffer<ALLOC>& buffer)
			{
				Extract(vertexElement.VertexAttribute, buffer);
				Extract(vertexElement.Index, buffer);
				Extract(vertexElement.Type, buffer);
			}
		};
		
		struct Permutation
		{
			GTSL::Array<VertexElement, 20> VertexElements;

			template<class ALLOC>
			friend void Insert(const Permutation& permutation, GTSL::Buffer<ALLOC>& buffer)
			{
				Insert(permutation.VertexElements, buffer);
			}

			template<class ALLOC>
			friend void Extract(Permutation& permutation, GTSL::Buffer<ALLOC>& buffer)
			{
				Extract(permutation.VertexElements, buffer);
			}
		};

		GTSL::Array<Permutation, 8> Permutations;
	};

	struct RasterMaterialDataSerialize : DataSerialize<RasterMaterialData>
	{
		INSERT_START(RasterMaterialDataSerialize)
		{
			INSERT_BODY
			Insert(insertInfo.RenderGroup, buffer);
			Insert(insertInfo.RenderPass, buffer);
			Insert(insertInfo.DepthTest, buffer);
			Insert(insertInfo.DepthWrite, buffer);
			Insert(insertInfo.StencilTest, buffer);
			Insert(insertInfo.CullMode, buffer);
			Insert(insertInfo.ColorBlendOperation, buffer);
			Insert(insertInfo.BlendEnable, buffer);
			Insert(insertInfo.Parameters, buffer);
			Insert(insertInfo.Front, buffer);
			Insert(insertInfo.Back, buffer);
			Insert(insertInfo.MaterialInstances, buffer);
			Insert(insertInfo.Shaders, buffer);
			Insert(insertInfo.Permutations, buffer);
		}

		EXTRACT_START(RasterMaterialDataSerialize)
		{
			EXTRACT_BODY
			Extract(extractInfo.RenderGroup, buffer);
			Extract(extractInfo.RenderPass, buffer);
			Extract(extractInfo.DepthTest, buffer);
			Extract(extractInfo.DepthWrite, buffer);
			Extract(extractInfo.StencilTest, buffer);
			Extract(extractInfo.CullMode, buffer);
			Extract(extractInfo.ColorBlendOperation, buffer);
			Extract(extractInfo.BlendEnable, buffer);
			Extract(extractInfo.Parameters, buffer);
			Extract(extractInfo.Front, buffer);
			Extract(extractInfo.Back, buffer);
			Extract(extractInfo.MaterialInstances, buffer);
			Extract(extractInfo.Shaders, buffer);
			Extract(extractInfo.Permutations, buffer);
		}
	};
	
	struct RasterMaterialCreateInfo
	{		
		GTSL::StaticString<64> ShaderName;
		GTSL::StaticString<64> RenderGroup;
		GTSL::Id64 RenderPass;

		GTSL::Array<Parameter, 16> Parameters;
		GTSL::Array<Parameter, 8> PerInstanceParameters;
		
		bool DepthWrite;
		bool DepthTest;
		GAL::CullMode CullMode;
		GAL::BlendOperation ColorBlendOperation;

		StencilState Front;
		StencilState Back;
		bool StencilTest;
		bool BlendEnable = false;

		GTSL::Array<MaterialInstance, 16> MaterialInstances;
		GTSL::Array<GTSL::Array<RasterMaterialData::VertexElement, 16>, 8> Permutations;
	};
	void CreateRasterMaterial(const RasterMaterialCreateInfo& materialCreateInfo);

	struct RayTracePipelineCreateInfo
	{
		struct ShaderCreateInfo
		{
			GTSL::ShortString<64> ShaderName;
			GAL::ShaderType Type;
			
			GTSL::Array<GTSL::Array<GTSL::ShortString<64>, 8>, 16> MaterialInstances;
		};

		GTSL::Array<ShaderCreateInfo, 8> Shaders;

		GTSL::Array<ParameterType, 8> Payload;
		uint8 RecursionDepth;
		GTSL::ShortString<64> PipelineName;
	};
	void CreateRayTracePipeline(const RayTracePipelineCreateInfo& pipelineCreateInfo);
	
	void GetMaterialSize(const Id name, uint32& size);

	struct RayTracingShaderInfo
	{
		/**
		 * \brief Size of the precompiled binary blob to be provided to the API.
		 */
		uint32 BinarySize;

		GTSL::ShortString<64> ShaderName;
		GAL::ShaderType ShaderType;
		GTSL::Array<GTSL::Array<GTSL::ShortString<64>, 8>, 8> MaterialInstances;

		template<class ALLOC>
		friend void Insert(const MaterialResourceManager::RayTracingShaderInfo& shaderInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(shaderInfo.BinarySize, buffer);
			Insert(shaderInfo.ShaderName, buffer);
			Insert(shaderInfo.ShaderType, buffer);
			Insert(shaderInfo.MaterialInstances, buffer);
		}

		template<class ALLOC>
		friend void Extract(MaterialResourceManager::RayTracingShaderInfo& shaderInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(shaderInfo.BinarySize, buffer);
			Extract(shaderInfo.ShaderName, buffer);
			Extract(shaderInfo.ShaderType, buffer);
			Extract(shaderInfo.MaterialInstances, buffer);
		}
	};

	struct RayTracePipelineInfo
	{
		uint32 OffsetToBinary;
		GTSL::Array<RayTracingShaderInfo, 8> Shaders;
		uint8 RecursionDepth;

		template<class ALLOC>
		friend void Insert(const MaterialResourceManager::RayTracePipelineInfo& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(materialInfo.OffsetToBinary, buffer);
			Insert(materialInfo.Shaders, buffer);
			Insert(materialInfo.RecursionDepth, buffer);
		}

		template<class ALLOC>
		friend void Extract(MaterialResourceManager::RayTracePipelineInfo& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(materialInfo.OffsetToBinary, buffer);
			Extract(materialInfo.Shaders, buffer);
			Extract(materialInfo.RecursionDepth, buffer);
		}
	};
	
	struct OnMaterialLoadInfo : RasterMaterialData, OnResourceLoad {};
	
	struct MaterialLoadInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(TaskInfo, OnMaterialLoadInfo)> OnMaterialLoad;
	};
	OnMaterialLoadInfo LoadMaterial(const MaterialLoadInfo& loadInfo);

	struct ShaderInfo
	{
		Id Name; uint32 Size;

	private:
		uint32 offset;
		
		friend class MaterialResourceManager;
	};

	template<typename... ARGS>
	void LoadShaderInfos(GameInstance* gameInstance, GTSL::Range<const Id*> shaderNames, DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<ShaderInfo, 8>, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{		
		auto loadShaderInfos = [](TaskInfo taskInfo, MaterialResourceManager* materialResourceManager, GTSL::Array<Id, 8> shaderNames, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			GTSL::Array<ShaderInfo, 8> shaderInfos;

			for (auto e : shaderNames)
			{				
				ShaderInfo shaderInfo;
				shaderInfo.Name = e;
				shaderInfo.Size = 0;
				shaderInfos.EmplaceBack(shaderInfo);
			}

			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(materialResourceManager), GTSL::MoveRef(shaderInfos), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask("loadShaderInfosFromDisk", Task<MaterialResourceManager*, GTSL::Array<Id, 8>, decltype(dynamicTaskHandle), ARGS...>::Create(loadShaderInfos), GTSL::Range<TaskDependency*>(), this, GTSL::Array<Id, 8>(shaderNames), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadShaders(GameInstance* gameInstance, GTSL::Range<const ShaderInfo*> shaderInfos, DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<ShaderInfo, 8>, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, GTSL::Range<byte*> buffer, ARGS&&... args)
	{
		auto loadShaders = [](TaskInfo taskInfo, MaterialResourceManager* materialResourceManager, GTSL::Array<ShaderInfo, 8> shaderInfos, GTSL::Range<byte*> buffer, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			uint32 offset = 0;

			for (auto e : shaderInfos)
			{
				materialResourceManager->package.SetPointer(e.offset);

				BE_ASSERT(e.Size != 0, "0 bytes!");
				[[maybe_unused]] const auto read = materialResourceManager->package.Read(e.Size, offset, buffer);
				BE_ASSERT(read != 0, "Read 0 bytes!");

				offset += e.Size;
			}

			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(materialResourceManager), GTSL::MoveRef(shaderInfos), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask("loadShadersFromDisk", Task<MaterialResourceManager*, GTSL::Array<ShaderInfo, 8>, GTSL::Range<byte*>, decltype(dynamicTaskHandle), ARGS...>::Create(loadShaders), GTSL::Range<TaskDependency*>(), this, GTSL::Array<ShaderInfo, 8>(shaderInfos), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	RayTracePipelineInfo GetRayTracePipelineInfo() { return rtPipelineInfos.At("ScenePipeline"); }
	void LoadRayTraceShadersForPipeline(const RayTracePipelineInfo& info, GTSL::Range<byte*> buffer);

private:
	GTSL::File package, index;
	GTSL::FlatHashMap<Id, RasterMaterialDataSerialize, BE::PersistentAllocatorReference> rasterMaterialInfos;
	GTSL::FlatHashMap<Id, RayTracePipelineInfo, BE::PersistentAllocatorReference> rtPipelineInfos;
	mutable GTSL::ReadWriteMutex mutex;
};
