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

	struct Uniform;
	struct Binding;

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
	
	struct MaterialInfo
	{
		uint32 MaterialOffset = 0;
		GTSL::Id64 RenderGroup;
		GTSL::Id64 SubPass;
		GTSL::Array<uint32, 12> ShaderSizes;
		GTSL::Array<uint8, 20> VertexElements;
		bool DepthWrite; bool DepthTest; bool StencilTest;
		GAL::CullMode CullMode;
		GTSL::Id64 RenderPass;

		struct Binding
		{
			uint8 Type = 0;
			GAL::ShaderStage::value_type Stage;

			Binding() = default;
			Binding(const MaterialResourceManager::Binding& binding) : Type(static_cast<GTSL::UnderlyingType<GAL::BindingType>>(binding.Type)), Stage(binding.Stage) {}

			friend void Insert(const Binding& materialInfo, GTSL::Buffer& buffer);
			friend void Extract(Binding& materialInfo, GTSL::Buffer& buffer);
		};

		struct Uniform
		{
			GTSL::Id64 Name;
			uint8 Type = 0;

			Uniform() = default;
			Uniform(const MaterialResourceManager::Uniform& uniform) : Name(uniform.Name), Type(static_cast<GTSL::UnderlyingType<GAL::ShaderDataType>>(uniform.Type)) {}

			friend void Insert(const Uniform& materialInfo, GTSL::Buffer& buffer);
			friend void Extract(Uniform& materialInfo, GTSL::Buffer& buffer);
		};
		
		GTSL::Array<GTSL::Array<Binding, 6>, 6> BindingSets;
		GTSL::Array<GTSL::Array<Uniform, 6>, 6> Uniforms;
		GTSL::Array<uint8, 12> ShaderTypes;
		GAL::BlendOperation ColorBlendOperation;

		StencilState Front;
		StencilState Back;
		
		friend void Insert(const MaterialInfo& materialInfo, GTSL::Buffer& buffer);
		friend void Extract(MaterialInfo& materialInfo, GTSL::Buffer& buffer);
	};

	struct Binding
	{
		GAL::BindingType Type;
		GAL::ShaderStage::value_type Stage;

		Binding() = default;
		Binding(const GAL::BindingType type, const GAL::ShaderStage::value_type pipelineStage) : Type(type), Stage(pipelineStage) {}
		Binding(const MaterialInfo::Binding& other) : Type(static_cast<GAL::BindingType>(other.Type)), Stage(other.Stage) {}
	};

	struct Uniform
	{
		GTSL::Id64 Name;
		GAL::ShaderDataType Type;

		Uniform() = default;
		Uniform(const GTSL::Id64 name, const GAL::ShaderDataType type) : Name(name), Type(type) {}
		Uniform(const MaterialInfo::Uniform& other) : Name(other.Name), Type(static_cast<GAL::ShaderDataType>(other.Type)) {}
	};
	
	struct MaterialCreateInfo
	{
		GTSL::StaticString<128> ShaderName;
		GTSL::StaticString<128> RenderGroup;
		GTSL::Id64 RenderPass;
		GTSL::Id64 SubPass;
		GTSL::Ranger<const GAL::ShaderDataType> VertexFormat;
		
		GTSL::Ranger<const GTSL::Ranger<const Binding>> Bindings;
		GTSL::Ranger<const GTSL::Ranger<const Uniform>> Uniforms;
		GTSL::Ranger<const GAL::ShaderType> ShaderTypes;
		bool DepthWrite;
		bool DepthTest;
		GAL::CullMode CullMode;
		GAL::BlendOperation ColorBlendOperation;

		StencilState Front;
		StencilState Back;
		bool StencilTest;
	};
	void CreateMaterial(const MaterialCreateInfo& materialCreateInfo);

	void GetMaterialSize(GTSL::Id64 name, uint32& size);
	
	struct OnMaterialLoadInfo : OnResourceLoad
	{
		GTSL::Id64 RenderGroup;
		GTSL::Array<GAL::ShaderDataType, 20> VertexElements;
		GTSL::Array<GTSL::Array<Binding, 12>, 12> BindingSets;
		GTSL::Array<GTSL::Array<Uniform, 12>, 12> Uniforms;
		GTSL::Array<GAL::ShaderType, 12> ShaderTypes;
		GTSL::Array<uint32, 20> ShaderSizes;
		bool DepthWrite;
		bool DepthTest;
		GAL::CullMode CullMode;
		GAL::BlendOperation ColorBlendOperation;

		StencilState Front;
		StencilState Back;
		GTSL::Id64 RenderPass;
		GTSL::Id64 SubPass;
		bool StencilTest;
	};
	
	struct MaterialLoadInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(TaskInfo, OnMaterialLoadInfo)> OnMaterialLoad;
	};
	void LoadMaterial(const MaterialLoadInfo& loadInfo);
	
private:
	GTSL::File package, index;
	GTSL::FlatHashMap<MaterialInfo, BE::PersistentAllocatorReference> materialInfos;
	GTSL::ReadWriteMutex mutex;
};
