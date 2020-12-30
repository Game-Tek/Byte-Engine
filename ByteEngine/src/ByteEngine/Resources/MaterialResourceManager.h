#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Algorithm.h>
#include <GTSL/Array.hpp>
#include <GTSL/Delegate.hpp>
#include <GTSL/File.h>
#include <GTSL/FlatHashMap.h>
#include "ResourceManager.h"

class MaterialResourceManager final : public ResourceManager
{
public:
	MaterialResourceManager();
	~MaterialResourceManager();

	struct Binding
	{
		GAL::BindingType Type;
		GAL::ShaderStage::value_type Stage;

		Binding() = default;
		Binding(const GAL::BindingType type, const GAL::ShaderStage::value_type pipelineStage) : Type(type), Stage(pipelineStage) {}
		//Binding(const RasterMaterialInfo::Binding& other) : Type(static_cast<GAL::BindingType>(other.Type)), Stage(other.Stage) {}

		friend void Insert(const Binding& materialInfo, GTSL::Buffer& buffer);
		friend void Extract(Binding& materialInfo, GTSL::Buffer& buffer);
	};

	struct Uniform
	{
		GTSL::Id64 Name;
		GAL::ShaderDataType Type;

		Uniform() = default;
		Uniform(const GTSL::Id64 name, const GAL::ShaderDataType type) : Name(name), Type(type) {}
		//Uniform(const RasterMaterialInfo::Uniform& other) : Name(other.Name), Type(static_cast<GAL::ShaderDataType>(other.Type)) {}

		friend void Insert(const Uniform& materialInfo, GTSL::Buffer& buffer);
		friend void Extract(Uniform& materialInfo, GTSL::Buffer& buffer);
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

		friend void Insert(const StencilState& materialInfo, GTSL::Buffer& buffer);
		friend void Extract(StencilState& materialInfo, GTSL::Buffer& buffer);
	};
	
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
		
		friend void Insert(const RasterMaterialInfo& materialInfo, GTSL::Buffer& buffer);
		friend void Extract(RasterMaterialInfo& materialInfo, GTSL::Buffer& buffer);
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
	};

	struct RayTraceMaterialInfo
	{
		uint32 OffsetToBinary;
		RayTracingShaderInfo ShaderInfo;
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
