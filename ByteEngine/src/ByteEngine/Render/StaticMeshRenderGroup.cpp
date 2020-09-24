#include "StaticMeshRenderGroup.h"

#include "RenderSystem.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup()
{
}

void StaticMeshRenderGroup::Initialize(const InitializeInfo& initializeInfo)
{
	auto render_device = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	positions.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());
	meshes.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());
	meshesRefTable.Initialize(32, GetPersistentAllocator());
	renderAllocations.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* render_system = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");

	GTSL::ForEach(meshes, [&](Mesh& mesh)
	{
		mesh.Buffer.Destroy(render_system->GetRenderDevice());
	}
	);
	
	GTSL::ForEach(renderAllocations, [&](RenderAllocation& alloc) { render_system->DeallocateLocalBufferMemory(alloc); });
}

ComponentReference StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 bufferSize = 0, indicesOffset = 0; uint16 indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &indexSize, &indexSize, &bufferSize, &indicesOffset);

	Buffer::CreateInfo bufferCreateInfo;
	bufferCreateInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Buffer. StaticMesh: "); name += addStaticMeshInfo.MeshName.GetHash();
		bufferCreateInfo.Name = name.begin();
	}
	
	bufferCreateInfo.Size = bufferSize;
	bufferCreateInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer(bufferCreateInfo);
	
	HostRenderAllocation allocation;
	
	RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
	memoryAllocationInfo.Buffer = scratch_buffer;
	memoryAllocationInfo.Allocation = &allocation;
	addStaticMeshInfo.RenderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

	uint32 index = positions.GetFirstFreeIndex().Get();
	
	if (meshesRefTable.Find(addStaticMeshInfo.Material.MaterialType))
	{
		auto& meshList = meshesRefTable.At(addStaticMeshInfo.Material.MaterialType);
		meshList.EmplaceBack(index);
	}
	else
	{
		auto& meshList = meshesRefTable.Emplace(addStaticMeshInfo.Material.MaterialType);
		meshList.Initialize(8, GetPersistentAllocator());
		meshList.EmplaceBack(index);
	}
	
	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index, addStaticMeshInfo.Material);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(bufferSize, static_cast<byte*>(allocation.Data));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = indexSize;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName);
	positions.EmplaceAt(index);

	++meshCount;
	
	return index;
}

System::ComponentReference StaticMeshRenderGroup::AddRayTracedStaticMesh(const AddRayTracedStaticMeshInfo& addStaticMeshInfo)
{
	uint32 bufferSize = 0, indicesOffset = 0; uint16 indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &indexSize, &indexSize, &bufferSize, &indicesOffset);

	Buffer::CreateInfo bufferCreateInfo;
	bufferCreateInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Buffer. StaticMesh: "); name += addStaticMeshInfo.MeshName.GetHash();
		bufferCreateInfo.Name = name.begin();
	}

	bufferCreateInfo.Size = bufferSize;
	bufferCreateInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer(bufferCreateInfo);

	HostRenderAllocation allocation;

	RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
	memoryAllocationInfo.Buffer = scratch_buffer;
	memoryAllocationInfo.Allocation = &allocation;
	addStaticMeshInfo.RenderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

	uint32 index = positions.GetFirstFreeIndex().Get();

	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index, addStaticMeshInfo.Material);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };

	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onRayTracedStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(bufferSize, static_cast<byte*>(allocation.Data));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = indexSize;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName);
	positions.EmplaceAt(index);

	++meshCount;

	return index;
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	auto loadStaticMesh = [](TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad, StaticMeshRenderGroup* staticMeshRenderGroup)
	{
		MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

		RenderAllocation allocation;

		Buffer::CreateInfo createInfo;
		createInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Device buffer. StaticMeshRenderGroup: "); name += onStaticMeshLoad.ResourceName;
			createInfo.Name = name.begin();
		}

		createInfo.Size = onStaticMeshLoad.DataBuffer.Bytes();
		createInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_DESTINATION;
		Buffer deviceBuffer(createInfo);

		{
			RenderSystem::BufferLocalMemoryAllocationInfo memoryAllocationInfo;
			memoryAllocationInfo.Allocation = &allocation;
			memoryAllocationInfo.Buffer = deviceBuffer;
			loadInfo->RenderSystem->AllocateLocalBufferMemory(memoryAllocationInfo);
		}

		RenderSystem::BufferCopyData buffer_copy_data;
		buffer_copy_data.SourceOffset = 0;
		buffer_copy_data.DestinationOffset = 0;
		buffer_copy_data.SourceBuffer = loadInfo->ScratchBuffer;
		buffer_copy_data.DestinationBuffer = deviceBuffer;
		buffer_copy_data.Size = onStaticMeshLoad.DataBuffer.Bytes();
		buffer_copy_data.Allocation = loadInfo->Allocation;
		loadInfo->RenderSystem->AddBufferCopy(buffer_copy_data);

		{
			Mesh mesh;
			mesh.IndexType = SelectIndexType(onStaticMeshLoad.IndexSize);
			mesh.IndicesCount = onStaticMeshLoad.IndexCount;
			mesh.IndicesOffset = onStaticMeshLoad.IndicesOffset;
			mesh.Buffer = deviceBuffer;
			mesh.Material = loadInfo->Material;

			staticMeshRenderGroup->meshes.EmplaceAt(loadInfo->InstanceId, mesh);
		}

		staticMeshRenderGroup->renderAllocations.EmplaceAt(loadInfo->InstanceId, loadInfo->Allocation);

		GTSL::Delete(loadInfo, staticMeshRenderGroup->GetPersistentAllocator());
	};

	taskInfo.GameInstance->AddFreeDynamicTask(GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad, StaticMeshRenderGroup*)>::Create(loadStaticMesh),
		GTSL::Array<TaskDependency, 2>{ {"StaticMeshRenderGroup", AccessType::READ_WRITE} }, GTSL::MoveRef(onStaticMeshLoad), this);
}

void StaticMeshRenderGroup::onRayTracedStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	auto loadStaticMesh = [](TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad, StaticMeshRenderGroup* staticMeshRenderGroup)
	{
		MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

		RenderAllocation allocation;

		Buffer::CreateInfo createInfo;
		createInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Device buffer. StaticMeshRenderGroup: "); name += onStaticMeshLoad.ResourceName;
			createInfo.Name = name.begin();
		}

		createInfo.Size = onStaticMeshLoad.DataBuffer.Bytes();
		createInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_DESTINATION | BufferType::ADDRESS;
		Buffer deviceBuffer(createInfo);

		{
			RenderSystem::BufferLocalMemoryAllocationInfo memoryAllocationInfo;
			memoryAllocationInfo.Allocation = &allocation;
			memoryAllocationInfo.Buffer = deviceBuffer;
			loadInfo->RenderSystem->AllocateLocalBufferMemory(memoryAllocationInfo);
		}

		RenderSystem::BufferCopyData buffer_copy_data;
		buffer_copy_data.SourceOffset = 0;
		buffer_copy_data.DestinationOffset = 0;
		buffer_copy_data.SourceBuffer = loadInfo->ScratchBuffer;
		buffer_copy_data.DestinationBuffer = deviceBuffer;
		buffer_copy_data.Size = onStaticMeshLoad.DataBuffer.Bytes();
		buffer_copy_data.Allocation = loadInfo->Allocation;
		loadInfo->RenderSystem->AddBufferCopy(buffer_copy_data);

		{
			RayTracingMesh mesh;
			mesh.IndexType = SelectIndexType(onStaticMeshLoad.IndexSize);
			mesh.IndicesCount = onStaticMeshLoad.IndexCount;
			mesh.IndicesOffset = onStaticMeshLoad.IndicesOffset;
			mesh.Buffer = deviceBuffer;
			mesh.Material = loadInfo->Material;

			AccelerationStructure::GeometryType geometryType;
			geometryType.Type = GeometryType::TRIANGLES;
			geometryType.IndexType = SelectIndexType(onStaticMeshLoad.IndexSize);
			geometryType.VertexType = ShaderDataType::FLOAT3;
			geometryType.MaxVertexCount = onStaticMeshLoad.VertexCount;
			geometryType.MaxPrimitiveCount = mesh.IndicesCount / 3;
			geometryType.AllowTransforms = false;

			AccelerationStructure::GeometryTriangleData triangleData;
			triangleData.VertexType = ShaderDataType::FLOAT3;
			triangleData.VertexStride = sizeof(GTSL::Vector3);
			triangleData.VertexBufferAddress = mesh.Buffer.GetAddress(loadInfo->RenderSystem->GetRenderDevice());
			triangleData.IndexType = SelectIndexType(onStaticMeshLoad.IndexSize);
			triangleData.IndexBufferAddress = triangleData.VertexBufferAddress + onStaticMeshLoad.IndicesOffset;
			
			AccelerationStructure::Geometry geometry;
			geometry.GeometryType = GeometryType::TRIANGLES;
			geometry.GeometryFlags = GeometryFlags::OPAQUE;
			geometry.GeometryTriangleData = &triangleData;

			AccelerationStructure::AccelerationStructureBuildOffsetInfo offset;
			offset.FirstVertex = 0;
			offset.PrimitiveCount = geometryType.MaxPrimitiveCount;
			offset.PrimitiveOffset = 0;
			offset.TransformOffset = 0;

			AccelerationStructure::BottomLevelCreateInfo accelerationStructureCreateInfo;
			accelerationStructureCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
			accelerationStructureCreateInfo.MaxGeometryCount = 1;
			accelerationStructureCreateInfo.Flags = AccelerationStructureFlags::PREFER_FAST_TRACE;
			accelerationStructureCreateInfo.GeometryInfos = GTSL::Ranger<AccelerationStructure::GeometryType>(1, &geometryType);

			mesh.AccelerationStructure.Initialize(accelerationStructureCreateInfo);

			RenderDevice::MemoryRequirements memoryRequirements;
			RenderDevice::GetAccelerationStructureMemoryRequirementsInfo accelerationStructureMemoryRequirements;
			accelerationStructureMemoryRequirements.MemoryRequirements = &memoryRequirements;
			accelerationStructureMemoryRequirements.AccelerationStructure = &mesh.AccelerationStructure;
			accelerationStructureMemoryRequirements.AccelerationStructureMemoryRequirementsType = GAL::VulkanAccelerationStructureMemoryRequirementsType::BUILD_SCRATCH;
			accelerationStructureMemoryRequirements.AccelerationStructureBuildType = GAL::VulkanAccelerationStructureBuildType::GPU_LOCAL;
			loadInfo->RenderSystem->GetRenderDevice()->GetAccelerationStructureMemoryRequirements(accelerationStructureMemoryRequirements);

			//USE MAX SIZE TO BUILD
			memoryRequirements.Size;
			
			staticMeshRenderGroup->rayTracingMeshes.EmplaceAt(loadInfo->InstanceId, mesh);
		}

		staticMeshRenderGroup->renderAllocations.EmplaceAt(loadInfo->InstanceId, loadInfo->Allocation);
		
		GTSL::Delete(loadInfo, staticMeshRenderGroup->GetPersistentAllocator());
	};

	taskInfo.GameInstance->AddFreeDynamicTask(GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad, StaticMeshRenderGroup*)>::Create(loadStaticMesh),
		GTSL::Array<TaskDependency, 2>{ {"StaticMeshRenderGroup", AccessType::READ_WRITE} }, GTSL::MoveRef(onStaticMeshLoad), this);
}
