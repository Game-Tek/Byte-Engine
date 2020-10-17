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
		bufferCreateInfo.Name = name;
	}
	
	bufferCreateInfo.Size = bufferSize;
	bufferCreateInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer;
	
	HostRenderAllocation allocation;
	
	Buffer::GetMemoryRequirementsInfo memoryAllocationInfo;
	memoryAllocationInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	memoryAllocationInfo.CreateInfo = &bufferCreateInfo;
	scratch_buffer.GetMemoryRequirements(&memoryAllocationInfo);

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

	//TODO: DO ALLOCATION
	
	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index, addStaticMeshInfo.Material);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = Task<StaticMeshResourceManager::OnStaticMeshLoad>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Range<byte*>(bufferSize, static_cast<byte*>(allocation.Data));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = indexSize;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName);
	positions.EmplaceAt(index);

	++meshCount;
	
	return ComponentReference(GetSystemId(), index);
}

ComponentReference StaticMeshRenderGroup::AddRayTracedStaticMesh(const AddRayTracedStaticMeshInfo& addStaticMeshInfo)
{
	uint32 bufferSize = 0, indicesOffset = 0; uint16 indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &indexSize, &indexSize, &bufferSize, &indicesOffset);

	Buffer::CreateInfo bufferCreateInfo;
	bufferCreateInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Buffer. StaticMesh: "); name += addStaticMeshInfo.MeshName.GetHash();
		bufferCreateInfo.Name = name;
	}

	bufferCreateInfo.Size = bufferSize;
	bufferCreateInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer;

	HostRenderAllocation allocation;

	Buffer::GetMemoryRequirementsInfo memoryAllocationInfo;
	memoryAllocationInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	memoryAllocationInfo.CreateInfo = &bufferCreateInfo;
	scratch_buffer.GetMemoryRequirements(&memoryAllocationInfo);

	uint32 index = positions.GetFirstFreeIndex().Get();

	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index, addStaticMeshInfo.Material);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };

	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onRayTracedStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Range<byte*>(bufferSize, static_cast<byte*>(allocation.Data));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = indexSize;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName);
	positions.EmplaceAt(index);

	return ComponentReference(GetSystemId(), index);
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
			createInfo.Name = name;
		}

		createInfo.Size = onStaticMeshLoad.DataBuffer.Bytes();
		createInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_DESTINATION;
		Buffer deviceBuffer;

		{
			RenderSystem::BufferLocalMemoryAllocationInfo memoryAllocationInfo;
			memoryAllocationInfo.CreateInfo = &createInfo;
			memoryAllocationInfo.Buffer = &deviceBuffer;
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

	taskInfo.GameInstance->AddFreeDynamicTask(Task<StaticMeshResourceManager::OnStaticMeshLoad, StaticMeshRenderGroup*>::Create(loadStaticMesh),
		GTSL::Array<TaskDependency, 2>{ {"StaticMeshRenderGroup", AccessType::READ_WRITE} }, GTSL::MoveRef(onStaticMeshLoad), this);
}

void StaticMeshRenderGroup::onRayTracedStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	auto loadStaticMesh = [](TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad, StaticMeshRenderGroup* staticMeshRenderGroup)
	{
		MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

		RenderSystem::CreateRayTracingMeshInfo meshInfo;
		meshInfo.SourceBuffer = loadInfo->ScratchBuffer;
		meshInfo.SourceAllocation = loadInfo->Allocation;
		meshInfo.Vertices = onStaticMeshLoad.VertexCount;
		meshInfo.IndexCount = onStaticMeshLoad.IndexCount;
		meshInfo.IndicesOffset = onStaticMeshLoad.IndicesOffset;
		meshInfo.IndexType = SelectIndexType(onStaticMeshLoad.IndexSize);

		GTSL::Matrix3x4 matrix;
		meshInfo.Matrix = &matrix;
		loadInfo->RenderSystem->CreateRayTracedMesh(meshInfo);
		
		GTSL::Delete(loadInfo, staticMeshRenderGroup->GetPersistentAllocator());
	};

	taskInfo.GameInstance->AddFreeDynamicTask(Task<StaticMeshResourceManager::OnStaticMeshLoad, StaticMeshRenderGroup*>::Create(loadStaticMesh),
		GTSL::Array<TaskDependency, 2>{ {"StaticMeshRenderGroup", AccessType::READ_WRITE} }, GTSL::MoveRef(onStaticMeshLoad), this);
}
