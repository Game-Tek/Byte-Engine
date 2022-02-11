#pragma once

#include "GAL/Bindings.h"

#include "Vulkan.h"
#include "VulkanAccelerationStructures.h"
#include "VulkanBuffer.h"
#include "VulkanRenderDevice.h"
#include "VulkanTexture.h"
#include "GTSL/Vector.hpp"

namespace GAL
{
	class VulkanBindingsSet;

	class VulkanBindingsPool final : public BindingsPool
	{
	public:
		VulkanBindingsPool() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, GTSL::Range<const BindingsPoolSize*> bindingsPoolSizes, GTSL::uint32 maxSets) {
			GTSL::StaticVector<VkDescriptorPoolSize, MAX_BINDINGS_PER_SET> vkDescriptorPoolSizes;

			for (GTSL::uint32 i = 0; i < static_cast<GTSL::uint32>(bindingsPoolSizes.ElementCount()); ++i) {
				auto& descriptorPoolSize = vkDescriptorPoolSizes.EmplaceBack();
				descriptorPoolSize.type = ToVulkan(bindingsPoolSizes[i].BindingType);
				//Max number of descriptors of VkDescriptorPoolSize::type we can allocate.
				descriptorPoolSize.descriptorCount = bindingsPoolSizes[&descriptorPoolSize - vkDescriptorPoolSizes.begin()].Count;
			}

			VkDescriptorPoolCreateInfo vkDescriptorPoolCreateInfo{ VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO };
			//Is the total number of sets that can be allocated from the pool.
			vkDescriptorPoolCreateInfo.maxSets = maxSets;
			vkDescriptorPoolCreateInfo.poolSizeCount = vkDescriptorPoolSizes.GetLength();
			vkDescriptorPoolCreateInfo.pPoolSizes = vkDescriptorPoolSizes.begin();
			renderDevice->VkCreateDescriptorPool(renderDevice->GetVkDevice(), &vkDescriptorPoolCreateInfo, renderDevice->GetVkAllocationCallbacks(), &descriptorPool);
			//setName(renderDevice, descriptorPool, VK_OBJECT_TYPE_DESCRIPTOR_POOL, createInfo.Name);
		}

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyDescriptorPool(renderDevice->GetVkDevice(), descriptorPool, renderDevice->GetVkAllocationCallbacks());
			debugClear(descriptorPool);
		}

		//void FreeBindingsSet(const VulkanRenderDevice* renderDevice) {
		//	vkFreeDescriptorSets(renderDevice->GetVkDevice(), descriptorPool,
		//		static_cast<GTSL::uint32>(freeBindingsSetInfo.BindingsSet.ElementCount()), reinterpret_cast<VkDescriptorSet*>(freeBindingsSetInfo.BindingsSet.begin()));
		//}

		[[nodiscard]] VkDescriptorPool GetVkDescriptorPool() const { return descriptorPool; }
		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(descriptorPool); }

		struct TextureBindingUpdateInfo {
			VulkanSampler Sampler;
			VulkanTextureView TextureView;
			TextureLayout TextureLayout;
			FormatDescriptor FormatDescriptor;
		};

		struct BufferBindingUpdateInfo {
			VulkanBuffer Buffer;
			GTSL::uint64 Offset, Range;
		};

		struct AccelerationStructureBindingUpdateInfo {
			VulkanAccelerationStructure AccelerationStructure;
		};

		union BindingUpdateInfo
		{
			BindingUpdateInfo(TextureBindingUpdateInfo info) : TextureBindingUpdateInfo(info) {}
			BindingUpdateInfo(BufferBindingUpdateInfo info) : BufferBindingUpdateInfo(info) {}
			BindingUpdateInfo(AccelerationStructureBindingUpdateInfo info) : AccelerationStructureBindingUpdateInfo(info) {}

			TextureBindingUpdateInfo TextureBindingUpdateInfo;
			BufferBindingUpdateInfo BufferBindingUpdateInfo;
			AccelerationStructureBindingUpdateInfo AccelerationStructureBindingUpdateInfo;
		};

		struct BindingsUpdateInfo
		{
			VulkanBindingsSet* BindingsSet;
			BindingType Type;
			GTSL::uint32 SubsetIndex = 0, BindingIndex = 0;
			GTSL::Range<const BindingUpdateInfo*> BindingUpdateInfos;
		};

		template<class ALLOCATOR>
		void Update(const VulkanRenderDevice* renderDevice, GTSL::Range<const BindingsUpdateInfo*> bindingsUpdateInfos, const ALLOCATOR& allocator);

	private:
		VkDescriptorPool descriptorPool;
	};

	class VulkanBindingsSetLayout final : public BindingSetLayout {
	public:
		struct ImageBindingDescriptor : BindingDescriptor {
			GTSL::Range<const VulkanTextureView*> ImageViews;
			GTSL::Range<const TextureLayout*> Layouts;
		};

		struct BufferBindingDescriptor : BindingDescriptor {
			GTSL::Range<const VulkanBuffer*> Buffers;
			GTSL::Range<const GTSL::uint32*> Offsets;
			GTSL::Range<const GTSL::uint32*> Sizes;
		};

		struct BindingDescriptor {
			BindingType BindingType;
			ShaderStage ShaderStage;
			GTSL::uint32 BindingsCount;
			BindingFlag Flags;
			GTSL::Range<const VulkanSampler*> Samplers;
		};

		VulkanBindingsSetLayout() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, GTSL::Range<const BindingDescriptor*> bindingsDescriptors) {

			VkDescriptorSetLayoutCreateInfo vkDescriptorSetLayoutCreateInfo{ VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO };
			VkDescriptorSetLayoutBindingFlagsCreateInfo vkDescriptorSetLayoutBindingFlagsCreateInfo{ VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_BINDING_FLAGS_CREATE_INFO };
			vkDescriptorSetLayoutCreateInfo.pNext = &vkDescriptorSetLayoutBindingFlagsCreateInfo;

			GTSL::StaticVector<VkDescriptorBindingFlags, 16> vkDescriptorBindingFlags;
			GTSL::StaticVector<VkDescriptorSetLayoutBinding, MAX_BINDINGS_PER_SET> vkDescriptorSetLayoutBindings;

			GTSL::StaticVector<VkSampler, 16> staticSamplers;

			for (GTSL::uint32 i = 0; i < static_cast<GTSL::uint32>(bindingsDescriptors.ElementCount()); ++i) {
				vkDescriptorBindingFlags.EmplaceBack(ToVulkan(bindingsDescriptors[i].Flags));

				auto& binding = vkDescriptorSetLayoutBindings.EmplaceBack();
				binding.binding = i;
				binding.descriptorCount = bindingsDescriptors[i].BindingsCount;
				binding.descriptorType = ToVulkan(bindingsDescriptors[i].BindingType);
				binding.stageFlags = ToVulkan(bindingsDescriptors[i].ShaderStage);

				if(bindingsDescriptors[i].Samplers.ElementCount()) {
					for(auto& e : bindingsDescriptors[i].Samplers) {
						staticSamplers.EmplaceBack(static_cast<const VulkanSampler&>(e).GetVkSampler());
					}

					binding.pImmutableSamplers = staticSamplers.GetData();
				} else {
					binding.pImmutableSamplers = nullptr;
				}
			}

			vkDescriptorSetLayoutBindingFlagsCreateInfo.bindingCount = vkDescriptorSetLayoutBindings.GetLength();
			vkDescriptorSetLayoutBindingFlagsCreateInfo.pBindingFlags = vkDescriptorBindingFlags.begin();
			vkDescriptorSetLayoutCreateInfo.bindingCount = vkDescriptorSetLayoutBindings.GetLength();
			vkDescriptorSetLayoutCreateInfo.pBindings = vkDescriptorSetLayoutBindings.begin();

			renderDevice->VkCreateDescriptorSetLayout(renderDevice->GetVkDevice(), &vkDescriptorSetLayoutCreateInfo, renderDevice->GetVkAllocationCallbacks(), &descriptorSetLayout);
			//setName(createInfo.RenderDevice, &descriptorSetLayout, VK_OBJECT_TYPE_DESCRIPTOR_SET_LAYOUT, createInfo.Name);
		}
		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->VkDestroyDescriptorSetLayout(renderDevice->GetVkDevice(), descriptorSetLayout, renderDevice->GetVkAllocationCallbacks());
			debugClear(descriptorSetLayout);
		}

		[[nodiscard]] VkDescriptorSetLayout GetVkDescriptorSetLayout() const { return descriptorSetLayout; }
		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(descriptorSetLayout); }

	private:
		VkDescriptorSetLayout descriptorSetLayout = nullptr;
	};

	class VulkanBindingsSet final : public BindingsSet
	{
	public:
		VulkanBindingsSet() = default;

		void Initialize(const VulkanRenderDevice* renderDevice, VulkanBindingsPool bindingsPool, const VulkanBindingsSetLayout bindingsSetLayouts) {
			auto vkDescriptorSetLayout = bindingsSetLayouts.GetVkDescriptorSetLayout();
			
			VkDescriptorSetAllocateInfo vkDescriptorSetAllocateInfo{ VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO };
			//vkDescriptorSetAllocateInfo.pNext = &vkDescriptorSetVariableDescriptorCountAllocateInfo;
			vkDescriptorSetAllocateInfo.descriptorPool = bindingsPool.GetVkDescriptorPool();
			vkDescriptorSetAllocateInfo.descriptorSetCount = 1;
			vkDescriptorSetAllocateInfo.pSetLayouts = &vkDescriptorSetLayout;
			renderDevice->VkAllocateDescriptorSets(renderDevice->GetVkDevice(), &vkDescriptorSetAllocateInfo, &descriptorSet);
		}

		[[nodiscard]] VkDescriptorSet GetVkDescriptorSet() const { return descriptorSet; }
		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(descriptorSet); }

	private:
		VkDescriptorSet descriptorSet;
	};

	//TODO: fix up potential pointer errors

	template <class ALLOCATOR>
	void VulkanBindingsPool::Update(const VulkanRenderDevice* renderDevice, GTSL::Range<const BindingsUpdateInfo*> bindingsUpdateInfos, const ALLOCATOR& allocator) {
		GTSL::Vector<VkWriteDescriptorSet, ALLOCATOR> vkWriteDescriptorSets(static_cast<uint32_t>(bindingsUpdateInfos.ElementCount()), allocator);
		GTSL::Vector<VkWriteDescriptorSetAccelerationStructureKHR, ALLOCATOR> vkWriteDescriptorSetsAcc(2, allocator);

		GTSL::Vector<GTSL::Vector<VkAccelerationStructureKHR, ALLOCATOR>, ALLOCATOR> accelerationStructuresPerSubSetUpdate(8, allocator);
		GTSL::Vector<GTSL::Vector<VkDescriptorBufferInfo, ALLOCATOR>, ALLOCATOR> buffersPerSubSetUpdate(8, allocator);
		GTSL::Vector<GTSL::Vector<VkDescriptorImageInfo, ALLOCATOR>, ALLOCATOR> imagesPerSubSetUpdate(8, allocator);
		GTSL::Vector<GTSL::Vector<VulkanAccelerationStructure, ALLOCATOR>, ALLOCATOR> accPerSubSetUpdate(8, allocator);

		for (GTSL::uint32 index = 0; index < static_cast<GTSL::uint32>(bindingsUpdateInfos.ElementCount()); ++index) {
			VkWriteDescriptorSet& writeSet = vkWriteDescriptorSets.EmplaceBack();
			auto& info = bindingsUpdateInfos[index];

			writeSet.sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
			writeSet.pNext = nullptr;
			writeSet.dstSet = info.BindingsSet->GetVkDescriptorSet();
			writeSet.dstBinding = info.SubsetIndex;
			writeSet.dstArrayElement = info.BindingIndex;
			writeSet.descriptorCount = static_cast<GTSL::uint32>(info.BindingUpdateInfos.ElementCount());
			writeSet.descriptorType = ToVulkan(info.Type);
			writeSet.pImageInfo = nullptr;
			writeSet.pBufferInfo = nullptr;
			writeSet.pTexelBufferView = nullptr;

			switch (info.Type) {
			case BindingType::SAMPLER:
			case BindingType::COMBINED_IMAGE_SAMPLER:
			case BindingType::SAMPLED_IMAGE:
			case BindingType::STORAGE_IMAGE:
			case BindingType::INPUT_ATTACHMENT: {
				imagesPerSubSetUpdate.EmplaceBack(8, allocator);

				for (auto e : info.BindingUpdateInfos) {
					auto& vkDescriptorImageInfo = imagesPerSubSetUpdate.back().EmplaceBack();
					vkDescriptorImageInfo.sampler = e.TextureBindingUpdateInfo.Sampler.GetVkSampler();
					vkDescriptorImageInfo.imageView = e.TextureBindingUpdateInfo.TextureView.GetVkImageView();
					vkDescriptorImageInfo.imageLayout = ToVulkan(e.TextureBindingUpdateInfo.TextureLayout, e.TextureBindingUpdateInfo.FormatDescriptor);
				}

				writeSet.pImageInfo = imagesPerSubSetUpdate.back().begin();

				break;
			}
			case BindingType::UNIFORM_TEXEL_BUFFER: GAL_DEBUG_BREAK;
			case BindingType::STORAGE_TEXEL_BUFFER: GAL_DEBUG_BREAK;
			case BindingType::UNIFORM_BUFFER:
			case BindingType::STORAGE_BUFFER:
			case BindingType::UNIFORM_BUFFER_DYNAMIC:
			case BindingType::STORAGE_BUFFER_DYNAMIC: {
				buffersPerSubSetUpdate.EmplaceBack(8, allocator);

				for (auto e : info.BindingUpdateInfos) {
					auto& vkDescriptorBufferInfo = buffersPerSubSetUpdate.back().EmplaceBack();
					vkDescriptorBufferInfo.buffer = e.BufferBindingUpdateInfo.Buffer.GetVkBuffer();
					vkDescriptorBufferInfo.offset = e.BufferBindingUpdateInfo.Offset;
					vkDescriptorBufferInfo.range = e.BufferBindingUpdateInfo.Range;
				}

				writeSet.pBufferInfo = buffersPerSubSetUpdate.back().begin();
				break;
			}
			case BindingType::ACCELERATION_STRUCTURE: {
				auto& vkwds = vkWriteDescriptorSetsAcc.EmplaceBack();

				vkwds.sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR;
				vkwds.pNext = nullptr;

				writeSet.pNext = &vkwds;

				vkwds.accelerationStructureCount = static_cast<GTSL::uint32>(info.BindingUpdateInfos.ElementCount());
				accelerationStructuresPerSubSetUpdate.EmplaceBack(8, allocator);

				for (auto e : info.BindingUpdateInfos) {
					auto& vkAcc = accPerSubSetUpdate.back().EmplaceBack(e.AccelerationStructureBindingUpdateInfo.AccelerationStructure);
				}

				vkwds.pAccelerationStructures = accelerationStructuresPerSubSetUpdate.back().GetData();
				break;
			}
			}
		}

		renderDevice->VkUpdateDescriptorSets(renderDevice->GetVkDevice(), vkWriteDescriptorSets.GetLength(), vkWriteDescriptorSets.begin(), 0, nullptr);
	}
}
