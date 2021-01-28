#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Algorithm.h>
#include <GTSL/Array.hpp>
#include <GTSL/Delegate.hpp>
#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>
#include "ResourceManager.h"

#include "ByteEngine/Game/GameInstance.h"

class MaterialResourceManager final : public ResourceManager
{
public:
	MaterialResourceManager();
	~MaterialResourceManager();
	void GetShaderSize(Id id, uint32* shaderSize);

	struct Binding
	{
		GAL::BindingType Type;
		GAL::ShaderStage::value_type Stage;

		Binding() = default;
		Binding(const GAL::BindingType type, const GAL::ShaderStage::value_type pipelineStage) : Type(type), Stage(pipelineStage) {}
		//Binding(const RasterMaterialInfo::Binding& other) : Type(static_cast<GAL::BindingType>(other.Type)), Stage(other.Stage) {}

		template<class ALLOC>
		friend void Insert(const Binding& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(materialInfo.Type, buffer);
			Insert(materialInfo.Stage, buffer);
		}
		
		template<class ALLOC>
		friend void Extract(Binding& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(materialInfo.Type, buffer);
			Extract(materialInfo.Stage, buffer);
		}
	};

	struct Uniform
	{
		GTSL::Id64 Name;
		GAL::ShaderDataType Type;

		Uniform() = default;
		Uniform(const GTSL::Id64 name, const GAL::ShaderDataType type) : Name(name), Type(type) {}
		//Uniform(const RasterMaterialInfo::Uniform& other) : Name(other.Name), Type(static_cast<GAL::ShaderDataType>(other.Type)) {}

		template<class ALLOC>
		friend void Insert(const Uniform& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(materialInfo.Name, buffer);
			Insert(materialInfo.Type, buffer);
		}
		
		template<class ALLOC>
		friend void Extract(Uniform& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(materialInfo.Name, buffer);
			Extract(materialInfo.Type, buffer);
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

#define MAKE_SERIALIZE_FUNCTIONS(paramType, ...)\
	template<class ALLOCATOR>\
	friend void Insert(const paramType& param, GTSL::Buffer<ALLOCATOR>& buffer) {\
		Insert(param.__VA_ARGS__, buffer);\
	}\
	
	
	struct RasterMaterialInfo
	{
		uint32 MaterialOffset = 0;
		GTSL::Id64 RenderGroup;
		GTSL::Array<uint32, 12> ShaderSizes;
		GTSL::Array<uint8, 20> VertexElements;
		bool DepthWrite; bool DepthTest; bool StencilTest;
		GAL::CullMode CullMode;
		GTSL::Id64 RenderPass;

		GTSL::Array<Uniform, 8> MaterialParameters;
		GTSL::Array<GTSL::Id64, 8> Textures;
		GTSL::Array<Binding, 8> PerInstanceParameters;
		
		GTSL::Array<uint8, 12> ShaderTypes;
		GAL::BlendOperation ColorBlendOperation;

		StencilState Front;
		StencilState Back;
		bool BlendEnable = false;

		//MAKE_SERIALIZE_FUNCTIONS(RasterMaterialInfo, MaterialOffset)
		
		template<class ALLOC>
		friend void Insert(const RasterMaterialInfo& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(materialInfo.MaterialOffset, buffer);
			Insert(materialInfo.RenderGroup, buffer);
			Insert(materialInfo.RenderPass, buffer);

			Insert(materialInfo.ShaderSizes, buffer);
			Insert(materialInfo.VertexElements, buffer);
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
		
		template<class ALLOC>
		friend void Extract(RasterMaterialInfo& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(materialInfo.MaterialOffset, buffer);
			Extract(materialInfo.RenderGroup, buffer);
			Extract(materialInfo.RenderPass, buffer);

			Extract(materialInfo.ShaderSizes, buffer);
			Extract(materialInfo.VertexElements, buffer);
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
	};
	
	struct RasterMaterialCreateInfo
	{
		GTSL::StaticString<64> ShaderName;
		GTSL::StaticString<64> RenderGroup;
		GTSL::Id64 RenderPass;
		GTSL::Range<const GAL::ShaderDataType*> VertexFormat;

		GTSL::Array<Uniform, 8> MaterialParameters;
		GTSL::Array<Binding, 8> PerInstanceParameters;

		GTSL::Array<GTSL::Id64, 8> Textures;
		
		GTSL::Range<const Binding*> Bindings;
		GTSL::Range<const GAL::ShaderType*> ShaderTypes;
		bool DepthWrite;
		bool DepthTest;
		GAL::CullMode CullMode;
		GAL::BlendOperation ColorBlendOperation;

		StencilState Front;
		StencilState Back;
		bool StencilTest;
		bool BlendEnable = false;
	};
	void CreateRasterMaterial(const RasterMaterialCreateInfo& materialCreateInfo);

	struct RayTraceMaterialCreateInfo
	{
		GTSL::StaticString<64> ShaderName;
		GAL::ShaderType Type;
		GAL::BlendOperation ColorBlendOperation;
	};
	void CreateRayTraceMaterial(const RayTraceMaterialCreateInfo& materialCreateInfo);

	void GetMaterialSize(GTSL::Id64 name, uint32& size);

	struct RayTracingShaderInfo
	{
		/**
		 * \brief Size of the precompiled binary blob to be provided to the API.
		 */
		uint32 BinarySize;

		GAL::ShaderType ShaderType;
		GAL::BlendOperation ColorBlendOperation;

		template<class ALLOC>
		friend void Insert(const MaterialResourceManager::RayTracingShaderInfo& shaderInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(shaderInfo.BinarySize, buffer);
			Insert(shaderInfo.ShaderType, buffer);
			Insert(shaderInfo.ColorBlendOperation, buffer);
		}

		template<class ALLOC>
		friend void Extract(MaterialResourceManager::RayTracingShaderInfo& shaderInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(shaderInfo.BinarySize, buffer);
			Extract(shaderInfo.ShaderType, buffer);
			Extract(shaderInfo.ColorBlendOperation, buffer);
		}
	};

	struct RayTraceMaterialInfo
	{
		uint32 OffsetToBinary;
		RayTracingShaderInfo ShaderInfo;

		template<class ALLOC>
		friend void Insert(const MaterialResourceManager::RayTraceMaterialInfo& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Insert(materialInfo.OffsetToBinary, buffer);
			Insert(materialInfo.ShaderInfo, buffer);
		}

		template<class ALLOC>
		friend void Extract(MaterialResourceManager::RayTraceMaterialInfo& materialInfo, GTSL::Buffer<ALLOC>& buffer)
		{
			Extract(materialInfo.OffsetToBinary, buffer);
			Extract(materialInfo.ShaderInfo, buffer);
		}
	};
	
	struct OnMaterialLoadInfo : OnResourceLoad
	{
		GTSL::Id64 RenderGroup;
		GTSL::Array<GAL::ShaderDataType, 20> VertexElements;
		GTSL::Array<Uniform, 8> MaterialParameters;
		GTSL::Array<Binding, 8> PerInstanceParameters;

		GTSL::Array<Uniform, 6> Uniforms;
		GTSL::Array<GTSL::Id64, 8> Textures;
		GTSL::Array<GAL::ShaderType, 12> ShaderTypes;
		GTSL::Array<uint32, 20> ShaderSizes;
		bool DepthWrite;
		bool DepthTest;
		GAL::CullMode CullMode;
		GAL::BlendOperation ColorBlendOperation;

		StencilState Front;
		StencilState Back;
		GTSL::Id64 RenderPass;
		bool StencilTest;
		bool BlendEnable = false;
	};
	
	struct MaterialLoadInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(TaskInfo, OnMaterialLoadInfo)> OnMaterialLoad;
	};
	void LoadMaterial(const MaterialLoadInfo& loadInfo);

	struct ShaderInfo
	{
		Id Name; uint32 Size;

	private:
		uint32 offset;
		
		friend class MaterialResourceManager;
	};
	
	struct Shader
	{
		Id Name; uint32 Size;
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
	void LoadShaders(GameInstance* gameInstance, GTSL::Range<const ShaderInfo*> shaderInfos, DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<Shader, 8>, GTSL::Range<byte*>, ARGS...> dynamicTaskHandle, GTSL::Range<byte*> buffer, ARGS&&... args)
	{
		auto loadShaders = [](TaskInfo taskInfo, MaterialResourceManager* materialResourceManager, GTSL::Array<ShaderInfo, 8> shaderInfos, GTSL::Range<byte*> buffer, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			GTSL::Array<Shader, 8> shaders;

			uint32 offset = 0;

			for (auto e : shaderInfos)
			{
				materialResourceManager->package.SetPointer(e.offset, GTSL::File::MoveFrom::BEGIN);

				BE_ASSERT(e.Size != 0, "0 bytes!");
				[[maybe_unused]] const auto read = materialResourceManager->package.ReadFile(e.Size, offset, buffer);
				BE_ASSERT(read != 0, "Read 0 bytes!");

				offset += e.Size;
			}

			taskInfo.GameInstance->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(materialResourceManager), GTSL::MoveRef(shaders), GTSL::MoveRef(buffer), GTSL::ForwardRef<ARGS>(args)...);
		};
		
		gameInstance->AddDynamicTask("loadShadersFromDisk", Task<MaterialResourceManager*, GTSL::Array<ShaderInfo, 8>, GTSL::Range<byte*>, decltype(dynamicTaskHandle), ARGS...>::Create(loadShaders), GTSL::Range<TaskDependency*>(), this, GTSL::Array<ShaderInfo, 8>(shaderInfos), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
	OnMaterialLoadInfo LoadMaterialSynchronous(uint64 id, GTSL::Range<byte*> buffer);
	
	RayTracingShaderInfo LoadRayTraceShaderSynchronous(Id id, GTSL::Range<byte*> buffer);

	uint32 GetRayTraceShaderSize(Id handle) const
	{
		GTSL::ReadLock lock(mutex);
		return rtMaterialInfos.At(handle()).ShaderInfo.BinarySize;
	}
	
	uint32 GetRayTraceShaderCount() const { return rtHandles.GetLength(); }
	Id GetRayTraceShaderHandle(const uint32 handle) const { return rtHandles[handle]; }

private:
	
	GTSL::File package, index;
	GTSL::FlatHashMap<RasterMaterialInfo, BE::PersistentAllocatorReference> rasterMaterialInfos;
	GTSL::FlatHashMap<RayTraceMaterialInfo, BE::PersistentAllocatorReference> rtMaterialInfos;
	mutable GTSL::ReadWriteMutex mutex;

	GTSL::Vector<Id, BE::PAR> rtHandles;
};
