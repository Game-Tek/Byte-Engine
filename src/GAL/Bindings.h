#pragma once

#include "RenderCore.h"

namespace GAL
{
	class Sampler;
	constexpr GTSL::uint8 MAX_BINDINGS_PER_SET = 10;

	class RenderContext;
	
	//struct BindingLayoutCreateInfo
	//{
	//	GTSL::Range<BindingDescriptor> BindingsSetLayout;
	//	GTSL::uint32 DescriptorCount = 0;
	//};

	class BindingsPool {
	public:
		~BindingsPool() = default;

		struct BindingsPoolSize {
			BindingType Type;
			GTSL::uint32 Count = 0;
		};
	};

	struct BindingSetLayout {
		struct BindingDescriptor {
			BindingType Type;
			ShaderStage Stage;
			GTSL::uint32 BindingsCount;
			BindingFlag Flags;
			GTSL::Range<const Sampler*> Samplers;
		};		
	};

	class BindingsSet {
	public:
	};
}