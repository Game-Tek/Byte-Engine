#include "RenderOrchestrator.h"

#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Matrix4.h>

#include "RenderGroup.h"
#include "ByteEngine/Game/GameInstance.h"
#include "ByteEngine/Game/Tasks.h"
#include <ByteEngine\Render\BindingsManager.hpp>


#include "FrameManager.h"
#include "MaterialSystem.h"
#include "StaticMeshRenderGroup.h"
#include "TextSystem.h"
#include "ByteEngine/Game/CameraSystem.h"

struct StaticMeshRenderManager : RenderOrchestrator::RenderManager
{
	void Render(const RenderInfo& renderInfo) override
	{
		if (renderInfo.RenderPass == 0 && renderInfo.SubPass == 0)
		{
			auto* const renderGroup = renderInfo.GameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");

			auto renderGroupName = Id("StaticMeshRenderGroup");

			auto& renderGroups = renderInfo.MaterialSystem->GetRenderGroups();
			auto& renderGroupInstance = renderGroups.At(renderGroupName);

			{
				auto offset = GTSL::Array<uint32, 1>{ 64u * renderInfo.CurrentFrame };
				renderInfo.BindingsManager->AddBinding(renderGroupInstance.BindingsSets[renderInfo.CurrentFrame], offset, PipelineType::RASTER, renderGroupInstance.PipelineLayout);
			}

			auto& materialInstances = renderInfo.MaterialSystem->GetMaterialInstances();
			auto& materialInstance = materialInstances[0];

			if (materialInstance.BindingsSets.GetLength())
			{
				auto materialOffsets = GTSL::Array<uint32, 1>{ 64u * renderInfo.CurrentFrame };
				renderInfo.BindingsManager->AddBinding(materialInstance.BindingsSets[renderInfo.CurrentFrame], materialOffsets, PipelineType::RASTER, materialInstance.PipelineLayout);
			}

			if (renderInfo.MaterialSystem->IsMaterialReady(0))
			{
				CommandBuffer::BindPipelineInfo bindPipelineInfo;
				bindPipelineInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
				bindPipelineInfo.PipelineType = PipelineType::RASTER;
				bindPipelineInfo.Pipeline = &materialInstance.Pipeline;
				renderInfo.CommandBuffer->BindPipeline(bindPipelineInfo);
				for (const auto& e : renderGroup->GetMeshes())
				{
					CommandBuffer::BindVertexBufferInfo bindVertexInfo;
					bindVertexInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
					bindVertexInfo.Buffer = &e.Buffer;
					bindVertexInfo.Offset = 0;
					renderInfo.CommandBuffer->BindVertexBuffer(bindVertexInfo);
					CommandBuffer::BindIndexBufferInfo bindIndexBuffer;
					bindIndexBuffer.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
					bindIndexBuffer.Buffer = &e.Buffer;
					bindIndexBuffer.Offset = e.IndicesOffset;
					bindIndexBuffer.IndexType = e.IndexType;
					renderInfo.CommandBuffer->BindIndexBuffer(bindIndexBuffer);
					CommandBuffer::DrawIndexedInfo drawIndexedInfo;
					drawIndexedInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
					drawIndexedInfo.InstanceCount = 1;
					drawIndexedInfo.IndexCount = e.IndicesCount;
					renderInfo.CommandBuffer->DrawIndexed(drawIndexedInfo);
				}
			}

			if (materialInstance.BindingsSets.GetLength())
			{
				renderInfo.BindingsManager->PopBindings(); //material
			}

			renderInfo.BindingsManager->PopBindings(); //render group
		}
	}

	void Setup(const SetupInfo& info) override
	{
		uint32 offset = GTSL::Math::PowerOf2RoundUp(sizeof(GTSL::Matrix4), static_cast<uint64>(info.RenderSystem->GetRenderDevice()->GetMinUniformBufferOffset())) * info.RenderSystem->GetCurrentFrame();

		auto* data = info.MaterialSystem->GetRenderGroupDataPointer("StaticMeshRenderGroup");
		
		auto* const renderGroup = info.GameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
		auto positions = renderGroup->GetPositions();
		auto pos = GTSL::Math::Translation(positions[0]);
		pos(2, 3) *= -1.f;
		*reinterpret_cast<GTSL::Matrix4*>(static_cast<byte*>(data) + offset) = info.ProjectionMatrix * info.ViewMatrix * pos;
		
		MaterialSystem::UpdateRenderGroupDataInfo updateInfo;
		updateInfo.RenderGroup = "StaticMeshRenderGroup";
		updateInfo.Data = GTSL::Ranger<const byte>(64, static_cast<const byte*>(data));
		updateInfo.Offset = 64;
		info.MaterialSystem->UpdateRenderGroupData(updateInfo);
	}
};

struct TextRenderManager : RenderOrchestrator::RenderManager
{
	void Render(const RenderInfo& renderInfo) override
	{
		if (renderInfo.RenderPass == 0 && renderInfo.SubPass == 1)
		{
			Id renderGroupName = "TextSystem";

			auto& renderGroups = renderInfo.MaterialSystem->GetRenderGroups();
			auto& renderGroupInstance = renderGroups.At(renderGroupName);

			{
				auto offset = GTSL::Array<uint32, 1>{ 0 };
				renderInfo.BindingsManager->AddBinding(renderGroupInstance.BindingsSets[renderInfo.CurrentFrame], offset, PipelineType::RASTER, renderGroupInstance.PipelineLayout);
			}

			auto& materialInstances = renderInfo.MaterialSystem->GetMaterialInstances();
			auto& materialInstance = materialInstances[1];

			if (renderInfo.MaterialSystem->IsMaterialReady(1))
			{
				CommandBuffer::BindPipelineInfo bindPipelineInfo;
				bindPipelineInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
				bindPipelineInfo.PipelineType = PipelineType::RASTER;
				bindPipelineInfo.Pipeline = &materialInstance.Pipeline;
				renderInfo.CommandBuffer->BindPipeline(bindPipelineInfo);

				auto* textSystem = renderInfo.GameInstance->GetSystem<TextSystem>("TextSystem");
				auto& text = textSystem->GetTexts()[0];
				auto& glyph = textSystem->GetRenderingFont().Glyphs.at(textSystem->GetRenderingFont().GlyphMap.at(text.String[0]));

				CommandBuffer::DrawInfo drawInfo;
				drawInfo.FirstInstance = 0;
				drawInfo.FirstVertex = 0;
				drawInfo.InstanceCount = glyph.NumTriangles;
				drawInfo.VertexCount = 3;
				renderInfo.CommandBuffer->Draw(drawInfo);
			}

			renderInfo.BindingsManager->PopBindings();
		}
	}

	void Setup(const SetupInfo& info) override
	{
		auto textSystem = info.GameInstance->GetSystem<TextSystem>("TextSystem");
		
		auto& text = textSystem->GetTexts()[0];
		auto& glyph = const_cast<FontResourceManager::Glyph&>(textSystem->GetRenderingFont().Glyphs.at(textSystem->GetRenderingFont().GlyphMap.at(text.String[0])));

		byte* data = static_cast<byte*>(info.MaterialSystem->GetRenderGroupDataPointer("TextSystem"));

		GTSL::Vector4 windingPoint(-glyph.BoundingBox[0], -glyph.BoundingBox[3], 0, 1);

		//vec2 positions[3] = vec2[](
		//	vec2(-0.5, 0.5),
		//	vec2(0.5, 0.5),
		//	vec2(0.0, -0.5)
		//	);
		
		//glyph.PathList[0].Curves[0].p0.X = -0.5;
		//glyph.PathList[0].Curves[0].p0.Y = 0.5;
		//glyph.PathList[0].Curves[0].p1.X = 0.5;
		//glyph.PathList[0].Curves[0].p1.Y = 0.5;
		//glyph.PathList[0].Curves[0].p2.X = 0.0;
		//glyph.PathList[0].Curves[0].p2.Y = -0.5;
		
		//curve is aligned to glsl std140
		GTSL::MemCopy(sizeof(GTSL::Vector4), &windingPoint, data); data += sizeof(GTSL::Vector4);
		GTSL::MemCopy(glyph.PathList[0].Curves.GetLengthSize(), glyph.PathList[0].Curves.GetData(), data);	

		//MaterialSystem::UpdateRenderGroupDataInfo updateInfo;
		//updateInfo.RenderGroup = "TextSystem";
		//updateInfo.Data = GTSL::Ranger<const byte>(64, static_cast<const byte*>(nullptr));
		//updateInfo.Offset = 0;
		//info.MaterialSystem->UpdateRenderGroupData(updateInfo);
	}
};

void RenderOrchestrator::Initialize(const InitializeInfo& initializeInfo)
{
	systems.Initialize(32, GetPersistentAllocator());
	
	{
		const GTSL::Array<TaskDependency, 4> dependencies{ { CLASS_NAME, AccessType::READ_WRITE } };
		initializeInfo.GameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, "GameplayEnd", "RenderStart");
		initializeInfo.GameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderSetup", "RenderFinished");
	}

	renderManagers.Initialize(16, GetPersistentAllocator());

	renderManagers.Emplace(Id("StaticMeshRenderGroup"), new StaticMeshRenderManager());
	renderManagers.Emplace(Id("TextSystem"), new TextRenderManager());
}

void RenderOrchestrator::Shutdown(const ShutdownInfo& shutdownInfo)
{
	ForEach(renderManagers, [&](RenderManager* renderManager)
	{
		delete renderManager;
	});
}

void RenderOrchestrator::Setup(TaskInfo taskInfo)
{
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
	
	RenderManager::SetupInfo setupInfo;
	setupInfo.GameInstance = taskInfo.GameInstance;
	setupInfo.RenderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	setupInfo.MaterialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	setupInfo.ProjectionMatrix = projectionMatrix;
	setupInfo.ViewMatrix = viewMatrix;
	GTSL::ForEach(renderManagers, [&](RenderManager* renderManager) { renderManager->Setup(setupInfo); });
}

void RenderOrchestrator::Render(TaskInfo taskInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	auto& commandBuffer = *renderSystem->GetCurrentCommandBuffer();
	uint8 currentFrame = renderSystem->GetCurrentFrame();
	auto* materialSystem = taskInfo.GameInstance->GetSystem<MaterialSystem>("MaterialSystem");
	
	BindingsManager<BE::TAR> bindingsManager(GetTransientAllocator(), renderSystem, renderSystem->GetCurrentCommandBuffer());
	
	bindingsManager.AddBinding(materialSystem->globalBindingsSets[currentFrame], PipelineType::RASTER, materialSystem->globalPipelineLayout);

	GTSL::Array<Id, 16> renderGroups;

	renderGroups.EmplaceBack("StaticMeshRenderGroup"); renderGroups.EmplaceBack("TextSystem");

	auto* frameManager = taskInfo.GameInstance->GetSystem<FrameManager>("FrameManager");
	
	for (uint8 rp = 0; rp < frameManager->GetRenderPassCount(); ++rp)
	{
		auto renderPass = frameManager->GetRenderPass(rp);
		auto frameBuffer = frameManager->GetFrameBuffer(rp);

		CommandBuffer::BeginRenderPassInfo beginRenderPass;
		beginRenderPass.RenderDevice = renderSystem->GetRenderDevice();
		beginRenderPass.RenderPass = &renderPass;
		beginRenderPass.Framebuffer = &frameBuffer;
		beginRenderPass.RenderArea = renderSystem->GetRenderExtent();
		beginRenderPass.ClearValues = frameManager->GetClearValues(rp);
		commandBuffer.BeginRenderPass(beginRenderPass);
		
		for (uint8 sp = 0; sp < frameManager->GetSubPassCount(rp); ++sp)
		{
			for (auto e : renderGroups)
			{
				RenderManager::RenderInfo renderInfo;
				renderInfo.RenderSystem = renderSystem;
				renderInfo.GameInstance = taskInfo.GameInstance;
				renderInfo.CommandBuffer = &commandBuffer;
				renderInfo.MaterialSystem = materialSystem;
				renderInfo.CurrentFrame = renderSystem->GetCurrentFrame();
				renderInfo.BindingsManager = &bindingsManager;
				renderInfo.RenderPass = rp; renderInfo.SubPass = sp;
				renderManagers.At(e)->Render(renderInfo);
			}

			if (sp < frameManager->GetSubPassCount(rp) - 1)
			{
				commandBuffer.AdvanceSubPass(CommandBuffer::AdvanceSubpassInfo{});
			}
		}

		CommandBuffer::EndRenderPassInfo endRenderPass;
		endRenderPass.RenderDevice = renderSystem->GetRenderDevice();
		commandBuffer.EndRenderPass(endRenderPass);
	}

	{
		CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
		pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
		pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
		pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
		GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
		textureBarriers[0].Texture = renderSystem->GetSwapchainTextures()[currentFrame];
		textureBarriers[0].CurrentLayout = TextureLayout::PRESENTATION;
		textureBarriers[0].TargetLayout = TextureLayout::TRANSFER_DST;
		textureBarriers[0].SourceAccessFlags = AccessFlags::TRANSFER_READ;
		textureBarriers[0].DestinationAccessFlags = AccessFlags::TRANSFER_WRITE;
		pipelineBarrierInfo.TextureBarriers = textureBarriers;
		commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);
	}

	CommandBuffer::CopyTextureToTextureInfo copyTexture;
	copyTexture.RenderDevice = renderSystem->GetRenderDevice();
	copyTexture.SourceTexture = frameManager->GetAttachmentTexture("Color");
	copyTexture.DestinationTexture = renderSystem->GetSwapchainTextures()[currentFrame];
	copyTexture.Extent = { renderSystem->GetRenderExtent().Width, renderSystem->GetRenderExtent().Height, 1 };
	copyTexture.SourceLayout = TextureLayout::TRANSFER_SRC;
	copyTexture.DestinationLayout = TextureLayout::TRANSFER_DST;
	commandBuffer.CopyTextureToTexture(copyTexture);

	{
		CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
		pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
		pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
		pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
		GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
		textureBarriers[0].Texture = renderSystem->GetSwapchainTextures()[currentFrame];
		textureBarriers[0].CurrentLayout = TextureLayout::TRANSFER_DST;
		textureBarriers[0].TargetLayout = TextureLayout::PRESENTATION;
		textureBarriers[0].SourceAccessFlags = AccessFlags::TRANSFER_READ;
		textureBarriers[0].DestinationAccessFlags = AccessFlags::TRANSFER_WRITE;
		pipelineBarrierInfo.TextureBarriers = textureBarriers;
		commandBuffer.AddPipelineBarrier(pipelineBarrierInfo);
	}
	
	bindingsManager.PopBindings();
}

void RenderOrchestrator::AddRenderGroup(GameInstance* gameInstance, Id renderGroupName, RenderGroup* renderGroup)
{
	systems.EmplaceBack(renderGroupName);
	gameInstance->RemoveTask(SETUP_TASK_NAME, "GameplayEnd");
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
	dependencies.EmplaceBack("MaterialSystem", AccessType::READ);
	dependencies.EmplaceBack("FrameManager", AccessType::READ);

	gameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, "GameplayEnd", "RenderStart");
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderSetup", "RenderFinished");
}

void RenderOrchestrator::RemoveRenderGroup(GameInstance* gameInstance, const Id renderGroupName)
{
	const auto element = systems.Find(renderGroupName);
	BE_ASSERT(element != systems.end())
	
	systems.Pop(element - systems.begin());
	systemsAccesses.Pop(element - systems.begin());
	gameInstance->RemoveTask(SETUP_TASK_NAME, "GameplayEnd");
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
	dependencies.EmplaceBack("MaterialSystem", AccessType::READ);
	dependencies.EmplaceBack("FrameManager", AccessType::READ);
	
	gameInstance->AddTask(SETUP_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Setup>(this), dependencies, "GameplayEnd", "RenderStart");
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderSetup", "RenderFinished");
}
