#include "RenderOrchestrator.h"


#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Matrix4.h>


#include "RenderGroup.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Game/Tasks.h"
#include <ByteEngine\Render\BindingsManager.hpp>

#include "MaterialSystem.h"
#include "StaticMeshRenderGroup.h"
#include "ByteEngine/Game/CameraSystem.h"

void RenderOrchestrator::Initialize(const InitializeInfo& initializeInfo)
{
	systems.Initialize(32, GetPersistentAllocator());
	
	{
		const GTSL::Array<TaskDependency, 4> dependencies{ { CLASS_NAME, AccessType::READ_WRITE } };
		initializeInfo.GameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderSetup", "RenderFinished");
	}
}

void RenderOrchestrator::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

void RenderOrchestrator::Render(TaskInfo taskInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto& commandBuffer = *renderSystem->GetCurrentCommandBuffer();
	uint8 currentFrame = renderSystem->GetCurrentFrame();
	
	BindingsManager<BE::TAR> bindingsManager(GetTransientAllocator(), renderSystem, renderSystem->GetCurrentCommandBuffer());
	
	auto positionMatrices = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetPositionMatrices();
	auto rotationMatrices = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetRotationMatrices();
	auto fovs = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetFieldOfViews();
	
	GTSL::Matrix4 projectionMatrix;
	GTSL::Math::BuildPerspectiveMatrix(projectionMatrix, fovs[0], 16.f / 9.f, 0.5f, 1000.f);
	
	auto cameraPosition = positionMatrices[0];
	
	cameraPosition(0, 3) *= -1;
	cameraPosition(1, 3) *= -1;
	
	auto viewMatrix = rotationMatrices[0] * cameraPosition;
	auto matrix = projectionMatrix * viewMatrix;
	
	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto& renderGroups = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem")->GetRenderGroups();
	
	bindingsManager.AddBinding(materialSystem->globalBindingsSets[currentFrame], PipelineType::RASTER, materialSystem->globalPipelineLayout);
	
	GTSL::PairForEach(renderGroups, [&](uint64 renderGroupKey, MaterialSystem::RenderGroupData& renderGroupData)
	{
		uint32 offset = GTSL::Math::PowerOf2RoundUp(sizeof(GTSL::Matrix4), static_cast<uint64>(renderSystem->GetRenderDevice()->GetMinUniformBufferOffset())) * currentFrame;
		auto* const data_pointer = static_cast<byte*>(renderGroupData.Data) + offset;

		auto renderGroupOffsets = GTSL::Array<uint32, 1>{ GTSL::Math::PowerOf2RoundUp(renderGroupData.DataSize, renderSystem->GetRenderDevice()->GetMinUniformBufferOffset()) * currentFrame };
		bindingsManager.AddBinding(renderGroupData.BindingsSets[currentFrame], renderGroupOffsets, PipelineType::RASTER, renderGroupData.PipelineLayout);

		auto* const renderGroup = taskInfo.GameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
	
		auto positions = renderGroup->GetPositions();
	
		auto pos = GTSL::Math::Translation(positions[0]);
		pos(2, 3) *= -1.f;
		*reinterpret_cast<GTSL::Matrix4*>(data_pointer) = projectionMatrix * viewMatrix * pos;
		
		GTSL::PairForEach(renderGroupData.Instances, [&](const uint64 materialKey, const MaterialSystem::MaterialInstance& materialInstance)
		{
			auto materialOffsets = GTSL::Array<uint32, 1>{ GTSL::Math::PowerOf2RoundUp(materialInstance.DataSize, renderSystem->GetRenderDevice()->GetMinUniformBufferOffset()) * currentFrame };
			bindingsManager.AddBinding(materialInstance.BindingsSets[currentFrame], materialOffsets, PipelineType::RASTER, materialInstance.PipelineLayout);

			if (materialSystem->IsMaterialReady(renderGroupKey, materialKey))
			{				
				CommandBuffer::BindPipelineInfo bindPipelineInfo;
				bindPipelineInfo.RenderDevice = renderSystem->GetRenderDevice();
				bindPipelineInfo.PipelineType = PipelineType::RASTER;
				bindPipelineInfo.Pipeline = &materialInstance.Pipeline;
				commandBuffer.BindPipeline(bindPipelineInfo);

				for (const auto& e : renderGroup->GetMeshes())
				{
					CommandBuffer::BindVertexBufferInfo bindVertexInfo;
					bindVertexInfo.RenderDevice = renderSystem->GetRenderDevice();
					bindVertexInfo.Buffer = &e.Buffer;
					bindVertexInfo.Offset = 0;
					renderSystem->GetCurrentCommandBuffer()->BindVertexBuffer(bindVertexInfo);

					CommandBuffer::BindIndexBufferInfo bindIndexBuffer;
					bindIndexBuffer.RenderDevice = renderSystem->GetRenderDevice();
					bindIndexBuffer.Buffer = &e.Buffer;
					bindIndexBuffer.Offset = e.IndicesOffset;
					bindIndexBuffer.IndexType = e.IndexType;
					renderSystem->GetCurrentCommandBuffer()->BindIndexBuffer(bindIndexBuffer);

					CommandBuffer::DrawIndexedInfo drawIndexedInfo;
					drawIndexedInfo.RenderDevice = renderSystem->GetRenderDevice();
					drawIndexedInfo.InstanceCount = 1;
					drawIndexedInfo.IndexCount = e.IndicesCount;
					renderSystem->GetCurrentCommandBuffer()->DrawIndexed(drawIndexedInfo);
				}
			}
			
			bindingsManager.PopBindings();
		}
		);
	
		bindingsManager.PopBindings();
	}
	);
}

void RenderOrchestrator::AddRenderGroup(GameInstance* gameInstance, Id renderGroupName, RenderGroup* renderGroup)
{
	systems.EmplaceBack(renderGroupName);
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderSetup");

	GTSL::Array<TaskDependency, 32> dependencies(systems.GetLength());
	{
		for (uint32 i = 0; i < dependencies.GetLength(); ++i)
		{
			dependencies[i].AccessedObject = systems[i];
			dependencies[i].Access = AccessType::READ;
		}
	}

	dependencies.EmplaceBack("RenderSystem", AccessType::READ);

	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderSetup", "RenderFinished");
}

void RenderOrchestrator::RemoveRenderGroup(GameInstance* gameInstance, const Id renderGroupName)
{
	const auto element = systems.Find(renderGroupName);
	BE_ASSERT(element != systems.end())
	
	systems.Pop(element - systems.begin());
	systemsAccesses.Pop(element - systems.begin());
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderSetup");

	GTSL::Array<TaskDependency, 32> dependencies(systems.GetLength());
	{
		for(uint32 i = 0; i < dependencies.GetLength(); ++i)
		{
			dependencies[i].AccessedObject = systems[i];
			dependencies[i].Access = AccessType::READ;
		}
	}

	dependencies.EmplaceBack("RenderSystem", AccessType::READ);
	
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderSetup", "RenderFinished");
}
