#pragma once

#include "RenderSystem.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include <ByteEngine/Render/RenderOrchestrator.h>

#include "LightsRenderGroup.h"

class StaticMeshRenderGroup;
class StaticMeshResouceManager;

class WorldRendererPipeline : public RenderPipeline {
public:
	MAKE_BE_HANDLE(Instance)

		WorldRendererPipeline(const InitializeInfo& initialize_info);

	void onAddShaderGroup(RenderOrchestrator* render_orchestrator, RenderSystem* render_system) {
		++shaderGroupCount;

		if (render_orchestrator->tag == GTSL::ShortString<16>(u8"Visibility")) {
			auto bwk = render_orchestrator->GetBufferWriteKey(render_system, visibilityDataKey);
			bwk[u8"shaderGroupLength"] = shaderGroupCount;
		}
	}

private:
	DECLARE_BE_TYPE(Instance)

	DECLARE_BE_TASK(OnAddRenderGroupMesh, BE_RESOURCES(StaticMeshResourceManager*, RenderOrchestrator*, RenderSystem*, StaticMeshRenderGroup*), StaticMeshRenderGroup::StaticMeshHandle, Id);
	DECLARE_BE_TASK(OnUpdateRenderGroupMesh, BE_RESOURCES(RenderSystem*, RenderOrchestrator*), StaticMeshRenderGroup::StaticMeshHandle, GTSL::Matrix3x4);

	DECLARE_BE_TASK(OnAddMesh, BE_RESOURCES(StaticMeshResourceManager*, RenderOrchestrator*, RenderSystem*), InstanceHandle, Id);
	DECLARE_BE_TASK(OnUpdateMesh, BE_RESOURCES(RenderSystem*, RenderOrchestrator*), InstanceHandle, GTSL::Matrix3x4);

	TaskHandle<StaticMeshResourceManager::StaticMeshInfo> onStaticMeshLoadHandle;
	TaskHandle<StaticMeshResourceManager::StaticMeshInfo> onStaticMeshInfoLoadHandle;

	TaskHandle<StaticMeshRenderGroup::StaticMeshHandle, Id, RenderModelHandle> OnAddInfiniteLight;

	TaskHandle<StaticMeshRenderGroup::StaticMeshHandle, Id, RenderModelHandle> OnAddBackdrop;
	TaskHandle<StaticMeshRenderGroup::StaticMeshHandle, Id, RenderModelHandle> OnAddParticleSystem;
	TaskHandle<StaticMeshRenderGroup::StaticMeshHandle, Id, RenderModelHandle> OnAddVolume;
	TaskHandle<StaticMeshRenderGroup::StaticMeshHandle, Id, RenderModelHandle> OnAddSkinnedMesh;

	uint32 shaderGroupCount = 0;
	RenderOrchestrator::NodeHandle staticMeshRenderGroup;

	GTSL::MultiVector<BE::PAR, false, float32, float32, float32, float32> spherePositionsAndRadius;
	GTSL::StaticVector<AABB, 8> aabss;
	
	bool rayTracing = false;
	RenderSystem::AccelerationStructureHandle topLevelAccelerationStructure;
	RenderOrchestrator::NodeHandle vertexBufferNodeHandle, indexBufferNodeHandle, meshDataNode;
	RenderOrchestrator::NodeHandle mainVisibilityPipelineNode;
	RenderOrchestrator::DataKeyHandle visibilityDataKey, lightsDataKey;

	struct Mesh {
		RenderModelHandle MaterialHandle;
		RenderSystem::BLASInstanceHandle InstanceHandle;
	};
	GTSL::FixedVector<Mesh, BE::PAR> instances;

	GTSL::HashMap<StaticMeshRenderGroup::StaticMeshHandle, InstanceHandle, BE::PAR> meshToInstanceMap;

	RenderOrchestrator::DataKeyHandle meshDataBuffer;

	struct Resource {
		GTSL::StaticVector<GTSL::StaticVector<GAL::ShaderDataType, 8>, 8> VertexElements;
		GTSL::StaticVector<InstanceHandle, 8> Instances;
		bool Loaded = false;
		uint32 VertexComponentsInStream = 0, IndicesInStream = 0;
		uint32 VertexSize, VertexCount = 0, IndexCount = 0;
		GAL::IndexType IndexType;
		RenderSystem::AccelerationStructureHandle BLAS;
		GTSL::Vector3 ScalingFactor = GTSL::Vector3(1.0f);
		bool Interleaved = true;
		RenderOrchestrator::NodeHandle nodeHandle;
		RenderModelHandle RenderModelHandle;
	};
	GTSL::HashMap<Id, Resource, BE::PAR> resources;

	GTSL::StaticVector<RenderSystem::AccelerationStructureHandle, 32> pendingBlasUpdates;
	GTSL::StaticVector<RenderSystem::AccelerationStructureHandle, 32> pendingAdditions;

	RenderSystem::BufferHandle sourceVertexBuffer, destinationVertexBuffer, sourceIndexBuffer, destinationIndexBuffer;
	uint32 vertexComponentsPerStream = 0, indicesInBuffer = 0;

	RenderOrchestrator::NodeHandle visibilityRenderPassNodeHandle, lightingDataNodeHandle;

	static uint32 calculateMeshSize(const uint32 vertexCount, const uint32 vertexSize, const uint32 indexCount, const uint32 indexSize) {
		return GTSL::Math::RoundUpByPowerOf2(vertexCount * vertexSize, 16) + indexCount * indexSize;
	}

	//GTSL::StaticVector<uint32, 16> prefixSum; GTSL::StaticVector<Id, 16> prefixSumGuide;

	void onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo);

	void onStaticMeshLoaded(TaskInfo taskInfo, RenderSystem* render_system, StaticMeshRenderGroup* render_group, RenderOrchestrator* render_orchestrator, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo);

	void OnAddRenderGroupMesh(TaskInfo task_info, StaticMeshResourceManager* static_mesh_resource_manager, RenderOrchestrator* render_orchestrator, RenderSystem* render_system, StaticMeshRenderGroup* static_mesh_render_group, StaticMeshRenderGroup::StaticMeshHandle static_mesh_handle, Id resourceName);

	void OnAddMesh(TaskInfo task_info, StaticMeshResourceManager* static_mesh_resource_manager, RenderOrchestrator* render_orchestrator, RenderSystem* render_system, InstanceHandle instance_handle, Id resourceName) {
		auto& instance = instances[instance_handle()];
		auto& resource = resources[resourceName];

		render_orchestrator->AddInstance(meshDataNode, resource.nodeHandle, instance_handle);

		auto key = render_orchestrator->GetBufferWriteKey(render_system, meshDataBuffer);

		instance.MaterialHandle = resource.RenderModelHandle;

		const uint32 instanceIndex = render_orchestrator->GetInstanceIndex(meshDataNode, instance_handle);

		key[instanceIndex][u8"vertexBufferOffset"] = resource.VertexComponentsInStream; key[instanceIndex][u8"indexBufferOffset"] = resource.IndicesInStream;
		render_orchestrator->SubscribeToUpdate(render_orchestrator->GetShaderGroupIndexUpdateKey(instance.MaterialHandle), key[instanceIndex][u8"shaderGroupIndex"], meshDataBuffer);
		key[instanceIndex][u8"transform"] = GTSL::Matrix3x4();

		if(rayTracing) {
			instance.InstanceHandle = render_system->AddBLASToTLAS(topLevelAccelerationStructure, resource.BLAS, instanceIndex, instance.InstanceHandle);
		}
	}

	void AddMeshInstance(Id resource_name, InstanceHandle instance_handle) {		
		GetApplicationManager()->EnqueueTask(OnAddMeshTaskHandle, GTSL::MoveRef(instance_handle), GTSL::MoveRef(resource_name)); //Signal can update
	}

	void OnUpdateRenderGroupMesh(TaskInfo, RenderSystem* renderSystem, RenderOrchestrator* render_orchestrator, StaticMeshRenderGroup::StaticMeshHandle static_mesh_handle, GTSL::Matrix3x4 transform) {
		auto instanceHandle = meshToInstanceMap.At(static_mesh_handle);
		GetApplicationManager()->EnqueueTask(OnUpdateMeshTaskHandle, GTSL::MoveRef(instanceHandle), GTSL::MoveRef(transform));
	}

	void OnUpdateMesh(TaskInfo, RenderSystem* renderSystem, RenderOrchestrator* render_orchestrator, InstanceHandle instance_handle, GTSL::Matrix3x4 transform) {
		auto key = render_orchestrator->GetBufferWriteKey(renderSystem, meshDataBuffer);

		const auto& instance = instances[instance_handle()];

		const auto instanceIndex = render_orchestrator->GetInstanceIndex(meshDataNode, instance_handle);

		key[instanceIndex][u8"transform"] = transform;
		//*spherePositionsAndRadius.GetPointer<0>(instanceIndex) = transform(0, 3);
		//*spherePositionsAndRadius.GetPointer<1>(instanceIndex) = transform(1, 3);
		//*spherePositionsAndRadius.GetPointer<2>(instanceIndex) = transform(2, 3);

		if (rayTracing) {
			renderSystem->SetInstancePosition(topLevelAccelerationStructure, instance.InstanceHandle, transform);
		}
	}

	uint32 lights = 0;

	void onAddLight(TaskInfo, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, LightsRenderGroup::PointLightHandle light_handle) {
		auto bwk = render_orchestrator->GetBufferWriteKey(render_system, lightingDataNodeHandle);
		bwk[u8"pointLightsLength"] = ++lights;
		//bwk[u8"pointLights"][light_handle()][u8"position"] = GTSL::Vector3(0, 0, 0);
		//bwk[u8"pointLights"][light_handle()][u8"color"] = GTSL::Vector3(1, 1, 1);
		//bwk[u8"pointLights"][light_handle()][u8"intensity"] = 5.f;
	}

	void updateLight(const TaskInfo, RenderSystem* render_system, RenderOrchestrator* render_orchestrator, LightsRenderGroup::PointLightHandle light_handle, GTSL::Vector3 position, GTSL::RGB color, float32 intensity) {
		auto bwk = render_orchestrator->GetBufferWriteKey(render_system, lightingDataNodeHandle);
		bwk[u8"pointLights"][light_handle()][u8"position"] = position;
		bwk[u8"pointLights"][light_handle()][u8"color"] = color;
		bwk[u8"pointLights"][light_handle()][u8"intensity"] = intensity;
	}

	void preRender(TaskInfo, RenderSystem* render_system, RenderOrchestrator* render_orchestrator) {
		////GTSL::Vector<float32, BE::TAR> results(GetTransientAllocator());
		////projectSpheres({0}, spherePositionsAndRadius, results);
		//
		//{ // Add BLAS instances to TLAS only if dependencies were fulfilled
		//	auto i = 0;
		//
		//	while (i < pendingAdditions) {
		//		const auto& addition = pendingAdditions[i];
		//		auto e = addition.Second;
		//		auto& mesh = instances[e()];
		//
		//		mesh.InstanceHandle = render_system->AddBLASToTLAS(topLevelAccelerationStructure, resources[addition.First].BLAS, e(), mesh.InstanceHandle);
		//
		//		pendingAdditions.Pop(i);
		//		++i;
		//	}
		//}
		
		
		auto workloadHandle = render_orchestrator->buildAccelerationStructuresWorkloadHandle[render_system->GetCurrentFrame()];
		render_system->Wait(workloadHandle);
		render_system->StartCommandList(render_orchestrator->buildCommandList[render_system->GetCurrentFrame()]);
		
		if (rayTracing) {
			render_system->DispatchBuild(render_orchestrator->buildCommandList[render_system->GetCurrentFrame()], pendingBlasUpdates);
			pendingBlasUpdates.Resize(0);
			render_system->DispatchBuild(render_orchestrator->buildCommandList[render_system->GetCurrentFrame()], { topLevelAccelerationStructure }); //Update TLAS
		}
		
		render_system->EndCommandList(render_orchestrator->buildCommandList[render_system->GetCurrentFrame()]);
		render_system->Submit(GAL::QueueTypes::COMPUTE, { { { render_orchestrator->buildCommandList[render_system->GetCurrentFrame()] }, {  }, { workloadHandle } } }, workloadHandle);
	}

	void terrain() {
		struct TerrainVertex {
			GTSL::Vector3 position; GTSL::RGBA color;
		};

		GTSL::Extent3D terrainExtent{ 256, 1, 256 };

		uint32 vertexCount = (terrainExtent.Width - 1) * (terrainExtent.Depth - 1) * 8;
		uint32 indexCount = vertexCount;

		TerrainVertex* vertices = nullptr; uint16* indices = nullptr;

		// Initialize the index into the vertex and index arrays.
		uint32 index = 0;

		GTSL::RGBA color; uint32 m_terrainWidth; GTSL::Vector3* m_terrainModel = nullptr, * m_heightMap = nullptr;

		// Load the vertex array and index array with data.
		for (uint32 j = 0; j < (terrainExtent.Depth - 1); j++) {
			for (uint32 i = 0; i < (terrainExtent.Width - 1); i++) {
				// Get the indexes to the four points of the quad.
				uint32 index1 = (m_terrainWidth * j) + i;          // Upper left.
				uint32 index2 = (m_terrainWidth * j) + (i + 1);      // Upper right.
				uint32 index3 = (m_terrainWidth * (j + 1)) + i;      // Bottom left.
				uint32 index4 = (m_terrainWidth * (j + 1)) + (i + 1);  // Bottom right.

				// Now create two triangles for that quad.
				// Triangle 1 - Upper left.
				m_terrainModel[index].X() = m_heightMap[index1].X();
				m_terrainModel[index].Y() = m_heightMap[index1].Y();
				m_terrainModel[index].Z() = m_heightMap[index1].Z();
				index++;

				// Triangle 1 - Upper right.
				m_terrainModel[index].X() = m_heightMap[index2].X();
				m_terrainModel[index].Y() = m_heightMap[index2].Y();
				m_terrainModel[index].Z() = m_heightMap[index2].Z();
				index++;

				// Triangle 1 - Bottom left.
				m_terrainModel[index].X() = m_heightMap[index3].X();
				m_terrainModel[index].Y() = m_heightMap[index3].Y();
				m_terrainModel[index].Z() = m_heightMap[index3].Z();
				index++;

				// Triangle 2 - Bottom left.
				m_terrainModel[index].X() = m_heightMap[index3].X();
				m_terrainModel[index].Y() = m_heightMap[index3].Y();
				m_terrainModel[index].Z() = m_heightMap[index3].Z();
				index++;

				// Triangle 2 - Upper right.
				m_terrainModel[index].X() = m_heightMap[index2].X();
				m_terrainModel[index].Y() = m_heightMap[index2].Y();
				m_terrainModel[index].Z() = m_heightMap[index2].Z();
				index++;

				// Triangle 2 - Bottom right.
				m_terrainModel[index].X() = m_heightMap[index4].X();
				m_terrainModel[index].Y() = m_heightMap[index4].Y();
				m_terrainModel[index].Z() = m_heightMap[index4].Z();
				index++;
			}
		}
	}

	void setupDirectionShadowRenderPass(RenderSystem* renderSystem, RenderOrchestrator* renderOrchestrator) {
		renderOrchestrator->RegisterType(u8"global", u8"TraceRayParameterData", TRACE_RAY_PARAMETER_DATA);

		// Global Data
		// RenderPassData
		// CameraData
		// StaticMeshesData
		// RayTraceData

		// Make render pass
		RenderOrchestrator::PassData pass_data;
		pass_data.PassType = RenderOrchestrator::PassType::RAY_TRACING;
		pass_data.Attachments.EmplaceBack(GTSL::StringView(u8"Color"), GAL::AccessTypes::WRITE);
		pass_data.Attachments.EmplaceBack(GTSL::StringView(u8"Position"), GAL::AccessTypes::READ);
		pass_data.Attachments.EmplaceBack(GTSL::StringView(u8"Depth"), GAL::AccessTypes::READ);

		RenderOrchestrator::NodeHandle chain = renderOrchestrator->GetGlobalDataLayer();

		chain = renderOrchestrator->AddRenderPassNode(chain, u8"DirectionalShadow", renderSystem, pass_data);

		// Create shader group
		auto rayTraceShaderGroupHandle = renderOrchestrator->CreateShaderGroup(Id(u8"DirectionalShadow"));
		// Add dispatch
		chain = renderOrchestrator->AddDataNode(chain, u8"CameraData", renderOrchestrator->cameraDataKeyHandle);
		chain = renderOrchestrator->AddDataNode(chain, u8"InstancesData", meshDataBuffer);
		chain = renderOrchestrator->AddDataNode(chain, u8"LightingData", lightsDataKey); //lighting data
		chain = renderOrchestrator->addPipelineBindNode(chain, rayTraceShaderGroupHandle);


		auto dataKeyHandle = renderOrchestrator->MakeDataKey(renderSystem, u8"global", u8"TraceRayParameterData");

		chain = renderOrchestrator->AddDataNode(chain, u8"RayTraceData", dataKeyHandle);

		renderOrchestrator->AddRayTraceNode(chain, rayTraceShaderGroupHandle);

		auto bwk = renderOrchestrator->GetBufferWriteKey(renderSystem, dataKeyHandle);
		bwk[u8"accelerationStructure"] = topLevelAccelerationStructure;
		bwk[u8"rayFlags"] = 0u;
		bwk[u8"recordOffset"] = 0u;
		bwk[u8"recordStride"] = 0u;
		bwk[u8"missIndex"] = 0u;
		bwk[u8"tMin"] = 0.008f;
		bwk[u8"tMax"] = 100.0f;
	}
};