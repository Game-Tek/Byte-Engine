#pragma once

#include "GAL/RenderDevice.h"

#include "Vulkan.h"

#include <GTSL/Pair.hpp>

#include "GTSL/Buffer.hpp"
#include "GTSL/Allocator.h"
#include "GTSL/DataSizes.h"
#include "GTSL/DLL.h"
#include "GTSL/HashMap.hpp"
#include "GTSL/Vector.hpp"
#include <GTSL/Id.h>
#include <GTSL/HashMap.hpp>

namespace GAL
{
	class VulkanRenderDevice;
#undef ERROR

	template<typename T>
	void setName(const VulkanRenderDevice* renderDevice, T handle, const VkObjectType objectType, const GTSL::Range<const char8_t*> text);

	class VulkanRenderDevice final : public RenderDevice {
	public:
		struct RayTracingCapabilities
		{
			GTSL::uint32 RecursionDepth = 0, ShaderGroupHandleAlignment = 0, ShaderGroupBaseAlignment = 0, ShaderGroupHandleSize = 0, ScratchBuildOffsetAlignment = 0;
			Device BuildDevice;
		};

		VulkanRenderDevice() = default;

		using InitRes = GTSL::Result<GTSL::StaticString<256>>;

		template<class ALLOC>
		[[nodiscard]] InitRes Initialize(const CreateInfo& createInfo, const ALLOC& alloc) {
			debugPrintFunction = createInfo.DebugPrintFunction;
			
			if (!vulkanDLL.LoadLibrary(u8"vulkan-1")) { return InitRes(GTSL::Range(u8"Dynamic library could not be loaded."), false); }

			vulkanDLL.LoadDynamicFunction(u8"vkGetInstanceProcAddr", &VkGetInstanceProcAddr);
			if (!VkGetInstanceProcAddr) { return InitRes(GTSL::Range(u8"vkGetInstanceProcAddr function could not be loaded."), false); }
			
			auto vkAllocate = [](void* data, GTSL::uint64 size, GTSL::uint64 alignment, VkSystemAllocationScope) {
				auto* allocation_info = static_cast<AllocationInfo*>(data);
				return allocation_info->Allocate(allocation_info->UserData, size, alignment);
			};

			auto vkReallocate = [](void* data, void* originalAlloc, const GTSL::uint64 size, const GTSL::uint64 alignment, VkSystemAllocationScope) -> void* {
				auto* allocation_info = static_cast<AllocationInfo*>(data);

				if (originalAlloc && size) {
					return allocation_info->Reallocate(allocation_info->UserData, originalAlloc, size, alignment);
				}

				if (!originalAlloc && size) {
					return allocation_info->Allocate(allocation_info->UserData, size, alignment);
				}

				allocation_info->Deallocate(allocation_info->UserData, originalAlloc);
				return nullptr;
			};

			auto vkFree = [](void* data, void* alloc) -> void {
				if (alloc) {
					auto* allocation_info = static_cast<AllocationInfo*>(data);
					allocation_info->Deallocate(allocation_info->UserData, alloc);
				}
			};
			
			allocationCallbacks.pUserData = &allocationInfo;
			allocationCallbacks.pfnAllocation = vkAllocate; allocationCallbacks.pfnReallocation = vkReallocate; allocationCallbacks.pfnFree = vkFree;
			allocationCallbacks.pfnInternalAllocation = nullptr; allocationCallbacks.pfnInternalFree = nullptr;

			allocationInfo = createInfo.AllocationInfo; debug = createInfo.Debug;

			{
				GTSL::HashMap<uint64, uint32, ALLOC> availableInstanceExtensions(32, alloc);
				VkExtensionProperties extension_properties[64];
				
				uint32 extensionCount = 64;
				getInstanceProcAddr<PFN_vkEnumerateInstanceExtensionProperties>(u8"vkEnumerateInstanceExtensionProperties")(nullptr, &extensionCount, extension_properties);

				for (uint32 i = 0; i < extensionCount; ++i) {
					availableInstanceExtensions.Emplace(GTSL::Hash(reinterpret_cast<const char8_t*>(extension_properties[i].extensionName)), i);
				}				

				VkApplicationInfo vkApplicationInfo{ VK_STRUCTURE_TYPE_APPLICATION_INFO };
				//vkEnumerateInstanceVersion(&vkApplicationInfo.apiVersion);
				vkApplicationInfo.apiVersion = VK_MAKE_VERSION(1, 2, 0);
				vkApplicationInfo.applicationVersion = VK_MAKE_VERSION(createInfo.ApplicationVersion[0], createInfo.ApplicationVersion[1], createInfo.ApplicationVersion[2]);
				vkApplicationInfo.engineVersion = VK_MAKE_VERSION(0, 0, 1);
				//vkApplicationInfo.pApplicationName = createInfo.ApplicationName.begin(); //todo: translate
				vkApplicationInfo.pEngineName = "Game-Tek | ByteEngine";

				VkInstanceCreateInfo vkInstanceCreateInfo{ VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };

				auto setInstancepNext = [&](void* newPointer) {
					if (vkInstanceCreateInfo.pNext) {
						//pointer to last structure now extending vkInstanceCreateInfo
						auto* str = static_cast<GTSL::byte*>(const_cast<void*>(vkInstanceCreateInfo.pNext)); //constness is only there to guarantee VK will not touch it, WE can do it with no problem
						void** strpNext = reinterpret_cast<void**>(str + sizeof(VkStructureType));
						*strpNext = newPointer;
						return;
					}

					vkInstanceCreateInfo.pNext = newPointer;
				};

				GTSL::StaticVector<const char*, 8> instanceLayers;
				GTSL::StaticVector<const char*, 16> instanceExtensions;

				if (debug) {
					instanceLayers.EmplaceBack("VK_LAYER_KHRONOS_validation");
					instanceExtensions.EmplaceBack(VK_EXT_DEBUG_UTILS_EXTENSION_NAME);
				}

				auto tryAddExtension = [&](const GTSL::StringView extensionName) {
					if (auto searchResult = availableInstanceExtensions.TryGet(Hash(extensionName))) {
						instanceExtensions.EmplaceBack(extension_properties[searchResult.Get()].extensionName);
						return true;
					}

					return false;
				};

				for (auto e : createInfo.Extensions) {
					switch (e.First) {
					case Extension::RAY_TRACING: break;
					case Extension::PIPELINE_CACHE_EXTERNAL_SYNC: break;
					case Extension::SCALAR_LAYOUT: break;
					case Extension::SWAPCHAIN_RENDERING: {
						if(!tryAddExtension(u8"VK_KHR_surface")) {
							return InitRes(GTSL::Range(u8"Required instance extension: \nVK_KHR_surface\" is not available."), false);
						}
#if (_WIN32)
						if(!tryAddExtension(u8"VK_KHR_win32_surface")) {
							return InitRes(GTSL::Range(u8"Required instance extension: \nVK_KHR_win32_surface\" is not available."), false);
						}
#endif
						break;
					}
					default:;
					}
				}

#if (_DEBUG)
				GTSL::StaticVector<VkValidationFeatureEnableEXT, 8> enables;
				if (createInfo.SynchronizationValidation) { enables.EmplaceBack(VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT); }
				if (createInfo.PerformanceValidation) { enables.EmplaceBack(VK_VALIDATION_FEATURE_ENABLE_BEST_PRACTICES_EXT); }
				VkValidationFeaturesEXT features = {};
				features.sType = VK_STRUCTURE_TYPE_VALIDATION_FEATURES_EXT;
				features.enabledValidationFeatureCount = enables.GetLength();
				features.pEnabledValidationFeatures = enables.begin();

				setInstancepNext(&features);
				
				VkDebugUtilsMessengerCreateInfoEXT vkDebugUtilsMessengerCreateInfoExt{ VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT };
				vkDebugUtilsMessengerCreateInfoExt.pNext = nullptr;
				vkDebugUtilsMessengerCreateInfoExt.messageSeverity = VK_DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT;
				vkDebugUtilsMessengerCreateInfoExt.messageType = VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
				vkDebugUtilsMessengerCreateInfoExt.pfnUserCallback = debugCallback;
				vkDebugUtilsMessengerCreateInfoExt.pUserData = this;
				vkInstanceCreateInfo.pNext = debug ? &vkDebugUtilsMessengerCreateInfoExt : nullptr;
#endif

				vkInstanceCreateInfo.pApplicationInfo = &vkApplicationInfo;
				vkInstanceCreateInfo.enabledLayerCount = instanceLayers.GetLength();
				vkInstanceCreateInfo.ppEnabledLayerNames = instanceLayers.begin();
				vkInstanceCreateInfo.enabledExtensionCount = instanceExtensions.GetLength();
				vkInstanceCreateInfo.ppEnabledExtensionNames = instanceExtensions.begin();

				if (getInstanceProcAddr<PFN_vkCreateInstance>(u8"vkCreateInstance")(&vkInstanceCreateInfo, GetVkAllocationCallbacks(), &instance) != VK_SUCCESS) {
					return InitRes(GTSL::Range(u8"Failed to create instance."), false);
				}
				
#if (_DEBUG)
				if (debug) {
					getInstanceProcAddr<PFN_vkCreateDebugUtilsMessengerEXT>(u8"vkCreateDebugUtilsMessengerEXT")(instance, &vkDebugUtilsMessengerCreateInfoExt, GetVkAllocationCallbacks(), &debugMessenger);
				}
#endif
			}

			{
				uint32_t physicalDeviceCount{ 16 }; VkPhysicalDevice vkPhysicalDevices[16];
				getInstanceProcAddr<PFN_vkEnumeratePhysicalDevices>(u8"vkEnumeratePhysicalDevices")(instance, &physicalDeviceCount, vkPhysicalDevices);

				if (!physicalDeviceCount) { return InitRes(GTSL::Range(u8"Physical device count returned was 0."), false); }

				uint32 bestScore = 0, bestPhysicalDevice = ~0U;

				for(uint32 i = 0; i < physicalDeviceCount; ++i) {
					VkPhysicalDeviceProperties physical_device_properties;
					getInstanceProcAddr<PFN_vkGetPhysicalDeviceProperties>(u8"vkGetPhysicalDeviceProperties")(vkPhysicalDevices[i], &physical_device_properties);
					VkPhysicalDeviceFeatures physical_device_features;
					getInstanceProcAddr<PFN_vkGetPhysicalDeviceFeatures>(u8"vkGetPhysicalDeviceFeatures")(vkPhysicalDevices[i], &physical_device_features);

					uint64 currentScore = 0;

					switch (physical_device_properties.deviceType) {
					case VK_PHYSICAL_DEVICE_TYPE_OTHER: currentScore += 1000; break;
					case VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: currentScore += 4000; break;
					case VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: currentScore += 5000; break;
					case VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU: currentScore += 3000; break;
					case VK_PHYSICAL_DEVICE_TYPE_CPU: currentScore += 2000; break;
					default: return InitRes(GTSL::Range(u8"Driver returned unhandled value."), false);
					}

					currentScore += physical_device_properties.limits.maxImageDimension2D / 1024;
					currentScore += physical_device_features.textureCompressionBC;

					if(currentScore > bestScore) {
						bestPhysicalDevice = i;
					}
				}

				if (bestPhysicalDevice == ~0U) { return InitRes(GTSL::Range(u8"No suitable physical device could be chosen."), false); }

				physicalDevice = vkPhysicalDevices[bestPhysicalDevice];
			}

			GTSL::HashMap<uint64, uint32, ALLOC> availableDeviceExtensions(256, 0.25f, alloc);
			VkExtensionProperties extension_properties[256];
			GTSL::StaticVector<const char*, 32> deviceExtensions;

			{
				uint32 extensionCount = 256;
				getInstanceProcAddr<PFN_vkEnumerateDeviceExtensionProperties>(u8"vkEnumerateDeviceExtensionProperties")(physicalDevice, nullptr, &extensionCount, extension_properties);

				for (uint32 i = 0; i < extensionCount; ++i) {
					availableDeviceExtensions.Emplace(GTSL::Hash(reinterpret_cast<const char8_t*>(extension_properties[i].extensionName)), i);
				}
			}

			{
				GTSL::StaticVector<VkDeviceQueueCreateInfo, 8> vkDeviceQueueCreateInfos; GTSL::uint32 queueFamiliesCount = 32;
				
				{
					VkQueueFamilyProperties vkQueueFamiliesProperties[32];
					//Get the amount of queue families there are in the physical device.
					getInstanceProcAddr<PFN_vkGetPhysicalDeviceQueueFamilyProperties>(u8"vkGetPhysicalDeviceQueueFamilyProperties")(physicalDevice, &queueFamiliesCount, vkQueueFamiliesProperties);

					GTSL::float32 familiesPriorities[8][8]{ 0.5f };

					GTSL::StaticMap<GTSL::uint64, GTSL::uint8, 16> familyMap;
						
					for (GTSL::uint8 queueIndex = 0; auto & queue : createInfo.Queues) {
						uint32 bestFamilyIndex = 0xFFFFFFFF, lessSetBits = 0xFFFFFFFF;

						for (uint32 i = 0; i < queueFamiliesCount; ++i) {
							auto setBits = GTSL::NumberOfSetBits(vkQueueFamiliesProperties[i].queueFlags);
							if (setBits < lessSetBits && vkQueueFamiliesProperties[i].queueFlags & ToVulkan(queue) && vkQueueFamiliesProperties[i].queueCount) {
								bestFamilyIndex = i;
								lessSetBits = setBits;
							}
						}

						auto res = familyMap.TryEmplace(bestFamilyIndex, vkDeviceQueueCreateInfos.GetLength());

						if (res) {
							auto& queueCreateInfo = vkDeviceQueueCreateInfos.EmplaceBack();
							queueCreateInfo.sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;
							queueCreateInfo.pNext = nullptr;
							queueCreateInfo.flags = 0;
							queueCreateInfo.queueFamilyIndex = bestFamilyIndex;
							queueCreateInfo.queueCount = 0;
							queueCreateInfo.pQueuePriorities = familiesPriorities[res.Get()];
						}

						createInfo.QueueKeys[queueIndex].Queue = res.Get();
						createInfo.QueueKeys[queueIndex].Family = bestFamilyIndex;
						familiesPriorities[bestFamilyIndex][vkDeviceQueueCreateInfos[res.Get()].queueCount] = 1.0f;
						++vkDeviceQueueCreateInfos[res.Get()].queueCount;

						++queueIndex;
					}
				}

				VkPhysicalDeviceProperties2 properties2{ VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2 }; VkPhysicalDeviceFeatures2 features2{ VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2 };

				features2.features.samplerAnisotropy = true;
				features2.features.shaderSampledImageArrayDynamicIndexing = true;
				features2.features.shaderStorageImageArrayDynamicIndexing = true;
				features2.features.shaderUniformBufferArrayDynamicIndexing = true;
				features2.features.shaderStorageBufferArrayDynamicIndexing = true;
				features2.features.shaderInt16 = true; features2.features.shaderInt64 = true;
				features2.features.robustBufferAccess = false;
				features2.features.shaderStorageImageReadWithoutFormat = true; features2.features.shaderStorageImageWriteWithoutFormat = true;

				void** lastProperty = &properties2.pNext; void** lastFeature = &features2.pNext;

				{
					GTSL::Buffer buffer(8192, 8, alloc);

					auto placePropertiesStructure = [&]<typename T>(T** structure, VkStructureType structureType) {
						auto* newStructure = buffer.AllocateStructure<T>(); *lastProperty = static_cast<void*>(newStructure);
						*structure = newStructure; newStructure->sType = structureType;
						lastProperty = &newStructure->pNext;
					};

					auto placeFeaturesStructure = [&]<typename T>(T** structure, VkStructureType structureType) {
						auto* newStructure = buffer.AllocateStructure<T>(); *lastFeature = static_cast<void*>(newStructure);
						*structure = newStructure; newStructure->sType = structureType;
						lastFeature = &newStructure->pNext;
					};

					auto getProperties = [&](void* prop) {
						VkPhysicalDeviceProperties2 props{ VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2 };
						props.pNext = prop;
						getInstanceProcAddr<PFN_vkGetPhysicalDeviceProperties2>(u8"vkGetPhysicalDeviceProperties2")(physicalDevice, &props);
					};

					auto getFeatures = [&](void* feature) {
						VkPhysicalDeviceFeatures2 feats{ VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2 };
						feats.pNext = feature;
						getInstanceProcAddr<PFN_vkGetPhysicalDeviceFeatures2>(u8"vkGetPhysicalDeviceFeatures2")(physicalDevice, &feats);
					};

					auto tryAddExtension = [&](const GTSL::StringView extensionName) {
						if(auto searchResult = availableDeviceExtensions.TryGet(Hash(extensionName))) {
							deviceExtensions.EmplaceBack(extension_properties[searchResult.Get()].extensionName);
							return true;
						}
						return false;
					};

					{
						VkPhysicalDeviceVulkan11Features* structure;
						placeFeaturesStructure(&structure, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES);
						structure->storageBuffer16BitAccess = true; structure->storagePushConstant16 = true;
					}

					{
						VkPhysicalDeviceVulkan12Features* structure;
						placeFeaturesStructure(&structure, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES);
						structure->separateDepthStencilLayouts = true; structure->timelineSemaphore = true;
						structure->bufferDeviceAddress = true; structure->descriptorIndexing = true;
						structure->scalarBlockLayout = true; structure->shaderInt8 = true;
						structure->storageBuffer8BitAccess = true; structure->runtimeDescriptorArray = true;
						structure->descriptorBindingPartiallyBound = true; structure->shaderSampledImageArrayNonUniformIndexing = true;
						structure->shaderStorageBufferArrayNonUniformIndexing = true; structure->shaderStorageImageArrayNonUniformIndexing = true;
						structure->shaderUniformBufferArrayNonUniformIndexing = true;
					}

					if (tryAddExtension(u8"VK_KHR_synchronization2")) {
						VkPhysicalDeviceSynchronization2FeaturesKHR* structure;
						placeFeaturesStructure(&structure, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SYNCHRONIZATION_2_FEATURES_KHR);
						structure->synchronization2 = true;
					} else {
						return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_synchronization2\" is not available."), false);
					}

					if(!tryAddExtension(u8"VK_KHR_copy_commands2")) {
						return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_copy_commands2\" is not available."), false);
					}

					if(!tryAddExtension(u8"VK_KHR_swapchain")) {
						return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_swapchain\" is not available."), false);
					}

					if (tryAddExtension(u8"VK_NV_mesh_shader")) {
						VkPhysicalDeviceMeshShaderFeaturesNV* features;
						placeFeaturesStructure(&features, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MESH_SHADER_FEATURES_NV);
						features->taskShader = true; features->meshShader = true;
					} else {
						return InitRes(GTSL::Range(u8"Required extension: \nVK_NV_mesh_shader\" is not available."), false);						
					}

					if(!tryAddExtension(u8"VK_KHR_maintenance4")) {
						return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_maintenance4\" is not available."), false);
					}

					if (tryAddExtension(u8"VK_KHR_dynamic_rendering")) {
						VkPhysicalDeviceDynamicRenderingFeaturesKHR* features;
						placeFeaturesStructure(&features, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES_KHR);
						features->dynamicRendering = true;
					} else {
						return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_dynamic_rendering\" is not available."), false);
					}

					for (GTSL::uint32 extension = 0; extension < static_cast<GTSL::uint32>(createInfo.Extensions.ElementCount()); ++extension) {
						switch (createInfo.Extensions[extension].First) {
						case Extension::RAY_TRACING: {
							if (tryAddExtension(u8"VK_KHR_acceleration_structure")) {								
								VkPhysicalDeviceAccelerationStructureFeaturesKHR* acceleration_structure_features;
								VkPhysicalDeviceAccelerationStructurePropertiesKHR* acceleration_structure_properties;

								placeFeaturesStructure(&acceleration_structure_features, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR);
								placePropertiesStructure(&acceleration_structure_properties, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_PROPERTIES_KHR);

								acceleration_structure_features->accelerationStructure = true;

								VkPhysicalDeviceAccelerationStructureFeaturesKHR features{ VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR };
								VkPhysicalDeviceAccelerationStructurePropertiesKHR properties{ VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_PROPERTIES_KHR };

								getFeatures(&features);
								getProperties(&properties);

								auto* capabilities = static_cast<RayTracingCapabilities*>(createInfo.Extensions[extension].Second);
								capabilities->BuildDevice = features.accelerationStructureHostCommands ? Device::CPU : Device::GPU;
								capabilities->ScratchBuildOffsetAlignment = properties.minAccelerationStructureScratchOffsetAlignment;
							} else{
								return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_acceleration_structure\" is not available."), false);
							}

							if(tryAddExtension(u8"VK_KHR_ray_query")) {								
							} else {
								return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_ray_query\" is not available."), false);
							}

							if (tryAddExtension(u8"VK_KHR_ray_tracing_pipeline")) {								
								VkPhysicalDeviceRayTracingPipelineFeaturesKHR* ray_tracing_pipeline_features;
								VkPhysicalDeviceRayTracingPipelinePropertiesKHR* ray_tracing_pipeline_properties;

								placeFeaturesStructure(&ray_tracing_pipeline_features, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_PIPELINE_FEATURES_KHR);
								placePropertiesStructure(&ray_tracing_pipeline_properties, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_PIPELINE_PROPERTIES_KHR);

								ray_tracing_pipeline_features->rayTracingPipeline = true;								

								VkPhysicalDeviceRayTracingPipelineFeaturesKHR features{ VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_PIPELINE_FEATURES_KHR };
								VkPhysicalDeviceRayTracingPipelinePropertiesKHR properties{ VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_PIPELINE_PROPERTIES_KHR };

								getProperties(&properties); getFeatures(&features);
								
								auto* capabilities = static_cast<RayTracingCapabilities*>(createInfo.Extensions[extension].Second);
								capabilities->RecursionDepth = properties.maxRayRecursionDepth;
								capabilities->ShaderGroupHandleAlignment = properties.shaderGroupHandleAlignment;
								capabilities->ShaderGroupBaseAlignment = properties.shaderGroupBaseAlignment;
								capabilities->ShaderGroupHandleSize = properties.shaderGroupHandleSize;
							} else {
								return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_ray_tracing_pipeline\" is not available."), false);
							}

							if (tryAddExtension(u8"VK_KHR_pipeline_library")) {								
							} else {
								return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_pipeline_library\" is not available."), false);
							}

							if (tryAddExtension(u8"VK_KHR_deferred_host_operations")) {								
							} else {
								return InitRes(GTSL::Range(u8"Required extension: \nVK_KHR_deferred_host_operations\" is not available."), false);
							}

							break;
						}
						case Extension::PIPELINE_CACHE_EXTERNAL_SYNC: {
							VkPhysicalDevicePipelineCreationCacheControlFeaturesEXT* pipelineCacheSyncControl;
							placeFeaturesStructure(&pipelineCacheSyncControl, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_CREATION_CACHE_CONTROL_FEATURES_EXT);
							pipelineCacheSyncControl->pipelineCreationCacheControl = true;
							break;
						}
						case Extension::SWAPCHAIN_RENDERING: break;
						}
					}

					VkDeviceCreateInfo vkDeviceCreateInfo{ VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
					vkDeviceCreateInfo.pNext = &features2; //extended features
					vkDeviceCreateInfo.queueCreateInfoCount = vkDeviceQueueCreateInfos.GetLength();
					vkDeviceCreateInfo.pQueueCreateInfos = vkDeviceQueueCreateInfos.begin();
					vkDeviceCreateInfo.pEnabledFeatures = nullptr;
					vkDeviceCreateInfo.enabledExtensionCount = deviceExtensions.GetLength();
					vkDeviceCreateInfo.ppEnabledExtensionNames = deviceExtensions.begin();

					if (getInstanceProcAddr<PFN_vkCreateDevice>(u8"vkCreateDevice")(physicalDevice, &vkDeviceCreateInfo, GetVkAllocationCallbacks(), &device) != VK_SUCCESS) {
						return InitRes(GTSL::Range(u8"Failed to create device."), false);
					}

					getInstanceProcAddr(u8"vkGetDeviceProcAddr", &VkGetDeviceProcAddr);
					
					getInstanceProcAddr<PFN_vkGetPhysicalDeviceProperties2>(u8"vkGetPhysicalDeviceProperties2")(physicalDevice, &properties2);
					getInstanceProcAddr<PFN_vkGetPhysicalDeviceFeatures2>(u8"vkGetPhysicalDeviceFeatures2")(physicalDevice, &features2);

					uniformBufferMinOffset = static_cast<GTSL::uint16>(properties2.properties.limits.minUniformBufferOffsetAlignment);
					storageBufferMinOffset = static_cast<GTSL::uint16>(properties2.properties.limits.minStorageBufferOffsetAlignment);
					linearNonLinearAlignment = static_cast<GTSL::uint16>(properties2.properties.limits.bufferImageGranularity);
				}
			}

			getInstanceProcAddr<PFN_vkGetPhysicalDeviceMemoryProperties>(u8"vkGetPhysicalDeviceMemoryProperties")(physicalDevice, &memoryProperties);

			getDeviceProcAddr(u8"vkQueueSubmit", &VkQueueSubmit);
			getDeviceProcAddr(u8"vkQueueSubmit2KHR", &VkQueueSubmit2);
			getDeviceProcAddr(u8"vkQueuePresentKHR", &VkQueuePresent);
			getDeviceProcAddr(u8"vkQueueWaitIdle", &VkQueueWaitIdle);
			getInstanceProcAddr(u8"vkCreateSwapchainKHR", &VkCreateSwapchain);
			getInstanceProcAddr(u8"vkGetSwapchainImagesKHR", &VkGetSwapchainImages);
			getInstanceProcAddr(u8"vkAcquireNextImageKHR", &VkAcquireNextImage);
			getInstanceProcAddr(u8"vkDestroySwapchainKHR", &VkDestroySwapchain);
#if (_WIN64)
			getInstanceProcAddr(u8"vkCreateWin32SurfaceKHR", &VkCreateWin32Surface);
#endif
			getInstanceProcAddr(u8"vkDestroySurfaceKHR", &VkDestroySurface);
			getInstanceProcAddr(u8"vkGetPhysicalDeviceSurfaceCapabilitiesKHR", &VkGetPhysicalDeviceSurfaceCapabilities);
			getInstanceProcAddr(u8"vkGetPhysicalDeviceSurfaceFormatsKHR", &VkGetPhysicalDeviceSurfaceFormats);
			getInstanceProcAddr(u8"vkGetPhysicalDeviceSurfacePresentModesKHR", &VkGetPhysicalDeviceSurfacePresentModes);
			getInstanceProcAddr(u8"vkGetPhysicalDeviceSurfaceSupportKHR", &VkGetPhysicalDeviceSurfaceSupport);
			getDeviceProcAddr(u8"vkCreateBuffer", &VkCreateBuffer);
			getDeviceProcAddr(u8"vkGetBufferDeviceAddress", &VkGetBufferDeviceAddress);
			getDeviceProcAddr(u8"vkDestroyBuffer", &VkDestroyBuffer);
			getDeviceProcAddr(u8"vkGetBufferMemoryRequirements", &VkGetBufferMemoryRequirements);
			getDeviceProcAddr(u8"vkBindBufferMemory", &VkBindBufferMemory);
			getDeviceProcAddr(u8"vkCreateImage", &VkCreateImage);			
			getDeviceProcAddr(u8"vkDestroyImage", &VkDestroyImage);
			getDeviceProcAddr(u8"vkGetImageMemoryRequirements", &VkGetImageMemoryRequirements);
			getDeviceProcAddr(u8"vkBindImageMemory", &VkBindImageMemory);
			getDeviceProcAddr(u8"vkCreateCommandPool", &VkCreateCommandPool);
			getDeviceProcAddr(u8"vkDestroyCommandPool", &VkDestroyCommandPool);
			getDeviceProcAddr(u8"vkResetCommandPool", &VkResetCommandPool);
			getDeviceProcAddr(u8"vkAllocateCommandBuffers", &VkAllocateCommandBuffers);
			getDeviceProcAddr(u8"vkBeginCommandBuffer", &VkBeginCommandBuffer);
			getDeviceProcAddr(u8"vkEndCommandBuffer", &VkEndCommandBuffer);
			getDeviceProcAddr(u8"vkCreateRenderPass", &VkCreateRenderPass);
			getDeviceProcAddr(u8"vkDestroyRenderPass", &VkDestroyRenderPass);
			getDeviceProcAddr(u8"vkCreateFramebuffer", &VkCreateFramebuffer);
			getDeviceProcAddr(u8"vkDestroyFramebuffer", &VkDestroyFramebuffer);
			getDeviceProcAddr(u8"vkCreateShaderModule", &VkCreateShaderModule);
			getDeviceProcAddr(u8"vkDestroyShaderModule", &VkDestroyShaderModule);
			getDeviceProcAddr(u8"vkCreatePipelineLayout", &VkCreatePipelineLayout);
			getDeviceProcAddr(u8"vkDestroyPipelineLayout", &VkDestroyPipelineLayout);
			getDeviceProcAddr(u8"vkCreatePipelineCache", &VkCreatePipelineCache);
			getDeviceProcAddr(u8"vkMergePipelineCaches", &VkMergePipelineCaches);
			getDeviceProcAddr(u8"vkGetPipelineCacheData", &VkGetPipelineCacheData);
			getDeviceProcAddr(u8"vkDestroyPipelineCache", &VkDestroyPipelineCache);
			getDeviceProcAddr(u8"vkCreateDescriptorSetLayout", &VkCreateDescriptorSetLayout);
			getDeviceProcAddr(u8"vkDestroyDescriptorSetLayout", &VkDestroyDescriptorSetLayout);
			getDeviceProcAddr(u8"vkCreateDescriptorPool", &VkCreateDescriptorPool);
			getDeviceProcAddr(u8"vkAllocateDescriptorSets", &VkAllocateDescriptorSets);
			getDeviceProcAddr(u8"vkUpdateDescriptorSets", &VkUpdateDescriptorSets);
			getDeviceProcAddr(u8"vkDestroyDescriptorPool", &VkDestroyDescriptorPool);
			getDeviceProcAddr(u8"vkCreateFence", &VkCreateFence);
			getDeviceProcAddr(u8"vkWaitForFences", &VkWaitForFences);
			getDeviceProcAddr(u8"vkGetFenceStatus", &VkGetFenceStatus);
			getDeviceProcAddr(u8"vkResetFences", &VkResetFences);
			getDeviceProcAddr(u8"vkDestroyFence", &VkDestroyFence);
			getDeviceProcAddr(u8"vkCreateSemaphore", &VkCreateSemaphore);
			getDeviceProcAddr(u8"vkDestroySemaphore", &VkDestroySemaphore);
			getDeviceProcAddr(u8"vkCreateEvent", &VkCreateEvent);
			getDeviceProcAddr(u8"vkSetEvent", &VkSetEvent);
			getDeviceProcAddr(u8"vkResetEvent", &VkResetEvent);
			getDeviceProcAddr(u8"vkDestroyEvent", &VkDestroyEvent);
			getDeviceProcAddr(u8"vkCreateGraphicsPipelines", &VkCreateGraphicsPipelines);
			getDeviceProcAddr(u8"vkCreateComputePipelines", &VkCreateComputePipelines);
			getDeviceProcAddr(u8"vkDestroyPipeline", &VkDestroyPipeline);
			getDeviceProcAddr(u8"vkAllocateMemory", &VkAllocateMemory);
			getDeviceProcAddr(u8"vkFreeMemory", &VkFreeMemory);
			getDeviceProcAddr(u8"vkMapMemory", &VkMapMemory);
			getDeviceProcAddr(u8"vkUnmapMemory", &VkUnmapMemory);
			getDeviceProcAddr(u8"vkCreateImageView", &VkCreateImageView);
			getDeviceProcAddr(u8"vkDestroyImageView", &VkDestroyImageView);
			getDeviceProcAddr(u8"vkCreateSampler", &VkCreateSampler);
			getDeviceProcAddr(u8"vkDestroySampler", &VkDestroySampler);
			getDeviceProcAddr(u8"vkCreateQueryPool", &VkCreateQueryPool);
			getDeviceProcAddr(u8"vkGetQueryPoolResults", &VkGetQueryPoolResults);
			getDeviceProcAddr(u8"vkDestroyQueryPool", &VkDestroyQueryPool);
			getDeviceProcAddr(u8"vkBeginCommandBuffer", &VkBeginCommandBuffer);
			getDeviceProcAddr(u8"vkEndCommandBuffer", &VkEndCommandBuffer);
			getDeviceProcAddr(u8"vkCmdExecuteCommands", &VkCmdExecuteCommands);
			getDeviceProcAddr(u8"vkCmdBeginRenderPass", &VkCmdBeginRenderPass);
			getDeviceProcAddr(u8"vkCmdNextSubpass", &VkCmdNextSubpass);
			getDeviceProcAddr(u8"vkCmdEndRenderPass", &VkCmdEndRenderPass);
			getDeviceProcAddr(u8"vkCmdSetScissor", &VkCmdSetScissor);
			getDeviceProcAddr(u8"vkCmdSetViewport", &VkCmdSetViewport);
			getDeviceProcAddr(u8"vkCmdBindPipeline", &VkCmdBindPipeline);
			getDeviceProcAddr(u8"vkCmdBindDescriptorSets", &VkCmdBindDescriptorSets);
			getDeviceProcAddr(u8"vkCmdPushConstants", &VkCmdPushConstants);
			getDeviceProcAddr(u8"vkCmdBindVertexBuffers", &VkCmdBindVertexBuffers);
			getDeviceProcAddr(u8"vkCmdBindIndexBuffer", &VkCmdBindIndexBuffer);
			getDeviceProcAddr(u8"vkCmdDraw", &VkCmdDraw);
			getDeviceProcAddr(u8"vkCmdDrawIndexed", &VkCmdDrawIndexed);
			getDeviceProcAddr(u8"vkCmdDispatch", &VkCmdDispatch);
			getDeviceProcAddr(u8"vkCmdDispatchIndirect", &VkCmdDispatchIndirect);
			getDeviceProcAddr(u8"vkCmdCopyBuffer", &VkCmdCopyBuffer);
			getDeviceProcAddr(u8"vkCmdCopyBufferToImage", &VkCmdCopyBufferToImage);
			getDeviceProcAddr(u8"vkCmdBlitImage2KHR", &VkCmdBlitImage2KHR);
			getDeviceProcAddr(u8"vkCmdCopyImage", &VkCmdCopyImage);
			getDeviceProcAddr(u8"vkCmdPipelineBarrier", &VkCmdPipelineBarrier);
			getDeviceProcAddr(u8"vkCmdPipelineBarrier2KHR", &VkCmdPipelineBarrier2);
			getDeviceProcAddr(u8"vkCmdSetEvent", &VkCmdSetEvent);
			getDeviceProcAddr(u8"vkCmdResetEvent", &VkCmdResetEvent);

			//getDeviceProcAddr(u8"vkGetDeviceBufferMemoryRequirementsKHR", &VkGetDeviceBufferMemoryRequirements);
			//getDeviceProcAddr(u8"vkGetDeviceImageMemoryRequirementsKHR", &VkGetDeviceImageMemoryRequirements);

			if (availableDeviceExtensions.Find(GTSL::Hash(u8"VK_NV_mesh_shader"))) {
				getDeviceProcAddr(u8"vkCmdDrawMeshTasksNV", &VkCmdDrawMeshTasks);
			} else {
				return InitRes(GTSL::Range(u8"Required extension: \nVK_NV_mesh_shader\" is not available."), false);
			}

			getDeviceProcAddr(u8"vkCmdBeginRenderingKHR", &VkCmdBeginRendering);
			getDeviceProcAddr(u8"vkCmdEndRenderingKHR", &VkCmdEndRendering);

			for (auto e : createInfo.Extensions) {
				switch (e.First) {
				case Extension::RAY_TRACING: {
					getDeviceProcAddr(u8"vkCreateAccelerationStructureKHR", &vkCreateAccelerationStructureKHR);
					getDeviceProcAddr(u8"vkDestroyAccelerationStructureKHR", &vkDestroyAccelerationStructureKHR);
					getDeviceProcAddr(u8"vkCreateRayTracingPipelinesKHR", &vkCreateRayTracingPipelinesKHR);
					getDeviceProcAddr(u8"vkGetAccelerationStructureBuildSizesKHR", &vkGetAccelerationStructureBuildSizesKHR);
					getDeviceProcAddr(u8"vkGetRayTracingShaderGroupHandlesKHR", &vkGetRayTracingShaderGroupHandlesKHR);
					getDeviceProcAddr(u8"vkBuildAccelerationStructuresKHR", &vkBuildAccelerationStructuresKHR);
					getDeviceProcAddr(u8"vkCmdBuildAccelerationStructuresKHR", &vkCmdBuildAccelerationStructuresKHR);
					getDeviceProcAddr(u8"vkGetAccelerationStructureDeviceAddressKHR", &vkGetAccelerationStructureDeviceAddressKHR);
					getDeviceProcAddr(u8"vkCreateDeferredOperationKHR", &vkCreateDeferredOperationKHR);
					getDeviceProcAddr(u8"vkDeferredOperationJoinKHR", &vkDeferredOperationJoinKHR);
					getDeviceProcAddr(u8"vkGetDeferredOperationResultKHR", &vkGetDeferredOperationResultKHR);
					getDeviceProcAddr(u8"vkGetDeferredOperationMaxConcurrencyKHR", &vkGetDeferredOperationMaxConcurrencyKHR);
					getDeviceProcAddr(u8"vkDestroyDeferredOperationKHR", &vkDestroyDeferredOperationKHR);
					getDeviceProcAddr(u8"vkCmdCopyAccelerationStructureKHR", &vkCmdCopyAccelerationStructureKHR);
					getDeviceProcAddr(u8"vkCmdCopyAccelerationStructureToMemoryKHR", &vkCmdCopyAccelerationStructureToMemoryKHR);
					getDeviceProcAddr(u8"vkCmdCopyMemoryToAccelerationStructureKHR", &vkCmdCopyMemoryToAccelerationStructureKHR);
					getDeviceProcAddr(u8"vkCmdWriteAccelerationStructuresPropertiesKHR", &vkCmdWriteAccelerationStructuresPropertiesKHR);
					getDeviceProcAddr(u8"vkCmdTraceRaysKHR", &vkCmdTraceRaysKHR);
					getDeviceProcAddr(u8"vkCmdSetRayTracingPipelineStackSizeKHR", &vkCmdSetRayTracingPipelineStackSizeKHR);
					getDeviceProcAddr(u8"vkGetRayTracingShaderGroupStackSizeKHR", &vkGetRayTracingShaderGroupStackSizeKHR);
					break;
				}
				default:;
				}
			}

			for (GTSL::uint32 i = 0; i < memoryProperties.memoryTypeCount; ++i) {
				memoryTypes[i] = ToGAL(memoryProperties.memoryTypes[i].propertyFlags);
			}

#if BE_DEBUG
				getInstanceProcAddr(u8"vkSetDebugUtilsObjectNameEXT", &vkSetDebugUtilsObjectNameEXT);
				getInstanceProcAddr(u8"vkCmdInsertDebugUtilsLabelEXT", &vkCmdInsertDebugUtilsLabelEXT);
				getInstanceProcAddr(u8"vkCmdBeginDebugUtilsLabelEXT", &vkCmdBeginDebugUtilsLabelEXT);
				getInstanceProcAddr(u8"vkCmdEndDebugUtilsLabelEXT", &vkCmdEndDebugUtilsLabelEXT);

				VkPhysicalDeviceProperties physicalDeviceProperties;
				getInstanceProcAddr<PFN_vkGetPhysicalDeviceProperties>(u8"vkGetPhysicalDeviceProperties")(physicalDevice, &physicalDeviceProperties);

				//NVIDIA's driver have a bug when setting the name for this 3 object types, TODO. fix in the future
				if (physicalDeviceProperties.vendorID != NVIDIA_VENDOR_ID) {					
					GTSL::StaticString<128> instanceName(createInfo.ApplicationName); instanceName += u8" instance";
					setName(this, instance, VK_OBJECT_TYPE_INSTANCE, instanceName);
					
					GTSL::StaticString<128> physicalDeviceName(createInfo.ApplicationName); physicalDeviceName += u8" physical device";
					setName(this, physicalDevice, VK_OBJECT_TYPE_PHYSICAL_DEVICE, physicalDeviceName);

					GTSL::StaticString<128> deviceName(createInfo.ApplicationName); deviceName += u8" device";
					setName(this, device, VK_OBJECT_TYPE_DEVICE, deviceName);					
				}
#endif

			return InitRes(true);
		}

		static constexpr uint32 NVIDIA_VENDOR_ID = 0x10DE;

		void Wait() const { getDeviceProcAddr<PFN_vkDeviceWaitIdle>(u8"vkDeviceWaitIdle")(device); }
		
		void Destroy() {
			Wait();
			getDeviceProcAddr<PFN_vkDestroyDevice>(u8"vkDestroyDevice")(device, GetVkAllocationCallbacks());

#if (_DEBUG)
			if (debug) {
				getInstanceProcAddr<PFN_vkDestroyDebugUtilsMessengerEXT>(u8"vkDestroyDebugUtilsMessengerEXT")(instance, debugMessenger, GetVkAllocationCallbacks());
			}
			debugClear(debugMessenger);
#endif

			getInstanceProcAddr<PFN_vkDestroyInstance>(u8"vkDestroyInstance")(instance, GetVkAllocationCallbacks());

			debugClear(device); debugClear(instance);
		}
		
		~VulkanRenderDevice() = default;

		GPUInfo GetGPUInfo() const {
			GPUInfo result; VkPhysicalDeviceProperties physicalDeviceProperties;

			getInstanceProcAddr<PFN_vkGetPhysicalDeviceProperties>(u8"vkGetPhysicalDeviceProperties")(physicalDevice, &physicalDeviceProperties);

			result.GPUName = GTSL::Range(reinterpret_cast<const char8_t*>(physicalDeviceProperties.deviceName));
			result.DriverVersion = physicalDeviceProperties.driverVersion;
			result.APIVersion = physicalDeviceProperties.apiVersion;
			for (auto e : physicalDeviceProperties.pipelineCacheUUID) {
				result.PipelineCacheUUID[&e - physicalDeviceProperties.pipelineCacheUUID] = e;
			}

			return result;
		}

		[[nodiscard]] uint32_t GetMemoryTypeIndex(MemoryType memoryType) const {			
			for (GTSL::uint32 i = 0; i < memoryProperties.memoryTypeCount; ++i) {
				if (memoryType == memoryTypes[i]) {
					return i;
				}
			}

			return 0xFFFFFFFF;
		}

		struct FindSupportedImageFormat
		{
			GTSL::Range<FormatDescriptor*> Candidates;
			TextureUse TextureUses;
			FormatDescriptor FormatDescriptor;
			Tiling TextureTiling;
		};
		[[nodiscard]] FormatDescriptor FindNearestSupportedImageFormat(const FindSupportedImageFormat& findSupportedImageFormat) const {
			VkFormatProperties format_properties;

			VkFormatFeatureFlags features{};

			TranslateMask(TextureUses::TRANSFER_SOURCE, VK_FORMAT_FEATURE_TRANSFER_SRC_BIT, findSupportedImageFormat.TextureUses, features);
			TranslateMask(TextureUses::TRANSFER_DESTINATION, VK_FORMAT_FEATURE_TRANSFER_DST_BIT, findSupportedImageFormat.TextureUses, features);
			TranslateMask(TextureUses::SAMPLE, VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT, findSupportedImageFormat.TextureUses, features);
			TranslateMask(TextureUses::STORAGE, VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT, findSupportedImageFormat.TextureUses, features);
			if(findSupportedImageFormat.TextureUses & TextureUses::ATTACHMENT) {
				switch (findSupportedImageFormat.FormatDescriptor.Type) {
				case TextureType::COLOR: features |= VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT; break;
				case TextureType::DEPTH: features |= VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT; break;
				}
			}

			for (auto e : findSupportedImageFormat.Candidates) {
				getInstanceProcAddr<PFN_vkGetPhysicalDeviceFormatProperties>(u8"vkGetPhysicalDeviceFormatProperties")(physicalDevice, ToVulkan(MakeFormatFromFormatDescriptor(e)), &format_properties);

				switch (static_cast<VkImageTiling>(findSupportedImageFormat.TextureTiling)) {
				case VK_IMAGE_TILING_LINEAR: {
					if (format_properties.linearTilingFeatures & features) { return e; }
					break;
				}
				case VK_IMAGE_TILING_OPTIMAL: {
					if (format_properties.optimalTilingFeatures & features) { return e; }
					break;
				}
				default: __debugbreak();
				}
			}

			return {};
		}
		
		[[nodiscard]] VkInstance GetVkInstance() const { return instance; }
		[[nodiscard]] VkPhysicalDevice GetVkPhysicalDevice() const { return physicalDevice; }
		[[nodiscard]] VkDevice GetVkDevice() const { return device; }
		
		[[nodiscard]] MemoryType FindNearestMemoryType(MemoryType memoryType) const {
			for (GTSL::uint32 i = 0; i < memoryProperties.memoryTypeCount; ++i) {
				if ((ToGAL(memoryProperties.memoryTypes[i].propertyFlags) & memoryType) == memoryType) {
					return ToGAL(memoryProperties.memoryTypes[i].propertyFlags);
				}
			}

			return MemoryType();
		}

		[[nodiscard]] GTSL::uint32 GetUniformBufferBindingOffsetAlignment() const { return static_cast<GTSL::uint32>(uniformBufferMinOffset); }
		[[nodiscard]] GTSL::uint32 GetStorageBufferBindingOffsetAlignment() const { return static_cast<GTSL::uint32>(storageBufferMinOffset); }

		struct MemoryHeap {
			GTSL::Byte Size;
			MemoryType HeapType;

			GTSL::StaticVector<MemoryType, 16> MemoryTypes;
		};
		
		GTSL::StaticVector<MemoryHeap, 16> GetMemoryHeaps() const {
			GTSL::StaticVector<MemoryHeap, 16> memoryHeaps;

			for (GTSL::uint8 heapIndex = 0; heapIndex < memoryProperties.memoryHeapCount; ++heapIndex) {
				MemoryHeap memoryHeap;
				memoryHeap.Size = GTSL::Byte(memoryProperties.memoryHeaps[heapIndex].size);
				
				TranslateMask(VK_MEMORY_HEAP_DEVICE_LOCAL_BIT, memoryProperties.memoryHeaps[heapIndex].flags, MemoryTypes::GPU, memoryHeap.HeapType);

				for (GTSL::uint8 memType = 0; memType < memoryProperties.memoryTypeCount; ++memType) {
					if (memoryProperties.memoryTypes[memType].heapIndex == heapIndex) {
						memoryHeap.MemoryTypes.EmplaceBack(ToGAL(memoryProperties.memoryTypes[memType].propertyFlags));
					}
				}

				memoryHeaps.EmplaceBack(memoryHeap);
			}

			return memoryHeaps;
		}

		void Log(const GTSL::StringView message, MessageSeverity severity) const {
			GetDebugPrintFunction()(message, severity);
		}

		static auto debugCallback(VkDebugUtilsMessageSeverityFlagBitsEXT messageSeverity, VkDebugUtilsMessageTypeFlagsEXT, const VkDebugUtilsMessengerCallbackDataEXT* pCallbackData, void* pUserData) -> VkBool32 {
			auto* deviceCallback = static_cast<VulkanRenderDevice*>(pUserData);

			switch (messageSeverity) {
			case VK_DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE_BIT_EXT: {
				deviceCallback->GetDebugPrintFunction()(GTSL::StringView(reinterpret_cast<const char8_t*>(pCallbackData->pMessage)), MessageSeverity::MESSAGE);
				break;
			}
			case VK_DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT: {
				deviceCallback->GetDebugPrintFunction()(GTSL::StringView(reinterpret_cast<const char8_t*>(pCallbackData->pMessage)), MessageSeverity::MESSAGE);
				break;
			}
			case VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT: {
				deviceCallback->GetDebugPrintFunction()(GTSL::StringView(reinterpret_cast<const char8_t*>(pCallbackData->pMessage)), MessageSeverity::WARNING);
				break;
			}
			case VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT: {
				deviceCallback->GetDebugPrintFunction()(GTSL::StringView(reinterpret_cast<const char8_t*>(pCallbackData->pMessage)), MessageSeverity::ERROR);
				break;
			}
			case VK_DEBUG_UTILS_MESSAGE_SEVERITY_FLAG_BITS_MAX_ENUM_EXT: break;
			default: __debugbreak(); break;
			}

			return VK_FALSE;
		};


		[[nodiscard]] const VkAllocationCallbacks* GetVkAllocationCallbacks() const { return nullptr; }

		GTSL::uint32 GetLinearNonLinearGranularity() const { return linearNonLinearAlignment; }

		[[nodiscard]] GTSL::Byte GetAccelerationStructureInstanceSize() const { return GTSL::Byte(64); }
		
		GTSL::DLL vulkanDLL;

		PFN_vkGetInstanceProcAddr VkGetInstanceProcAddr; PFN_vkGetDeviceProcAddr VkGetDeviceProcAddr;

		template<typename FT>
		FT getInstanceProcAddr(const char8_t* name) const { return reinterpret_cast<FT>(VkGetInstanceProcAddr(instance, reinterpret_cast<const char*>(name))); }
		template<typename FT>
		void getInstanceProcAddr(const char8_t* name, FT* function) const { *function = *reinterpret_cast<FT>(VkGetInstanceProcAddr(instance, reinterpret_cast<const char*>(name))); }
		
		template<typename FT>
		void getDeviceProcAddr(const char8_t* name, FT* function) const { *function = *reinterpret_cast<FT>(VkGetDeviceProcAddr(device, reinterpret_cast<const char*>(name))); }

		template<typename FT>
		FT getDeviceProcAddr(const char8_t* name) const { return reinterpret_cast<FT>(VkGetDeviceProcAddr(device, reinterpret_cast<const char*>(name))); }
		
		PFN_vkCmdBeginRenderPass VkCmdBeginRenderPass; PFN_vkCmdNextSubpass VkCmdNextSubpass; PFN_vkCmdEndRenderPass VkCmdEndRenderPass;
		PFN_vkCmdDrawIndexed VkCmdDrawIndexed; PFN_vkCmdDraw VkCmdDraw;
		PFN_vkAcquireNextImageKHR VkAcquireNextImage;
		PFN_vkResetCommandPool VkResetCommandPool;
		PFN_vkCreateBuffer VkCreateBuffer; PFN_vkDestroyBuffer VkDestroyBuffer;
		PFN_vkGetBufferMemoryRequirements VkGetBufferMemoryRequirements;
		PFN_vkGetImageMemoryRequirements VkGetImageMemoryRequirements;
		PFN_vkGetBufferDeviceAddress VkGetBufferDeviceAddress;
		PFN_vkCreateImage VkCreateImage; PFN_vkDestroyImage VkDestroyImage;
		PFN_vkCreateImageView VkCreateImageView; PFN_vkDestroyImageView VkDestroyImageView;
		PFN_vkCreateFramebuffer VkCreateFramebuffer; PFN_vkDestroyFramebuffer VkDestroyFramebuffer;
		PFN_vkAllocateMemory VkAllocateMemory; PFN_vkFreeMemory VkFreeMemory;
		PFN_vkMapMemory VkMapMemory; PFN_vkUnmapMemory VkUnmapMemory;
		PFN_vkCreatePipelineCache VkCreatePipelineCache; PFN_vkDestroyPipelineCache VkDestroyPipelineCache;
		PFN_vkMergePipelineCaches VkMergePipelineCaches;
		PFN_vkGetPipelineCacheData VkGetPipelineCacheData;
		PFN_vkCreateShaderModule VkCreateShaderModule; PFN_vkDestroyShaderModule VkDestroyShaderModule;
		PFN_vkCreatePipelineLayout VkCreatePipelineLayout; PFN_vkDestroyPipelineLayout VkDestroyPipelineLayout;
		PFN_vkCreateGraphicsPipelines VkCreateGraphicsPipelines; PFN_vkCreateComputePipelines VkCreateComputePipelines;
		PFN_vkDestroyPipeline VkDestroyPipeline;
		PFN_vkCreateDescriptorPool VkCreateDescriptorPool; PFN_vkDestroyDescriptorPool VkDestroyDescriptorPool;
		PFN_vkCreateDescriptorSetLayout VkCreateDescriptorSetLayout; PFN_vkDestroyDescriptorSetLayout VkDestroyDescriptorSetLayout;
		PFN_vkAllocateDescriptorSets VkAllocateDescriptorSets;
		PFN_vkUpdateDescriptorSets VkUpdateDescriptorSets;
		PFN_vkAllocateCommandBuffers VkAllocateCommandBuffers;
		PFN_vkCreateCommandPool VkCreateCommandPool; PFN_vkDestroyCommandPool VkDestroyCommandPool;
		PFN_vkBeginCommandBuffer VkBeginCommandBuffer; PFN_vkEndCommandBuffer VkEndCommandBuffer;
		PFN_vkCreateSampler VkCreateSampler; PFN_vkDestroySampler VkDestroySampler;
		PFN_vkCreateSwapchainKHR VkCreateSwapchain; PFN_vkDestroySwapchainKHR VkDestroySwapchain;
		PFN_vkGetSwapchainImagesKHR VkGetSwapchainImages;
		PFN_vkCreateRenderPass VkCreateRenderPass; PFN_vkDestroyRenderPass VkDestroyRenderPass;
		PFN_vkBindBufferMemory VkBindBufferMemory;
		PFN_vkBindImageMemory VkBindImageMemory;
		PFN_vkCmdBindPipeline VkCmdBindPipeline;
		PFN_vkCmdDispatch VkCmdDispatch;
		PFN_vkCmdDispatchIndirect VkCmdDispatchIndirect;
		PFN_vkCmdCopyBuffer VkCmdCopyBuffer;
		PFN_vkCmdCopyBufferToImage VkCmdCopyBufferToImage;
		PFN_vkCmdBlitImage2KHR VkCmdBlitImage2KHR;
		PFN_vkCmdCopyImage VkCmdCopyImage;
		PFN_vkCmdPipelineBarrier VkCmdPipelineBarrier;
		PFN_vkCmdPipelineBarrier2KHR VkCmdPipelineBarrier2;
		PFN_vkCmdBindDescriptorSets VkCmdBindDescriptorSets;
		PFN_vkCmdPushConstants VkCmdPushConstants;
		PFN_vkCmdBindVertexBuffers VkCmdBindVertexBuffers;
		PFN_vkCmdBindIndexBuffer VkCmdBindIndexBuffer;
		PFN_vkCmdSetScissor VkCmdSetScissor;
		PFN_vkCmdSetViewport VkCmdSetViewport;
		PFN_vkCmdSetEvent VkCmdSetEvent;
		PFN_vkCmdResetEvent VkCmdResetEvent;
		PFN_vkCreateQueryPool VkCreateQueryPool; PFN_vkDestroyQueryPool VkDestroyQueryPool;
		PFN_vkGetQueryPoolResults VkGetQueryPoolResults;
		PFN_vkQueueSubmit VkQueueSubmit;
		PFN_vkQueueSubmit2KHR VkQueueSubmit2;
		PFN_vkQueuePresentKHR VkQueuePresent;
		PFN_vkQueueWaitIdle VkQueueWaitIdle;
		PFN_vkWaitSemaphores vkWaitSemaphores;
		
		PFN_vkCmdBeginRenderingKHR VkCmdBeginRendering;
		PFN_vkCmdEndRenderingKHR VkCmdEndRendering;

		PFN_vkCmdExecuteCommands VkCmdExecuteCommands;

		//PFN_vkGetDeviceBuffer
		//PFN_vkGetDeviceBufferMemoryRequirementsKHR VkGetDeviceBufferMemoryRequirements;
		//PFN_vkGetDeviceBufferMemoryRequirementsKHR VkGetDeviceImageMemoryRequirements;

#if (_WIN64)
		PFN_vkCreateWin32SurfaceKHR VkCreateWin32Surface;
#endif
		PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR VkGetPhysicalDeviceSurfaceCapabilities;
		PFN_vkGetPhysicalDeviceSurfaceFormatsKHR VkGetPhysicalDeviceSurfaceFormats;
		PFN_vkGetPhysicalDeviceSurfacePresentModesKHR VkGetPhysicalDeviceSurfacePresentModes;
		PFN_vkGetPhysicalDeviceSurfaceSupportKHR VkGetPhysicalDeviceSurfaceSupport;
		PFN_vkDestroySurfaceKHR VkDestroySurface;
		PFN_vkCreateFence VkCreateFence; PFN_vkDestroyFence VkDestroyFence;
		PFN_vkWaitForFences VkWaitForFences; PFN_vkResetFences VkResetFences;
		PFN_vkGetFenceStatus VkGetFenceStatus;
		PFN_vkCreateSemaphore VkCreateSemaphore; PFN_vkDestroySemaphore VkDestroySemaphore;
		PFN_vkCreateEvent VkCreateEvent; PFN_vkDestroyEvent VkDestroyEvent;
		PFN_vkSetEvent VkSetEvent; PFN_vkResetEvent VkResetEvent;		
		
		PFN_vkCreateAccelerationStructureKHR vkCreateAccelerationStructureKHR = nullptr;
		PFN_vkDestroyAccelerationStructureKHR vkDestroyAccelerationStructureKHR = nullptr;
		PFN_vkCreateRayTracingPipelinesKHR vkCreateRayTracingPipelinesKHR = nullptr;
		PFN_vkGetAccelerationStructureBuildSizesKHR vkGetAccelerationStructureBuildSizesKHR = nullptr;
		PFN_vkGetAccelerationStructureDeviceAddressKHR vkGetAccelerationStructureDeviceAddressKHR = nullptr;
		PFN_vkGetRayTracingShaderGroupHandlesKHR vkGetRayTracingShaderGroupHandlesKHR = nullptr;
		PFN_vkBuildAccelerationStructuresKHR vkBuildAccelerationStructuresKHR = nullptr;
		PFN_vkCmdBuildAccelerationStructuresKHR vkCmdBuildAccelerationStructuresKHR = nullptr;
		PFN_vkCreateDeferredOperationKHR vkCreateDeferredOperationKHR = nullptr;
		PFN_vkDeferredOperationJoinKHR vkDeferredOperationJoinKHR = nullptr;
		PFN_vkGetDeferredOperationResultKHR vkGetDeferredOperationResultKHR = nullptr;
		PFN_vkGetDeferredOperationMaxConcurrencyKHR vkGetDeferredOperationMaxConcurrencyKHR = nullptr;
		PFN_vkDestroyDeferredOperationKHR vkDestroyDeferredOperationKHR = nullptr;
		PFN_vkCmdCopyAccelerationStructureKHR vkCmdCopyAccelerationStructureKHR = nullptr;
		PFN_vkCmdCopyAccelerationStructureToMemoryKHR vkCmdCopyAccelerationStructureToMemoryKHR = nullptr;
		PFN_vkCmdCopyMemoryToAccelerationStructureKHR vkCmdCopyMemoryToAccelerationStructureKHR = nullptr;
		PFN_vkCmdWriteAccelerationStructuresPropertiesKHR vkCmdWriteAccelerationStructuresPropertiesKHR = nullptr;
		PFN_vkCmdTraceRaysKHR vkCmdTraceRaysKHR = nullptr;
		PFN_vkCmdSetRayTracingPipelineStackSizeKHR vkCmdSetRayTracingPipelineStackSizeKHR = nullptr;
		PFN_vkGetRayTracingShaderGroupStackSizeKHR vkGetRayTracingShaderGroupStackSizeKHR = nullptr;

		PFN_vkCmdDrawMeshTasksNV VkCmdDrawMeshTasks;
		
#if (_DEBUG)
		PFN_vkSetDebugUtilsObjectNameEXT vkSetDebugUtilsObjectNameEXT = nullptr;
		PFN_vkCmdInsertDebugUtilsLabelEXT vkCmdInsertDebugUtilsLabelEXT = nullptr;
		PFN_vkCmdBeginDebugUtilsLabelEXT vkCmdBeginDebugUtilsLabelEXT = nullptr;
		PFN_vkCmdEndDebugUtilsLabelEXT vkCmdEndDebugUtilsLabelEXT = nullptr;
#endif

	private:
#if (_DEBUG)
		VkDebugUtilsMessengerEXT debugMessenger = nullptr;
#endif
		bool debug = false;

		GTSL::uint16 uniformBufferMinOffset, storageBufferMinOffset, linearNonLinearAlignment;
		
		VkInstance instance = nullptr;
		VkPhysicalDevice physicalDevice = nullptr;
		VkDevice device = nullptr;
		AllocationInfo allocationInfo;
		VkAllocationCallbacks allocationCallbacks;
		VkPhysicalDeviceMemoryProperties memoryProperties;

		MemoryType memoryTypes[16];
	};

	template<typename T>
	void setName(const VulkanRenderDevice* renderDevice, T handle, const VkObjectType objectType, const GTSL::Range<const char8_t*> text) {
#if BE_DEBUG
			VkDebugUtilsObjectNameInfoEXT vkDebugUtilsObjectNameInfo{ VK_STRUCTURE_TYPE_DEBUG_UTILS_OBJECT_NAME_INFO_EXT };
			vkDebugUtilsObjectNameInfo.objectHandle = reinterpret_cast<GTSL::uint64>(handle);
			vkDebugUtilsObjectNameInfo.objectType = objectType;
			vkDebugUtilsObjectNameInfo.pObjectName = reinterpret_cast<const char*>(text.GetData());
			renderDevice->vkSetDebugUtilsObjectNameEXT(renderDevice->GetVkDevice(), &vkDebugUtilsObjectNameInfo);
#endif
	}
}
