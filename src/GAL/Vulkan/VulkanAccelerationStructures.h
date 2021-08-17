#pragma once

#include "Vulkan.h"
#include "GTSL/Range.h"
#include <GAL/Vulkan/VulkanBuffer.h>

#include "GTSL/Vector.hpp"

namespace GAL {
	struct BuildAccelerationStructureInfo;

	struct GeometryTriangles {
		ShaderDataType VertexPositionFormat; IndexType IndexType; GTSL::uint8 VertexStride;
		DeviceAddress VertexData, IndexData;
		GTSL::uint32 FirstVertex, MaxVertices;
	};

	struct GeometryAABB {
		DeviceAddress Data;
		GTSL::uint32 Stride;
	};

	struct GeometryInstances {
		DeviceAddress Data;
	};

	struct Geometry {
		GeometryType Type;
		union {
			GeometryTriangles Triangles;
			GeometryAABB AABB;
			GeometryInstances Instances;
		};
		GeometryFlag Flags;

		Geometry(const GeometryTriangles triangles, const GeometryFlag flags, const GTSL::uint32 primCount, const GTSL::uint32 primOffset) : Type(GeometryType::TRIANGLES), Triangles(triangles), Flags(flags), PrimitiveCount(primCount), PrimitiveOffset(primOffset) {}
		Geometry(const GeometryAABB aabb, const GeometryFlag flags, const GTSL::uint32 primCount, const GTSL::uint32 primOffset) : Type(GeometryType::AABB), AABB(aabb), Flags(flags), PrimitiveCount(primCount), PrimitiveOffset(primOffset) {}
		Geometry(const GeometryInstances instances, const GeometryFlag flags, const GTSL::uint32 primCount, const GTSL::uint32 primOffset) : Type(GeometryType::INSTANCES), Instances(instances), Flags(flags), PrimitiveCount(primCount), PrimitiveOffset(primOffset) {}
		
		void SetGeometryTriangles(const GeometryTriangles triangles) { Type = GeometryType::TRIANGLES; Triangles = triangles; }
		void SetGeometryAABB(const GeometryAABB aabb) { Type = GeometryType::AABB; AABB = aabb; }
		void SetGeometryInstances(const GeometryInstances instances) { Type = GeometryType::INSTANCES; Instances = instances; }

		GTSL::uint32 PrimitiveCount, PrimitiveOffset;
	};
	
	inline void buildGeometryAndRange(const Geometry& descriptor, VkAccelerationStructureGeometryKHR& vkGeometry, VkAccelerationStructureBuildRangeInfoKHR& buildRange) {
		auto& s = descriptor;
		vkGeometry.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_KHR;
		vkGeometry.pNext = nullptr;
		vkGeometry.flags = ToVkGeometryFlagsKHR(s.Flags);
		buildRange.primitiveCount = descriptor.PrimitiveCount;
		buildRange.primitiveOffset = descriptor.PrimitiveOffset;

		switch (s.Type) {
			case GeometryType::TRIANGLES: {
				auto& t = s.Triangles;
				vkGeometry.geometryType = VK_GEOMETRY_TYPE_TRIANGLES_KHR;
				vkGeometry.geometry.triangles.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR;
				vkGeometry.geometry.triangles.pNext = nullptr;
				vkGeometry.geometry.triangles.vertexData.deviceAddress = static_cast<GTSL::uint64>(t.VertexData);
				vkGeometry.geometry.triangles.indexData.deviceAddress = static_cast<GTSL::uint64>(t.IndexData);
				vkGeometry.geometry.triangles.transformData.deviceAddress = 0;
				vkGeometry.geometry.triangles.indexType = ToVulkan(t.IndexType);
				vkGeometry.geometry.triangles.maxVertex = t.MaxVertices;
				vkGeometry.geometry.triangles.vertexFormat = ToVulkan(t.VertexPositionFormat);
				vkGeometry.geometry.triangles.vertexStride = t.VertexStride;
				buildRange.firstVertex = descriptor.Triangles.FirstVertex;
				buildRange.transformOffset = 0;
				break;
			}
			case GeometryType::AABB: {
				auto& a = s.AABB;
				vkGeometry.geometryType = VK_GEOMETRY_TYPE_AABBS_KHR;
				vkGeometry.geometry.aabbs.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_AABBS_DATA_KHR;
				vkGeometry.geometry.aabbs.pNext = nullptr;
				vkGeometry.geometry.aabbs.data.deviceAddress = static_cast<GTSL::uint64>(a.Data);
				vkGeometry.geometry.aabbs.stride = a.Stride;
				buildRange.firstVertex = 0;
				buildRange.transformOffset = 0;
				break;
			}
			case GeometryType::INSTANCES: {
				const auto& a = s.Instances;
				vkGeometry.geometryType = VK_GEOMETRY_TYPE_INSTANCES_KHR;
				vkGeometry.geometry.instances.sType = VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR;
				vkGeometry.geometry.instances.pNext = nullptr;
				vkGeometry.geometry.instances.data.deviceAddress = static_cast<GTSL::uint64>(a.Data);
				vkGeometry.geometry.instances.arrayOfPointers = false;
				buildRange.firstVertex = 0;
				buildRange.transformOffset = 0;
				break;
			}
		}
	}
	
	class VulkanAccelerationStructure {
	public:
		[[nodiscard]] VkAccelerationStructureKHR GetVkAccelerationStructure() const { return accelerationStructure; }

		VulkanAccelerationStructure() = default;

		void GetMemoryRequirements(const VulkanRenderDevice* renderDevice, GTSL::Range<const Geometry*> geometries, Device buildDevice, AccelerationStructureFlag flags, GTSL::uint32* accStructureSize, GTSL::uint32* scratchSize) {
			GTSL::StaticVector<VkAccelerationStructureGeometryKHR, 8> vkGeometries;
			GTSL::StaticVector<uint32_t, 8> primitiveCounts;
			for (auto& e : geometries) {
				VkAccelerationStructureGeometryKHR geometryKhr; VkAccelerationStructureBuildRangeInfoKHR buildRange;
				buildGeometryAndRange(e, geometryKhr, buildRange);
				vkGeometries.EmplaceBack(geometryKhr); primitiveCounts.EmplaceBack(e.PrimitiveCount);
			}

			auto type = vkGeometries[0].geometryType == VK_GEOMETRY_TYPE_INSTANCES_KHR ? VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR : VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;

			VkAccelerationStructureBuildGeometryInfoKHR buildInfo{ VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR };
			buildInfo.flags = ToVulkan(flags);
			buildInfo.type = type;
			buildInfo.geometryCount = vkGeometries.GetLength();
			buildInfo.mode = VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR;
			buildInfo.pGeometries = vkGeometries.begin();

			VkAccelerationStructureBuildSizesInfoKHR buildSizes{ VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR };
			renderDevice->vkGetAccelerationStructureBuildSizesKHR(renderDevice->GetVkDevice(),
				ToVulkan(buildDevice), &buildInfo, primitiveCounts.begin(), &buildSizes);

			*accStructureSize = static_cast<GTSL::uint32>(buildSizes.accelerationStructureSize); *scratchSize = static_cast<GTSL::uint32>(buildSizes.buildScratchSize);
		}
		
		void Initialize(const VulkanRenderDevice* renderDevice, GTSL::Range<const Geometry*> geometries, VulkanBuffer buffer, GTSL::uint32 size, GTSL::uint32 offset) {
			VkAccelerationStructureCreateInfoKHR vkAccelerationStructureCreateInfoKhr{ VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_CREATE_INFO_KHR };
			vkAccelerationStructureCreateInfoKhr.createFlags = 0;
			vkAccelerationStructureCreateInfoKhr.type = geometries[0].Type == GeometryType::INSTANCES ? VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR : VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;
			vkAccelerationStructureCreateInfoKhr.offset = 0;
			vkAccelerationStructureCreateInfoKhr.deviceAddress = 0;
			vkAccelerationStructureCreateInfoKhr.buffer = buffer.GetVkBuffer();
			vkAccelerationStructureCreateInfoKhr.size = size;

			renderDevice->vkCreateAccelerationStructureKHR(renderDevice->GetVkDevice(), &vkAccelerationStructureCreateInfoKhr, renderDevice->GetVkAllocationCallbacks(), &accelerationStructure);

			//setName(info.RenderDevice, accelerationStructure, VK_OBJECT_TYPE_ACCELERATION_STRUCTURE_KHR, info.Name);
		}

		void Destroy(const VulkanRenderDevice* renderDevice) {
			renderDevice->vkDestroyAccelerationStructureKHR(renderDevice->GetVkDevice(), accelerationStructure, renderDevice->GetVkAllocationCallbacks());
			debugClear(accelerationStructure);
		}

		DeviceAddress GetAddress(const VulkanRenderDevice* renderDevice) const {
			VkAccelerationStructureDeviceAddressInfoKHR deviceAddress{ VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_DEVICE_ADDRESS_INFO_KHR };
			deviceAddress.accelerationStructure = accelerationStructure;
			return DeviceAddress(renderDevice->vkGetAccelerationStructureDeviceAddressKHR(renderDevice->GetVkDevice(), &deviceAddress));
		}

		[[nodiscard]] GTSL::uint64 GetHandle() const { return reinterpret_cast<GTSL::uint64>(accelerationStructure); }
		
		//UTILITY
		static void BuildAccelerationStructure(const VulkanRenderDevice* renderDevice,
		                                       GTSL::Range<const BuildAccelerationStructureInfo*>
		                                       buildAccelerationStructureInfos);

	protected:
		VkAccelerationStructureKHR accelerationStructure = nullptr;

	};

	struct BuildAccelerationStructureInfo {
		VulkanAccelerationStructure SourceAccelerationStructure, DestinationAccelerationStructure;
		GTSL::Range<const Geometry*> Geometries;
		DeviceAddress ScratchBufferAddress;
		GTSL::uint32 Flags = 0;
	};
	
	inline void VulkanAccelerationStructure::BuildAccelerationStructure(const VulkanRenderDevice* renderDevice,
	                                                                    GTSL::Range<const BuildAccelerationStructureInfo*>
	                                                                    buildAccelerationStructureInfos) {
		GTSL::StaticVector<VkAccelerationStructureBuildGeometryInfoKHR, 8> buildGeometryInfos;
		GTSL::StaticVector<GTSL::StaticVector<VkAccelerationStructureGeometryKHR, 8>, 8> geometriesPerAccelerationStructure;
		GTSL::StaticVector<GTSL::StaticVector<VkAccelerationStructureBuildRangeInfoKHR, 8>, 8> buildRangesPerAccelerationStructure;
		GTSL::StaticVector<VkAccelerationStructureBuildRangeInfoKHR*, 8> buildRangesRangePerAccelerationStructure;

		for (GTSL::uint32 accStrInfoIndex = 0; accStrInfoIndex < static_cast<GTSL::uint32>(buildAccelerationStructureInfos.ElementCount()); ++accStrInfoIndex) {
			auto& source = buildAccelerationStructureInfos[accStrInfoIndex];

			geometriesPerAccelerationStructure.EmplaceBack();
			buildRangesPerAccelerationStructure.EmplaceBack();
			buildRangesRangePerAccelerationStructure.EmplaceBack(buildRangesPerAccelerationStructure[accStrInfoIndex].begin());

			for (GTSL::uint32 i = 0; i < static_cast<GTSL::uint32>(source.Geometries.ElementCount()); ++i) {
				VkAccelerationStructureGeometryKHR accelerationStructureGeometry;
				VkAccelerationStructureBuildRangeInfoKHR buildRange;
				buildGeometryAndRange(source.Geometries[i], accelerationStructureGeometry, buildRange);
				geometriesPerAccelerationStructure[accStrInfoIndex].EmplaceBack(accelerationStructureGeometry);
				buildRangesPerAccelerationStructure[accStrInfoIndex].EmplaceBack(buildRange);
			}

			VkAccelerationStructureBuildGeometryInfoKHR buildGeometryInfo{ VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR };
			buildGeometryInfo.flags = source.Flags;
			buildGeometryInfo.srcAccelerationStructure = source.SourceAccelerationStructure.GetVkAccelerationStructure();
			buildGeometryInfo.dstAccelerationStructure = source.DestinationAccelerationStructure.GetVkAccelerationStructure();
			buildGeometryInfo.type = source.Geometries[0].Type == GeometryType::INSTANCES
				                         ? VK_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL_KHR
				                         : VK_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL_KHR;
			buildGeometryInfo.pGeometries = geometriesPerAccelerationStructure[accStrInfoIndex].begin();
			buildGeometryInfo.ppGeometries = nullptr;
			buildGeometryInfo.geometryCount = geometriesPerAccelerationStructure[accStrInfoIndex].GetLength();
			buildGeometryInfo.scratchData.deviceAddress = static_cast<GTSL::uint64>(source.ScratchBufferAddress);
			buildGeometryInfo.mode = source.SourceAccelerationStructure.GetVkAccelerationStructure()
				                         ? VK_BUILD_ACCELERATION_STRUCTURE_MODE_UPDATE_KHR
				                         : VK_BUILD_ACCELERATION_STRUCTURE_MODE_BUILD_KHR;
			buildGeometryInfos.EmplaceBack(buildGeometryInfo);
		}

		renderDevice->vkBuildAccelerationStructuresKHR(renderDevice->GetVkDevice(), nullptr,
		                                               static_cast<GTSL::uint32>(buildAccelerationStructureInfos.ElementCount()),
		                                               buildGeometryInfos.begin(),
		                                               buildRangesRangePerAccelerationStructure.begin());
	}

	inline void WriteInstance(const VulkanAccelerationStructure accelerationStructure, GTSL::uint32 instanceIndex, GeometryFlag geometryFlags, const VulkanRenderDevice* renderDevice, void* data, GTSL::uint32 index, Device device) {
		auto& inst = *(static_cast<VkAccelerationStructureInstanceKHR*>(data) + index);
		inst.flags = ToVkGeometryInstanceFlagsKHR(geometryFlags);
		inst.accelerationStructureReference = device == Device::CPU ? accelerationStructure.GetHandle() : static_cast<GTSL::uint64>(accelerationStructure.GetAddress(renderDevice));
		inst.instanceCustomIndex = instanceIndex;
		inst.mask = 0xFF;
	}

	inline void WriteInstanceMatrix(const GTSL::Matrix3x4& matrix3X4, void* data, GTSL::uint32 index) {
		auto& inst = *(static_cast<VkAccelerationStructureInstanceKHR*>(data) + index);
		inst.transform = ToVulkan(matrix3X4);
	}

	inline void WriteInstanceBindingTableRecordOffset(const GTSL::uint32 offset, void* data, GTSL::uint32 index) {
		auto& inst = *(static_cast<VkAccelerationStructureInstanceKHR*>(data) + index);
		inst.instanceShaderBindingTableRecordOffset = offset;
	}
}
