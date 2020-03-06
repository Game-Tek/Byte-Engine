#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Containers/Array.hpp"

namespace RAPI
{

	constexpr uint8 MAX_BINDINGS_PER_SET = 10;

	class RenderContext;

	struct BindingDescriptor
	{
		/**
		 * \brief If binding is an array how many elements does it have.
		 */
		uint8 ArrayLength = 0;
		BindingType BindingType = BindingType::UNIFORM_BUFFER;
		ShaderType ShaderStage = ShaderType::ALL_STAGES;
		void* BindingResource = nullptr;
	};

	struct BindingLayoutCreateInfo
	{
		Array<BindingDescriptor, MAX_BINDINGS_PER_SET> BindingsSetLayout;
		int DescriptorCount = 0;
	};

	struct BindingsSetUpdateInfo : RenderInfo
	{
		Array<BindingDescriptor, MAX_BINDINGS_PER_SET> BindingsSetLayout;
		uint8 DestinationSet = 0;
	};


	struct BindingsPoolCreateInfo : RenderInfo
	{
		Array<BindingDescriptor, MAX_BINDINGS_PER_SET> BindingsSetLayout;
		/**
		 * \brief How many sets to allocate.
		 */
		uint8 BindingsSetCount = 0;
	};

	class BindingsPool : public RAPIObject
	{
	public:
		virtual ~BindingsPool() = default;

		struct FreeBindingsPoolInfo : RenderInfo
		{
		};

		virtual void FreePool(const FreeBindingsPoolInfo& freeDescriptorPoolInfo) = 0;
		struct FreeBindingsSetInfo : RenderInfo
		{
			class BindingsSet* BindingsSet = nullptr;
		};
		virtual void FreeBindingsSet(const FreeBindingsSetInfo& freeBindingsSetInfo) = 0;
	};

	struct BindingsSetCreateInfo : RenderInfo
	{
		/**
		 * \brief Pointer to a binding pool to allocated the bindings set from.
		 */
		BindingsPool* BindingsPool = nullptr;
		Array<BindingDescriptor, MAX_BINDINGS_PER_SET> BindingsSetLayout;
		/**
		 * \brief How many sets to allocate.
		 */
		uint8 BindingsSetCount = 0;
	};

	class BindingsSet : public RAPIObject
	{
	public:
		virtual ~BindingsSet() = default;

		virtual void Update(const BindingsSetUpdateInfo& uniformLayoutUpdateInfo) = 0;
	};

}