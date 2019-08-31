#include "VKFence.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "VKDevice.h"

VKFenceCreator::VKFenceCreator(VKDevice* _Device, const VkFenceCreateInfo* _VkFCI) : VKObjectCreator<VkFence>(_Device)
{
	GS_VK_CHECK(vkCreateFence(m_Device->GetVkDevice(), _VkFCI, ALLOCATOR, &Handle), "Failed to create Fence!");
}

VKFence::~VKFence()
{
	vkDestroyFence(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}

void VKFence::Wait() const
{
	vkWaitForFences(m_Device->GetVkDevice(), 1, &Handle, true, 0xffffffffffffffff);
}

void VKFence::Reset() const
{
	vkResetFences(m_Device->GetVkDevice(), 1, &Handle);
}

void VKFence::WaitForFences(uint8 _Count, VKFence* _Fences, bool _WaitForAll)
{
	FVector<VkFence> Fences(1);
	vkWaitForFences(_Fences->m_Device->GetVkDevice(), _Count, Fences.data(), _WaitForAll, 0xffffffffffffffff);
}

void VKFence::ResetFences(uint8 _Count, VKFence* _Fences)
{
	FVector<VkFence> Fences(1);
	vkResetFences(_Fences->m_Device->GetVkDevice(), _Count, Fences.data());
}

bool VKFence::GetStatus() const
{
	const VkResult Result = vkGetFenceStatus(m_Device->GetVkDevice(), Handle);

	switch (Result)
	{
	case VK_SUCCESS: return true;
	case VK_NOT_READY: return false;
	default: return false;
	}
}
