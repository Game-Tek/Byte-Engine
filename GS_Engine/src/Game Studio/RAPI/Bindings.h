#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Containers/Array.hpp"

namespace RAPI
{

#define MAX_BINDINGS_PER_SET 8

	class RenderContext;

	struct BindingDescriptor
	{
		/**
		 * \brief If binding is an array how many elements does it have.
		 */
		uint8 ArrayLength = 0;
		UniformType BindingType = UniformType::UNIFORM_BUFFER;
		ShaderType ShaderStage = ShaderType::ALL_STAGES;
		void* BindingResource = nullptr;
	};

	struct BindingLayoutCreateInfo
	{
		Array<BindingDescriptor, MAX_BINDINGS_PER_SET> LayoutBindings;
		int DescriptorCount = 0;
	};

	struct BindingSetUpdateInfo : RenderInfo
	{
		Array<BindingDescriptor, MAX_BINDINGS_PER_SET> LayoutBindings;
		uint8 DestinationSet = 0;
	};


	struct BindingsPoolCreateInfo : RenderInfo
	{
		Array<BindingDescriptor, MAX_BINDINGS_PER_SET> LayoutBindings;
		/**
		 * \brief How many sets to allocate.
		 */
		uint8 BindingsSetCount = 0;
	};

	class BindingsPool
	{
	public:
		virtual ~BindingsPool() = default;

		struct FreeBindingsPoolInfo : RenderInfo
		{
		};

		virtual void FreePool(const FreeBindingsPoolInfo& freeDescriptorPoolInfo) = 0;
		virtual void FreeBindingsSet() = 0;
	};

	struct BindingsSetCreateInfo : RenderInfo
	{
		/**
		 * \brief Pointer to a binding pool to allocated the bindings set from.
		 */
		BindingsPool* BindingsPool = nullptr;
		Array<BindingDescriptor, MAX_BINDINGS_PER_SET> LayoutBindings;
		/**
		 * \brief How many sets to allocate.
		 */
		uint8 BindingsSetCount = 0;
	};

	class BindingsSet
	{
	public:
		virtual ~BindingsSet() = default;

		virtual void Update(const BindingSetUpdateInfo& uniformLayoutUpdateInfo) = 0;
	};

}