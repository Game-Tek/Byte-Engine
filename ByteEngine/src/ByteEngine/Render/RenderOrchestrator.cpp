#include "RenderOrchestrator.h"

#undef MemoryBarrier

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
#include "ByteEngine/Application/Application.h"
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

			auto meshes = renderGroup->GetMeshes();

			GTSL::ForEach(renderGroup->GetMeshesByMaterial(), [&](const GTSL::Vector<uint32, BE::PersistentAllocatorReference>& e)
				{
					for(auto m : e)
					{
						const auto& i = meshes[m];
					
						if (renderInfo.MaterialSystem->IsMaterialReady(i.Material))
						{
							auto& materialInstances = renderInfo.MaterialSystem->GetMaterialInstances();
							auto& materialInstance = materialInstances[i.Material.MaterialInstance];

							{
								auto offset = GTSL::Array<uint32, 1>{ ((renderGroup->GetMeshCount() * 64) * renderInfo.CurrentFrame) + (m * 64) };
								renderInfo.BindingsManager->AddBinding(renderGroupInstance.BindingsSets[renderInfo.CurrentFrame], offset, PipelineType::RASTER, renderGroupInstance.PipelineLayout);
							}
							
							if (materialInstance.TextureParametersBindings.DataSize)
							{
								renderInfo.BindingsManager->AddBinding(materialInstance.TextureParametersBindings.BindingsSets[renderInfo.CurrentFrame], PipelineType::RASTER, materialInstance.PipelineLayout);
							}
							
							CommandBuffer::BindPipelineInfo bindPipelineInfo;
							bindPipelineInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
							bindPipelineInfo.PipelineType = PipelineType::RASTER;
							bindPipelineInfo.Pipeline = &materialInstance.Pipeline;
							renderInfo.CommandBuffer->BindPipeline(bindPipelineInfo);

							CommandBuffer::BindVertexBufferInfo bindVertexInfo;
							bindVertexInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
							bindVertexInfo.Buffer = &i.Buffer;
							bindVertexInfo.Offset = 0;
							renderInfo.CommandBuffer->BindVertexBuffer(bindVertexInfo);
							CommandBuffer::BindIndexBufferInfo bindIndexBuffer;
							bindIndexBuffer.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
							bindIndexBuffer.Buffer = &i.Buffer;
							bindIndexBuffer.Offset = i.IndicesOffset;
							bindIndexBuffer.IndexType = i.IndexType;
							renderInfo.CommandBuffer->BindIndexBuffer(bindIndexBuffer);
							CommandBuffer::DrawIndexedInfo drawIndexedInfo;
							drawIndexedInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
							drawIndexedInfo.InstanceCount = 1;
							drawIndexedInfo.IndexCount = i.IndicesCount;
							renderInfo.CommandBuffer->DrawIndexed(drawIndexedInfo);

							if (materialInstance.TextureParametersBindings.DataSize)
							{
								renderInfo.BindingsManager->PopBindings(); //material
							}

							renderInfo.BindingsManager->PopBindings(); //render group
						}

					}
				}
			);
		}
	}

	void Setup(const SetupInfo& info) override
	{
		auto* data = info.MaterialSystem->GetRenderGroupDataPointer("StaticMeshRenderGroup");
		
		auto* const renderGroup = info.GameInstance->GetSystem<StaticMeshRenderGroup>("StaticMeshRenderGroup");
		auto positions = renderGroup->GetPositions();
		
		uint32 offset = renderGroup->GetMeshCount() * 64 * info.RenderSystem->GetCurrentFrame();

		{
			uint32 index = 0;
			
			for (auto& e : positions)
			{
				auto pos = GTSL::Math::Translation(e);
				pos(2, 3) *= -1.f;
				*(reinterpret_cast<GTSL::Matrix4*>(static_cast<byte*>(data) + offset) + index) = info.ProjectionMatrix * info.ViewMatrix * pos;
				
				++index;
			}
		}
		
		MaterialSystem::UpdateRenderGroupDataInfo updateInfo;
		updateInfo.RenderGroup = "StaticMeshRenderGroup";
		updateInfo.Data = GTSL::Range<const byte*>(64, static_cast<const byte*>(data));
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
			auto* textSystem = renderInfo.GameInstance->GetSystem<TextSystem>("TextSystem");
			
			if (textSystem->GetTexts().ElementCount())
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
		
				if (renderInfo.MaterialSystem->IsMaterialReady(MaterialHandle{}))
				{
					CommandBuffer::BindPipelineInfo bindPipelineInfo;
					bindPipelineInfo.RenderDevice = renderInfo.RenderSystem->GetRenderDevice();
					bindPipelineInfo.PipelineType = PipelineType::RASTER;
					bindPipelineInfo.Pipeline = &materialInstance.Pipeline;
					renderInfo.CommandBuffer->BindPipeline(bindPipelineInfo);
		
					auto& text = textSystem->GetTexts()[0];
		
					CommandBuffer::DrawInfo drawInfo;
					drawInfo.FirstInstance = 0;
					drawInfo.FirstVertex = 0;
					drawInfo.InstanceCount = (text.String.GetLength() - 1);
					drawInfo.VertexCount = 6;
					renderInfo.CommandBuffer->Draw(drawInfo);
				}
		
				renderInfo.BindingsManager->PopBindings();
			}
		}
	}

	void Setup(const SetupInfo& info) override
	{
		auto textSystem = info.GameInstance->GetSystem<TextSystem>("TextSystem");
		
		float32 scale = 1.0f;
		
		if (textSystem->GetTexts().ElementCount())
		{
			int32 atlasIndex = 0;
			
			auto& text = textSystem->GetTexts()[0];
			auto& imageFont = textSystem->GetFont();
		
			auto x = text.Position.X;
			auto y = text.Position.Y;
			
			byte* data = static_cast<byte*>(info.MaterialSystem->GetRenderGroupDataPointer("TextSystem"));
		
			uint32 offset = 0;
			
			GTSL::Matrix4 ortho;
			auto renderExtent = info.RenderSystem->GetRenderExtent();
			GTSL::Math::MakeOrthoMatrix(ortho, static_cast<float32>(renderExtent.Width) * 0.5f, static_cast<float32>(renderExtent.Width) * -0.5f, static_cast<float32>(renderExtent.Height) * 0.5f, static_cast<float32>(renderExtent.Height) * -0.5f, 1, 100);
			GTSL::MemCopy(sizeof(ortho), &ortho, data + offset); offset += sizeof(ortho);
			GTSL::MemCopy(sizeof(uint32), &atlasIndex, data + offset); offset += sizeof(uint32); offset += sizeof(uint32) * 3;
			
			for (auto* c = text.String.begin(); c != text.String.end() - 1; c++)
			{
				auto& ch = imageFont.Characters.at(*c);
		
				float xpos = x + ch.Bearing.X * scale;
				float ypos = y - (ch.Size.Height - ch.Bearing.Y) * scale;
		
				float w = ch.Size.Width * scale;
				float h = ch.Size.Height * scale;
				
				// update VBO for each character
				float vertices[6][4] = {
					{ xpos,     -(ypos + h),   0.0f, 0.0f },
					{ xpos,     -(ypos),       0.0f, 1.0f },
					{ xpos + w, -(ypos),       1.0f, 1.0f },
		
					{ xpos,     -(ypos + h),   0.0f, 0.0f },
					{ xpos + w, -(ypos),       1.0f, 1.0f },
					{ xpos + w, -(ypos + h),   1.0f, 0.0f }
				};
				
				// now advance cursors for next glyph (note that advance is number of 1/64 pixels)
				x += (ch.Advance >> 6) * scale; // bitshift by 6 to get value in pixels (2^6 = 64)
		
				uint32 val = ch.Position.Width;
				GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
				val = ch.Position.Height;
				GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);

				val = ch.Size.Width;
				GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
				val = ch.Size.Height;
				GTSL::MemCopy(sizeof(val), &val, data + offset); offset += sizeof(val);
				
				for (uint32 v = 0; v < 6; ++v)
				{
					GTSL::MemCopy(sizeof(GTSL::Vector2), &vertices[v][0], data + offset); offset += sizeof(GTSL::Vector2); //vertices
					GTSL::MemCopy(sizeof(GTSL::Vector2), &vertices[v][2], data + offset); offset += sizeof(GTSL::Vector2); //uv
				}
				
			}
		
		}
		//MaterialSystem::UpdateRenderGroupDataInfo updateInfo;
		//updateInfo.RenderGroup = "TextSystem";
		//updateInfo.Data = GTSL::Range<const byte>(64, static_cast<const byte*>(nullptr));
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
		initializeInfo.GameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderDo", "RenderFinished");
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

	{
		CommandBuffer::BeginRegionInfo beginRegionInfo;
		beginRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
		beginRegionInfo.Name = GTSL::StaticString<64>("Graphics");
		commandBuffer.BeginRegion(beginRegionInfo);
	}

	CommandBuffer::EndRegionInfo endRegionInfo;
	endRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
	
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

	commandBuffer.EndRegion(endRegionInfo);

	{
		CommandBuffer::BeginRegionInfo beginRegionInfo;
		beginRegionInfo.RenderDevice = renderSystem->GetRenderDevice();
		beginRegionInfo.Name = GTSL::StaticString<64>("Copy render target to Swapchain");
		commandBuffer.BeginRegion(beginRegionInfo);
	}
	
	{
		CommandBuffer::AddPipelineBarrierInfo pipelineBarrierInfo;
		pipelineBarrierInfo.RenderDevice = renderSystem->GetRenderDevice();
		pipelineBarrierInfo.InitialStage = PipelineStage::TRANSFER;
		pipelineBarrierInfo.FinalStage = PipelineStage::TRANSFER;
		GTSL::Array<CommandBuffer::TextureBarrier, 2> textureBarriers(1);
		textureBarriers[0].Texture = renderSystem->GetSwapchainTextures()[currentFrame];
		textureBarriers[0].CurrentLayout = TextureLayout::UNDEFINED;
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

	commandBuffer.EndRegion(endRegionInfo);
	
	bindingsManager.PopBindings();
}

void RenderOrchestrator::AddRenderGroup(GameInstance* gameInstance, Id renderGroupName, RenderGroup* renderGroup)
{
	systems.EmplaceBack(renderGroupName);
	gameInstance->RemoveTask(SETUP_TASK_NAME, "GameplayEnd");
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderDo");

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
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderDo", "RenderFinished");
}

void RenderOrchestrator::RemoveRenderGroup(GameInstance* gameInstance, const Id renderGroupName)
{
	const auto element = systems.Find(renderGroupName);
	BE_ASSERT(element != systems.end())
	
	systems.Pop(element - systems.begin());
	systemsAccesses.Pop(element - systems.begin());
	gameInstance->RemoveTask(SETUP_TASK_NAME, "GameplayEnd");
	gameInstance->RemoveTask(RENDER_TASK_NAME, "RenderDo");

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
	gameInstance->AddTask(RENDER_TASK_NAME, GTSL::Delegate<void(TaskInfo)>::Create<RenderOrchestrator, &RenderOrchestrator::Render>(this), dependencies, "RenderDo", "RenderFinished");
}
