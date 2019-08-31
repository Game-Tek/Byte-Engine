#include "VKDespcriptorSet.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKDescriptorSetCreator::VKDescriptorSetCreator(VKDevice* _Device, const VkDescriptorSetAllocateInfo* _VkDSCI) : VKObjectCreator<VkDescriptorSet>(_Device)
{
	vkAllocateDescriptorSets(_Device->GetVkDevice(), _VkDSCI, &Handle);
}