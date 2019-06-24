#pragma once

#include "Core.h"

#include "Vulkan.h"

#include "FVector.hpp"

class Window;

GS_STRUCT Vulkan_Physical_Device
{
	static uint8 GetDeviceTypeScore(VkPhysicalDeviceType _Type)
	{
		switch (_Type)
		{
		case VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: return 255;
		case VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: return 254;
		case VK_PHYSICAL_DEVICE_TYPE_CPU: return 253;
		default: return 0;
		}
	}
public:
	VkPhysicalDevice PhysicalDevice = VK_NULL_HANDLE;

	Vulkan_Physical_Device(VkInstance _Instance)
	{
		////////////////////////////////////////
		// PHYSICAL DEVICE CREATION/SELECTION //
		////////////////////////////////////////

		uint32_t DeviceCount = 0;
		vkEnumeratePhysicalDevices(_Instance, &DeviceCount, nullptr);	//Get the amount of physical devices(GPUs) there are.

		FVector<VkPhysicalDevice> PhysicalDevices(DeviceCount);
		vkEnumeratePhysicalDevices(_Instance, &DeviceCount, PhysicalDevices.data());	//Fill the array with VkPhysicalDevice handles to every physical device (GPU) there is.

		FVector<VkPhysicalDeviceProperties> PhysicalDevicesProperties;	//Create a vector to store the physical device properties associated with every physical device we queried before.
		//Loop through every physical device there is while getting/querying the physical device properties of each one and storing them in the vector.
		for (size_t i = 0; i < DeviceCount; i++)
		{
			vkGetPhysicalDeviceProperties(PhysicalDevices[i], &PhysicalDevicesProperties[i]);
		}


		uint16 BestPhysicalDeviceIndex = 0;	//Variable to hold the index into the physical devices properties vector of the current winner of the GPU sorting processes.
		uint16 i = 0;
		//Sort by Device Type.
		while (PhysicalDevicesProperties.length() > i)
		{
			if (GetDeviceTypeScore(PhysicalDevicesProperties[i].deviceType) > GetDeviceTypeScore(PhysicalDevicesProperties[BestPhysicalDeviceIndex].deviceType))
			{
				BestPhysicalDeviceIndex = i;

				PhysicalDevicesProperties.erase(i);

				i--;
			}

			i++;
		}

		PhysicalDevice = PhysicalDevices[i];	//Set the VulkanDevice's physical device as the one which resulted a winner from the sort.
	}
};

GS_STRUCT Vulkan_Queue
{
public:
	VkQueue Queue = nullptr;
	VkDeviceQueueCreateInfo QueueCreateInfo = { VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO };

	Vulkan_Queue() = default;

	Vulkan_Queue(Vulkan_Physical_Device _PhysicalDevice, VkQueueFlagBits _QueueType)
	{
		uint32_t QueueFamiliesCount = 0;
		vkGetPhysicalDeviceQueueFamilyProperties(_PhysicalDevice.PhysicalDevice, &QueueFamiliesCount, nullptr);	//Get the amount of queue families there are in the physical device.

		FVector<VkQueueFamilyProperties> queueFamilies(QueueFamiliesCount);
		vkGetPhysicalDeviceQueueFamilyProperties(_PhysicalDevice.PhysicalDevice, &QueueFamiliesCount, queueFamilies.data());

		uint8 i = 0;
		while (true)
		{
			if (queueFamilies[i].queueCount > 0 && queueFamilies[i].queueFlags & _QueueType)
			{
				break;
			}

			i++;
		}

		QueueCreateInfo.queueFamilyIndex = i;
		QueueCreateInfo.queueCount = 1;
		float queuePriority = 1.0f;
		QueueCreateInfo.pQueuePriorities = &queuePriority;
	}

	void SetFromDevice(const VulkanDevice& _Device)
	{
		vkGetDeviceQueue(_Device.Device, QueueCreateInfo.queueFamilyIndex, 0, &Queue);
	}
};

GS_STRUCT Vulkan_Vertex_Input
{
public:
	VkPipelineVertexInputStateCreateInfo VertexInputInfo = { VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };

	Vulkan_Vertex_Input()
	{
		VertexInputInfo.vertexBindingDescriptionCount = 0;
		VertexInputInfo.pVertexBindingDescriptions = nullptr; // Optional
		VertexInputInfo.vertexAttributeDescriptionCount = 0;
		VertexInputInfo.pVertexAttributeDescriptions = nullptr; // Optional
	}
};

GS_STRUCT Vulkan_Pipeline_Viewport
{
public:
	VkPipelineViewportStateCreateInfo viewportState = { VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO };

	Vulkan_Pipeline_Viewport()
	{
		VkViewport Viewport = {};
		Viewport.x = 0.0f;
		Viewport.y = 0.0f;
		Viewport.width = (float)swapChainExtent.width;
		Viewport.height = (float)swapChainExtent.height;
		Viewport.minDepth = 0.0f;
		Viewport.maxDepth = 1.0f;

		VkRect2D Scissor = {};
		Scissor.offset = { 0, 0 };
		Scissor.extent = swapChainExtent;

		viewportState.viewportCount = 1;
		viewportState.pViewports = &Viewport;
		viewportState.scissorCount = 1;
		viewportState.pScissors = &Scissor;
	}
};

GS_STRUCT Vulkan_Pipeline_Rasterization
{
	VkPipelineRasterizationStateCreateInfo RasterizationState = { VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO };

	Vulkan_Pipeline_Rasterization(bool ClampDepth = false)
	{
		RasterizationState.depthClampEnable = ClampDepth;
		RasterizationState.rasterizerDiscardEnable = VK_FALSE;
		RasterizationState.polygonMode = VK_POLYGON_MODE_FILL;

		//The lineWidth member describes the thickness of lines in terms of number of fragments.
		//The maximum line width that is supported depends on the hardware and any line thicker than 1.0f
		//requires you to enable the wideLines GPU feature.
		RasterizationState.lineWidth = 1.0f;
		RasterizationState.cullMode = VK_CULL_MODE_BACK_BIT;
		RasterizationState.frontFace = VK_FRONT_FACE_CLOCKWISE;
		RasterizationState.depthBiasEnable = VK_FALSE;
		RasterizationState.depthBiasConstantFactor = 0.0f; // Optional
		RasterizationState.depthBiasClamp = 0.0f; // Optional
		RasterizationState.depthBiasSlopeFactor = 0.0f; // Optional
	}
};

GS_STRUCT Vulkan_Pipeline_ColorBlend
{
	VkPipelineColorBlendAttachmentState ColorBlendAttachment = {};
	VkPipelineColorBlendStateCreateInfo colorBlending = { VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO };


	Vulkan_Pipeline_ColorBlend(bool _Blend = false)
	{
		ColorBlendAttachment.colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT | VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
		ColorBlendAttachment.blendEnable = _Blend;
		ColorBlendAttachment.srcColorBlendFactor = VK_BLEND_FACTOR_ONE; // Optional
		ColorBlendAttachment.dstColorBlendFactor = VK_BLEND_FACTOR_ZERO; // Optional
		ColorBlendAttachment.colorBlendOp = VK_BLEND_OP_ADD; // Optional
		ColorBlendAttachment.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE; // Optional
		ColorBlendAttachment.dstAlphaBlendFactor = VK_BLEND_FACTOR_ZERO; // Optional
		ColorBlendAttachment.alphaBlendOp = VK_BLEND_OP_ADD; // Optional

		colorBlending.logicOpEnable = VK_FALSE;
		colorBlending.logicOp = VK_LOGIC_OP_COPY; // Optional
		colorBlending.attachmentCount = 1;
		colorBlending.pAttachments = &ColorBlendAttachment;
		colorBlending.blendConstants[0] = 0.0f; // Optional
		colorBlending.blendConstants[1] = 0.0f; // Optional
		colorBlending.blendConstants[2] = 0.0f; // Optional
		colorBlending.blendConstants[3] = 0.0f; // Optional
	}
};

GS_STRUCT Vulkan_Pipeline_Dynamic_State
{
	VkPipelineDynamicStateCreateInfo DynamicState = { VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO };
	
	Vulkan_Pipeline_Dynamic_State()
	{
		VkDynamicState DynamicStates[] = {
			VK_DYNAMIC_STATE_VIEWPORT,
		};

		DynamicState.dynamicStateCount = 1;
		DynamicState.pDynamicStates = DynamicStates;
	}
};

GS_CLASS VulkanInstance
{
public:
	VkInstance Instance = nullptr;

	VulkanInstance(const FVector<const char*>& _Extensions)
	{
		VkApplicationInfo AppInfo = { VK_STRUCTURE_TYPE_APPLICATION_INFO };
		AppInfo.apiVersion = VK_API_VERSION_1_1;	//Should check if version is available vi vkEnumerateInstanceVersion().
		AppInfo.applicationVersion = VK_MAKE_VERSION(1, 0, 0);
		AppInfo.engineVersion = VK_MAKE_VERSION(1, 0, 0);
		AppInfo.pApplicationName = "Hello Triangle!";
		AppInfo.pEngineName = "Game Studio";
		AppInfo.pNext = nullptr;

		const char* InstanceLayers[] = { "VK_LAYER_LUNARG_standard_validation" };

		VkInstanceCreateInfo InstanceCreateInfo = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
		InstanceCreateInfo.pApplicationInfo = &AppInfo;
		InstanceCreateInfo.enabledLayerCount = 1;
		InstanceCreateInfo.ppEnabledLayerNames = InstanceLayers;
		InstanceCreateInfo.enabledExtensionCount = _Extensions.length();
		InstanceCreateInfo.ppEnabledExtensionNames = _Extensions.data();

		GS_VK_CHECK(vkCreateInstance(&InstanceCreateInfo, ALLOCATOR, &Instance), "Failed to create instance!")
	}
	~VulkanInstance()
	{
		vkDestroyInstance(Instance, ALLOCATOR);
	}
};

GS_CLASS VulkanDevice
{
public:
	VkDevice Device = nullptr;
	Vulkan_Physical_Device PhysicalDevice;

	Vulkan_Queue Queue;

	VulkanDevice(VkInstance _Instance) : PhysicalDevice(_Instance)
	{
		////////////////////////////////////////
		//      DEVICE CREATION/SELECTION     //
		////////////////////////////////////////

		VkPhysicalDeviceFeatures deviceFeatures = {};	//COME BACK TO

		const char* DeviceExtensions[] = { VK_KHR_SWAPCHAIN_EXTENSION_NAME };

		Vulkan_Queue l_Queue(PhysicalDevice, VK_QUEUE_GRAPHICS_BIT);

		VkDeviceCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
		CreateInfo.pQueueCreateInfos = &l_Queue.QueueCreateInfo;
		CreateInfo.queueCreateInfoCount = 1;
		CreateInfo.pEnabledFeatures = &deviceFeatures;
		CreateInfo.enabledExtensionCount = 1;
		CreateInfo.ppEnabledExtensionNames = DeviceExtensions;

		GS_VK_CHECK(vkCreateDevice(PhysicalDevice.PhysicalDevice, &CreateInfo, ALLOCATOR, &Device), "Failed to create logical device!")

		Queue = l_Queue;

		Queue.SetFromDevice(*this);
	}

	~VulkanDevice()
	{
		vkDestroyDevice(Device, ALLOCATOR);
	}
};

GS_CLASS VulkanSurface
{
	static VkSurfaceFormatKHR PickBestFormat(const Vulkan_Physical_Device& _PhysicalDevice, const VulkanSurface& _Surface)
	{
		uint32_t FormatsCount = 0;
		vkGetPhysicalDeviceSurfaceFormatsKHR(_PhysicalDevice.PhysicalDevice, _Surface.Surface, &FormatsCount, nullptr);
		FVector<VkSurfaceFormatKHR> SurfaceFormats(FormatsCount);
		vkGetPhysicalDeviceSurfaceFormatsKHR(_PhysicalDevice.PhysicalDevice, _Surface.Surface, &FormatsCount, SurfaceFormats.data());

		uint8 i = 0;
		if (SurfaceFormats[i].colorSpace == VK_FORMAT_B8G8R8A8_UNORM && SurfaceFormats[i].format == VK_COLOR_SPACE_SRGB_NONLINEAR_KHR)
		{
			return SurfaceFormats[i];
		}
	}

public:
	VkSurfaceKHR Surface = nullptr;
	VkSurfaceFormatKHR Format = {};
	VkExtent2D Extent = { 1280 , 720 };

	VulkanSurface(const VulkanInstance & _Instance, const Window & _Window, const VulkanSurface& _Surface, const Vulkan_Physical_Device& _PhysicalDevice)
	{
		VkWin32SurfaceCreateInfoKHR WcreateInfo = { VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR };
		WcreateInfo.hwnd = _Window; //TODO
		WcreateInfo.hinstance = GetModuleHandle(nullptr);

		VkSurfaceCapabilitiesKHR SurfaceCapabilities = {};
		vkGetPhysicalDeviceSurfaceCapabilitiesKHR(_PhysicalDevice.PhysicalDevice, _Surface.Surface, &SurfaceCapabilities);

		uint32_t PresentFormatCount = 1;
		VkPresentModeKHR PresentMode = {};

		vkGetPhysicalDeviceSurfaceSupportKHR(_PhysicalDevice.PhysicalDevice, i, _Surface.Surface, &tt);
		vkGetPhysicalDeviceSurfacePresentModesKHR(_PhysicalDevice.PhysicalDevice, _Surface.Surface, &PresentFormatCount, &PresentMode);

		if (vkCreateWin32SurfaceKHR(_Instance.Instance, &WcreateInfo, ALLOCATOR, &Surface) != VK_SUCCESS) {
			throw std::runtime_error("Failed to create window surface!");
		}

		Format = PickBestFormat(_PhysicalDevice, *this);
	}
	~VulkanSurface()
	{
		vkDestroySurfaceKHR(_Instance.Instance, Surface, ALLOCATOR);
	}
};

GS_CLASS VulkanSwapchain
{
	static uint8 ScorePresentMode(VkPresentModeKHR _PresentMode)
	{
		switch (_PresentMode)
		{
			case VK_PRESENT_MODE_MAILBOX_KHR: return 255;
			case VK_PRESENT_MODE_FIFO_KHR: return 254;
			default:	break;
		}
	}

	static VkPresentModeKHR PickPresentMode(const Vulkan_Physical_Device & _PhysicalDevice, const VulkanSurface & _Surface)
	{
		uint32_t PresentModesCount = 0;
		vkGetPhysicalDeviceSurfacePresentModesKHR(_PhysicalDevice.PhysicalDevice, _Surface.Surface, &PresentModesCount, nullptr);
		FVector<VkPresentModeKHR> PresentModes(PresentModesCount);
		vkGetPhysicalDeviceSurfacePresentModesKHR(_PhysicalDevice.PhysicalDevice, _Surface.Surface, &PresentModesCount, PresentModes.data());

		uint8 BestScore = 0;
		uint8 BestPresentModeIndex = 0;
		for (uint8 i = 0; i < PresentModesCount; i++)
		{
			if (ScorePresentMode(PresentModes[i]) > BestScore)
			{
				BestScore = ScorePresentMode(PresentModes[i]);

				BestPresentModeIndex = i;
			}
		}

		return PresentModes[BestPresentModeIndex];
	}

public:
	VkSwapchainKHR Swapchain = nullptr;
	VkPresentModeKHR PresentationMode = VK_PRESENT_MODE_IMMEDIATE_KHR;

	FVector<VulkanImageView> ImageViews;

	VulkanSwapchain(const VulkanDevice & _Device, const VulkanSurface & _Surface) : PresentationMode(PickPresentMode(_Device.PhysicalDevice, _Surface))
	{
		VkSwapchainCreateInfoKHR SCcreateInfo = { VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR };
		SCcreateInfo.surface = _Surface.Surface;
		SCcreateInfo.minImageCount = 3;
		SCcreateInfo.imageFormat = _Surface.Format.format;
		SCcreateInfo.imageColorSpace = _Surface.Format.colorSpace;
		SCcreateInfo.imageExtent = _Surface.Extent;
		SCcreateInfo.imageArrayLayers = 1;
		SCcreateInfo.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
		SCcreateInfo.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
		SCcreateInfo.queueFamilyIndexCount = 1; // Optional
		SCcreateInfo.pQueueFamilyIndices = nullptr;
		SCcreateInfo.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
		//The compositeAlpha field specifies if the alpha channel should be used for blending with other windows in the window system.
		//You'll almost always want to simply ignore the alpha channel, hence VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR.
		SCcreateInfo.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
		SCcreateInfo.presentMode = PresentationMode;
		SCcreateInfo.clipped = VK_TRUE;
		SCcreateInfo.oldSwapchain = VK_NULL_HANDLE;
		
		GS_VK_CHECK(vkCreateSwapchainKHR(_Device.Device, &SCcreateInfo, ALLOCATOR, &Swapchain), "Failed to create swap chain!")

		uint32_t ImageCount = 0;
		vkGetSwapchainImagesKHR(_Device.Device, Swapchain, &ImageCount, nullptr);
		FVector<VkImage> l_ImageViews(ImageCount);
		vkGetSwapchainImagesKHR(_Device.Device, Swapchain, &ImageCount, l_ImageViews.data());

		for (uint8 i = 0; i < ImageCount; i++)
		{
			ImageViews[i].Create(l_ImageViews[i]);
		}
	}

	~VulkanSwapchain()
	{
		vkDestroySwapchainKHR(DEVICE, Swapchain, ALLOCATOR);
	}

	uint32 AcquireNextImage(const VulkanSemaphore & _Semaphore)
	{
		uint32_t ImageIndex;

		vkAcquireNextImageKHR(DEVICE, Swapchain, std::numeric_limits<uint64_t>::max(), _Semaphore.Semaphore, VK_NULL_HANDLE, &ImageIndex);

		return ImageIndex;
	}

	void Present(const Vulkan_Queue & _Queue)
	{
		vkQueuePresentKHR(_Queue.Queue, &PresentInfo);
	}
};

GS_CLASS VulkanImageView
{
public:
	VkImageView ImageView = nullptr;

	VulkanImageView() = default;

	VulkanImageView(VkImage _Image)
	{
		Create(_Image);
	}

	void Create(VkImage _Image)
	{
		VkImageViewCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
		CreateInfo.image = _Image;
		CreateInfo.viewType = VK_IMAGE_VIEW_TYPE_2D;
		CreateInfo.format = VK_FORMAT_B8G8R8A8_UNORM;
		CreateInfo.components.r = VK_COMPONENT_SWIZZLE_IDENTITY;
		CreateInfo.components.g = VK_COMPONENT_SWIZZLE_IDENTITY;
		CreateInfo.components.b = VK_COMPONENT_SWIZZLE_IDENTITY;
		CreateInfo.components.a = VK_COMPONENT_SWIZZLE_IDENTITY;
		CreateInfo.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
		CreateInfo.subresourceRange.baseMipLevel = 0;
		CreateInfo.subresourceRange.levelCount = 1;
		CreateInfo.subresourceRange.baseArrayLayer = 0;
		CreateInfo.subresourceRange.layerCount = 1;

		GS_VK_CHECK(vkCreateImageView(Device, &CreateInfo, ALLOCATOR, &ImageView), "Failed to create image views!")
	}

	~VulkanImageView()
	{
		vkDestroyImageView(Device, ImageView, ALLOCATOR);
	}
};

GS_CLASS VulkanShader
{
public:
	VkShaderModule ShaderModule = nullptr;

	VulkanShader(void * Code, const size_t CodeSize)
	{
		VkShaderModuleCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO };
		CreateInfo.codeSize = CodeSize;
		CreateInfo.pCode = reinterpret_cast<const uint32_t*>(Code);


		GS_VK_CHECK(vkCreateShaderModule(_Device, &CreateInfo, ALLOCATOR, &ShaderModule), "Failed to create shader module!")
	}

	~VulkanShader()
	{
		vkDestroyShaderModule(DEVICE, ShaderModule, ALLOCATOR);
	}
};

GS_CLASS VulkanPipelineLayout
{
public:
	VkPipelineLayout PipelineLayout = nullptr;

	VulkanPipelineLayout()
	{
		VkPipelineLayoutCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO };
		CreateInfo.setLayoutCount = 0; // Optional
		CreateInfo.pSetLayouts = nullptr; // Optional
		CreateInfo.pushConstantRangeCount = 0; // Optional
		CreateInfo.pPushConstantRanges = nullptr; // Optional

		GS_VK_CHECK(vkCreatePipelineLayout(DEVICE, &CreateInfo, ALLOCATOR, &PipelineLayout), "Failed to create PipelineLayout!")
	}
	~VulkanPipelineLayout()
	{
		vkDestroyPipelineLayout(DEVICE, PipelineLayout, ALLOCATOR);
	}
};

GS_CLASS VulkanRenderPass
{
public:
	VkRenderPass RenderPass = nullptr;

	VulkanRenderPass()
	{
		VkAttachmentDescription colorAttachment = {};
		colorAttachment.format = swapChainImageFormat;
		colorAttachment.samples = VK_SAMPLE_COUNT_1_BIT;	//Should match that of the SwapChain images.
		colorAttachment.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
		colorAttachment.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
		colorAttachment.stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
		colorAttachment.stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
		colorAttachment.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
		colorAttachment.finalLayout = VK_IMAGE_LAYOUT_PRESENT_SRC_KHR;

		//ATTACHMENT = Render Pass.
		VkAttachmentReference colorAttachmentRef = {};
		colorAttachmentRef.attachment = 0;
		colorAttachmentRef.layout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;

		VkSubpassDescription subpass = {};
		subpass.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
		subpass.colorAttachmentCount = 1;
		subpass.pColorAttachments = &colorAttachmentRef;

		VkRenderPassCreateInfo renderPassInfo = { VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO };
		renderPassInfo.attachmentCount = 1;
		renderPassInfo.pAttachments = &colorAttachment;
		renderPassInfo.subpassCount = 1;
		renderPassInfo.pSubpasses = &subpass;

		GS_VK_CHECK(vkCreateRenderPass(DEVICE, &CreateInfo, ALLOCATOR, &RenderPass), "Failed to create RenderPass!")
	}

	~VulkanRenderPass()
	{
		vkDestroyRenderPass(DEVICE, &RenderPass, ALLOCATOR);
	}
};

/*
GS_CLASS VulkanGraphicsPipeline
{
public:
	VkPipeline GraphicsPipeline;

	VulkanGraphicsPipeline()
	{
		VkGraphicsPipelineCreateInfo pipelineInfo = { VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO };
		pipelineInfo.stageCount = 2;
		pipelineInfo.pStages = shaderStages;
		pipelineInfo.pVertexInputState = &vertexInputInfo;
		pipelineInfo.pInputAssemblyState = &inputAssembly;
		pipelineInfo.pViewportState = &viewportState;
		pipelineInfo.pRasterizationState = &rasterizer;
		pipelineInfo.pMultisampleState = &multisampling;
		pipelineInfo.pDepthStencilState = nullptr; // Optional
		pipelineInfo.pColorBlendState = &colorBlending;
		pipelineInfo.pDynamicState = nullptr; // Optional
		pipelineInfo.layout = pipelineLayout;
		pipelineInfo.renderPass = renderPass;
		pipelineInfo.subpass = 0;
		pipelineInfo.basePipelineHandle = VK_NULL_HANDLE; // Optional
		pipelineInfo.basePipelineIndex = -1; // Optional

		GS_VK_CHECK(vkCreateGraphicsPipelines(DEVICE, VK_NULL_HANDLE, 1, &CreateInfo, ALLOCATOR, &GraphicsPipeline), "Failed to create Graphics Pipeline!")
	}

	~VulkanGraphicsPipeline()
	{
		vkDestroyPipeline(DEVICE, &GraphicsPipeline, ALLOCATOR);
	}
};
*/

/*
GS_CLASS VulkanFrameBuffer
{
public:
	VkFramebuffer Framebuffer = nullptr;

	VulkanFrameBuffer()
	{
		VkFramebufferCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO };
		CreateInfo.renderPass = renderPass;
		CreateInfo.attachmentCount = 1;
		CreateInfo.pAttachments = attachments;
		CreateInfo.width = swapChainExtent.width;
		CreateInfo.height = swapChainExtent.height;
		CreateInfo.layers = 1;

		GS_VK_CHECK(vkCreateFramebuffer(DEVICE, &CreateInfo, ALLOCATOR, &Framebuffer), "Failed to create Frambuffer!")
	}

	~VulkanFrameBuffer()
	{
		vkDestroyFramebuffer(DEVICE, Framebuffer, ALLOCATOR);
	}
};
*/

GS_CLASS VulkanCommandPool
{
public:
	VkCommandPool CommandPool = nullptr;

	VulkanCommandPool()
	{
		VkCommandPoolCreateInfo poolInfo = { VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO };
		poolInfo.queueFamilyIndex = queueFamilyIndices.graphicsFamily.value();
		poolInfo.flags = 0; // Optional

		GS_VK_CHECK(vkCreateCommandPool(DEVICE, &CreateInfo, ALLOCATOR, &CommandPool), "Failed to create Command Pool!")
	}

	~VulkanCommandPool()
	{
		vkDestroyCommandPool(DEVICE, CommandPool, ALLOCATOR);
	}
};

GS_CLASS VulkanCommandBuffer
{
public:
	VkCommandBuffer CommandBuffer = nullptr;

	VulkanCommandBuffer(const VulkanCommandPool & _CommandPool)
	{
		VkCommandBufferAllocateInfo allocInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO };
		allocInfo.commandPool = _CommandPool.CommandPool;
		allocInfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
		allocInfo.commandBufferCount = (uint32_t)commandBuffers.size();

		GS_VK_CHECK(vkAllocateCommandBuffers(DEVICE, &allocInfo, &CommandBuffer), "Failed to Allocate Command Buffer!")
	}

	void Begin()
	{
		VkCommandBufferBeginInfo BeginInfo = { VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO };
		BeginInfo.flags = VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT;
		BeginInfo.pInheritanceInfo = nullptr; // Optional

		GS_VK_CHECK(vkBeginCommandBuffer(CommandBuffer, &BeginInfo), "Failed to begin Command Buffer!")
	}

	void End()
	{
		GS_VK_CHECK(vkEndCommandBuffer(CommandBuffer), "Failed to end Command Buffer!")
	}
};

//Ask for how to get extent
GS_CLASS VulkanRenderPass
{
public:
	VkRenderPass RenderPass = nullptr;

	VkRenderPassBeginInfo RenderPassInfo = { VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO };

	VulkanRenderPass(const VulkanFrameBuffer & _FB, const VulkanSwapchain & _Swapchain)
	{
		RenderPassInfo.renderPass = RenderPass;
		RenderPassInfo.framebuffer = _FB.Framebuffer;
		RenderPassInfo.renderArea.offset = { 0, 0 };
		RenderPassInfo.renderArea.extent = _Swapchain.Extent;

		VkClearValue clearColor = { 0.0f, 0.0f, 0.0f, 1.0f };
		RenderPassInfo.clearValueCount = 1;
		RenderPassInfo.pClearValues = &clearColor;
	}

	void Begin(const VulkanCommandBuffer & _CommandBuffer)
	{
		vkCmdBeginRenderPass(_CommandBuffer.CommandBuffer, &RenderPassInfo, VK_SUBPASS_CONTENTS_INLINE);
	}

	void End(const VulkanCommandBuffer& _CommandBuffer)
	{
		vkCmdEndRenderPass(_CommandBuffer.CommandBuffer);
	}
};

/*
GS_CLASS VulkanSemaphore
{
public:
	VkSemaphore Semaphore = nullptr;

	VulkanSemaphore()
	{
		VkSemaphoreCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO };

		GS_VK_CHECK(vkCreateSemaphore(DEVICE, &CreateInfo, ALLOCATOR, &Semaphore), "Failed to create Semaphore!")
	}

	~VulkanSemaphore()
	{
		vkDestroySemaphore(DEVICE, Semaphore, ALLOCATOR);
	}
};
*/

enum class BufferType : uint8
{
	BUFFER_VERTEX,
	BUFFER_INDEX,
	BUFFER_UNIFORM
};

/*
GS_CLASS VulkanBuffer
{
	static VkBufferUsageFlagBits BufferTypeToVkBufferUsageFlagBits(BufferType _Type)
	{
		switch (_Type)
		{
		case BufferType::BUFFER_VERTEX: return VK_BUFFER_USAGE_VERTEX_BUFFER_BIT;
		case BufferType::BUFFER_INDEX: return VK_BUFFER_USAGE_INDEX_BUFFER_BIT;
		case BufferType::BUFFER_UNIFORM: return VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
		default:	break;
		}
	}
	
	static uint32_t FindMemoryType(Vulkan_Physical_Device _PD, uint32 _TypeFilter, VkMemoryPropertyFlags _Properties)
	{
		VkPhysicalDeviceMemoryProperties memProperties;
		vkGetPhysicalDeviceMemoryProperties(_PD.PhysicalDevice, &memProperties);

		for (uint32_t i = 0; i < memProperties.memoryTypeCount; i++)
		{
			if ((_TypeFilter & (1 << i)) && (memProperties.memoryTypes[i].propertyFlags & _Properties) == _Properties)
			{
				return i;
			}
		}
	}

	void Allocate(Vulkan_Physical_Device _PD)
	{
		VkMemoryRequirements memRequirements;
		vkGetBufferMemoryRequirements(DEVICE, Buffer, &memRequirements);

		VkMemoryAllocateInfo allocInfo = { VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO };
		allocInfo.allocationSize = memRequirements.size;
		allocInfo.memoryTypeIndex = FindMemoryType(_PD, memRequirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);

		GS_VK_CHECK(vkAllocateMemory(DEVICE, &AllocateInfo, ALLOCATOR, &Memory), "Failed to allocate memory!")

		vkBindBufferMemory(DEVICE, Buffer, Memory, 0);
	}

	void FillBuffer(const void * _Data, size_t _Size)
	{
		void* Data;
		vkMapMemory(DEVICE, Memory, 0, _Size, 0, &Data);
		memcpy(Data, _Data, _Size);
		vkUnmapMemory(DEVICE, Memory);
	}

public:
	VkBuffer Buffer = nullptr;
	VkDeviceMemory Memory = nullptr;

	VulkanBuffer(BufferType _Type, size_t _Size)
	{
		VkBufferCreateInfo CreateInfo = { VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO };
		CreateInfo.size = _Size;
		CreateInfo.usage = BufferTypeToVkBufferUsageFlagBits(_Type);
		CreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

		GS_VK_CHECK(vkCreateBuffer(DEVICE, &CreateInfo, ALLOCATOR, &Buffer), "Failed to allocate Buffer!")
	}

	~VulkanBuffer()
	{
		vkDestroyBuffer(DEVICE, Buffer, ALLOCATOR);
		vkFreeMemory(DEVICE, Memory, ALLOCATOR);
	}
}
*/