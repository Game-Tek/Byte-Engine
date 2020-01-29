#pragma once

struct UniformBufferCreateInfo
{
	size_t Size = 0;
};

struct UniformBufferUpdateInfo
{
	void* Data = nullptr;
	size_t Size = 0;
};

class UniformBuffer
{
public:
	virtual void UpdateBuffer(const UniformBufferUpdateInfo& _BUI) const = 0;
};
