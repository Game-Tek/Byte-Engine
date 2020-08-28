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
	RenderGroup::RenderInfo renderInfo;
	renderInfo.GameInstance = taskInfo.GameInstance;
	renderInfo.RenderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	renderInfo.MaterialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");

	auto& commandBuffer = *renderInfo.RenderSystem->GetCurrentCommandBuffer();

	auto positionMatrices = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetPositionMatrices();
	auto rotationMatrices = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetRotationMatrices();
	auto fovs = taskInfo.GameInstance->GetSystem<CameraSystem>("CameraSystem")->GetFieldOfViews();
	
	GTSL::Matrix4 projectionMatrix;
	GTSL::Math::BuildPerspectiveMatrix(projectionMatrix, fovs[0], 16.f / 9.f, 0.5f, 1000.f);
	
	auto pos = positionMatrices[0];
	
	pos(0, 3) *= -1;
	pos(1, 3) *= -1;
	
	auto viewMatrix = rotationMatrices[0] * pos;
	auto matrix = projectionMatrix * viewMatrix;
	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	auto& renderGroups = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem")->GetRenderGroups();
	
	BindingsManager<BE::TAR> bindingsManager(GetTransientAllocator(), renderInfo.RenderSystem, const_cast<CommandBuffer*>(renderInfo.RenderSystem->GetCurrentCommandBuffer()));
	
	bindingsManager.AddBinding(materialSystem->globalBindingsSets[renderInfo.RenderSystem->GetCurrentFrame()], PipelineType::GRAPHICS, materialSystem->globalPipelineLayout);
	
	GTSL::ForEach(renderGroups, [&](MaterialSystem::RenderGroupData& renderGroupData)
	{
		bindingsManager.AddBinding(renderGroupData.BindingsSets[renderInfo.RenderSystem->GetCurrentFrame()], PipelineType::GRAPHICS, renderGroupData.PipelineLayout);
	
		const auto renderGroup = taskInfo.GameInstance->GetSystem<StaticMeshRenderGroup>(renderGroupData.RenderGroupName);
	
		auto positions = renderGroup->GetPositions();
	
		uint32 offset = GTSL::Math::PowerOf2RoundUp(static_cast<uint32>(sizeof(GTSL::Matrix4)), GetRenderDevice()->GetMinUniformBufferOffset()) * GetCurrentFrame();
		const auto data_pointer = static_cast<byte*>(renderGroupData.Data) + offset;
	
		auto pos = GTSL::Math::Translation(positions[0]);
		pos(2, 3) *= -1.f;
		*reinterpret_cast<GTSL::Matrix4*>(data_pointer) = projectionMatrix * viewMatrix * pos;
		
		GTSL::ForEach(renderGroupData.Instances, [&](const MaterialSystem::MaterialInstance& materialInstance)
		{
			bindingsManager.AddBinding(materialInstance.BindingsSets[renderInfo.RenderSystem->GetCurrentFrame()], PipelineType::GRAPHICS, materialInstance.PipelineLayout);
			materialBind.Offsets = GTSL::Array<uint32, 1>{ static_cast<uint32>(GTSL::Math::PowerOf2RoundUp(materialInstance.DataSize, renderDevice.GetMinUniformBufferOffset()) * GetCurrentFrame()) }; //CHECK
			
			CommandBuffer::BindPipelineInfo bindPipelineInfo;
			bindPipelineInfo.RenderDevice = GetRenderDevice();
			bindPipelineInfo.PipelineType = PipelineType::GRAPHICS;
			bindPipelineInfo.Pipeline = &materialInstance.Pipeline;
			commandBuffer.BindPipeline(bindPipelineInfo);
	
			renderGroup->Render(taskInfo.GameInstance, this);
			
			bindingsManager.PopBindings();
		}
		);
	
		bindingsManager.PopBindings();
	}
	);
	
	//for(const auto e : systems)
	//{
	//	taskInfo.GameInstance->GetSystem<RenderGroup>(e)->Render(renderInfo);
	//}
}

void RenderOrchestrator::AddRenderGroup(GameInstance* gameInstance, Id renderGroupName, RenderGroup* renderGroup)
{
	systems.EmplaceBack(renderGroupName);
	systemsAccesses.EmplaceBack(renderGroup->GetRenderDependencies());
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderFinished");

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
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderFinished");

	GTSL::Array<TaskDependency, 32> dependencies(systems.GetLength());
	{
		uint32 i = 0;

		for(uint32 j = 0; j < dependencies.GetLength(); ++j)
		{
			for(uint32 k = 0; k < systemsAccesses[j].GetLength(); ++k)
			{
				dependencies[i] = systemsAccesses[j][k];
				++i;
			}
		}
	}

	dependencies.EmplaceBack("RenderSystem", AccessType::READ);
	
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderSetup", "RenderFinished");
}
