#pragma once

#include "RenderCore.h"

namespace GAL
{
	constexpr GTSL::uint8 MAX_BINDINGS_PER_SET = 10;

	class RenderContext;
	
	//struct BindingLayoutCreateInfo
	//{
	//	GTSL::Range<BindingDescriptor> BindingsSetLayout;
	//	GTSL::uint32 DescriptorCount = 0;
	//};

	class BindingsPool
	{
	public:
		//struct CreateInfo : RenderInfo
		//{
		//	GTSL::Range<BindingDescriptor> BindingsDescriptors;
		//	GTSL::Range<class BindingsSet> BindingsSets;
		//};
		
		~BindingsPool() = default;

		struct BindingsPoolSize {
			BindingType BindingType;
			GTSL::uint32 Count = 0;
		};
		
		//struct BindingDescriptor
		//{
		//	BindingType BindingType = BindingType::UNIFORM_BUFFER;
		//	ShaderType ShaderStage = ShaderType::VERTEX_SHADER;
		//	GTSL::uint8 MaxNumberOfBindingsAllocatable{ 0 };
		//};
		//
		//struct ImageBindingDescriptor : BindingDescriptor
		//{
		//	GTSL::Range<const class ImageView> ImageViews;
		//	GTSL::Range<const class Sampler> Samplers;
		//	GTSL::Range<ImageLayout> Layouts;
		//};
		//
		//struct BufferBindingDescriptor : BindingDescriptor
		//{
		//	GTSL::Range<const class Buffer> Buffers;
		//	GTSL::Range<GTSL::uint32> Offsets;
		//	GTSL::Range<GTSL::uint32> Sizes;
		//};
		
		//struct FreeBindingsSetInfo : RenderInfo
		//{
		//	GTSL::Range<class BindingsSet> BindingsSet;
		//};
	};

	class BindingsSet
	{
	public:

		//struct BindingsSetUpdateInfo : RenderInfo
		//{
		//	GTSL::Array<ImageBindingDescriptor, MAX_BINDINGS_PER_SET> ImageBindingsSetLayout;
		//	GTSL::Array<BufferBindingDescriptor, MAX_BINDINGS_PER_SET> BufferBindingsSetLayout;
		//};
		//void Update(const BindingsSetUpdateInfo& uniformLayoutUpdateInfo);
	};

}
