#include "MaterialSystem.h"

#include "RenderSystem.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

#include <GTSL/SIMD/SIMD.hpp>
#include <GAL/Texture.h>

#include "RenderOrchestrator.h"
#include "ByteEngine/Application/Application.h"

const char* BindingTypeString(const BindingType binding)
{
	switch (binding)
	{
	case BindingType::UNIFORM_BUFFER_DYNAMIC: return "UNIFORM_BUFFER_DYNAMIC";
	case BindingType::COMBINED_IMAGE_SAMPLER: return "COMBINED_IMAGE_SAMPLER";
	case BindingType::UNIFORM_BUFFER: return "UNIFORM_BUFFER";
	default: return "null";
	}
}

void MaterialSystem::Initialize(const InitializeInfo& initializeInfo)
{
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	minUniformBufferOffset = renderSystem->GetRenderDevice()->GetMinUniformBufferOffset();

	{
		const GTSL::Array<TaskDependency, 6> taskDependencies{ { "MaterialSystem", AccessType::READ_WRITE }, { "RenderSystem", AccessType::READ } };
		//initializeInfo.GameInstance->AddTask("updateDescriptors", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateDescriptors>(this), taskDependencies, "FrameStart", "RenderStart");
		initializeInfo.GameInstance->AddTask("updateDescriptors", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateDescriptors>(this), taskDependencies, "RenderStartSetup", "RenderEndSetup");
	}

	{
		const GTSL::Array<TaskDependency, 6> taskDependencies{ { "MaterialSystem", AccessType::READ_WRITE }, };
		initializeInfo.GameInstance->AddTask("updateCounter", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateCounter>(this), taskDependencies, "RenderEnd", "FrameEnd");
	}

	textures.Initialize(64, GetPersistentAllocator());
	texturesRefTable.Initialize(64, GetPersistentAllocator());

	queuedBufferUpdates.Initialize(1, 2, GetPersistentAllocator());

	latestLoadedTextures.Initialize(8, GetPersistentAllocator());
	pendingMaterialsPerTexture.Initialize(16, GetPersistentAllocator());

	materials.Initialize(16, GetPersistentAllocator());
	pendingMaterials.Initialize(16, GetPersistentAllocator());
	readyMaterialsMap.Initialize(32, GetPersistentAllocator());
	readyMaterialHandles.Initialize(16, GetPersistentAllocator());

	setHandlesByName.Initialize(16, GetPersistentAllocator());

	renderGroupsData.Initialize(4, GetPersistentAllocator());
	readyMaterialsPerRenderGroup.Initialize(8, GetPersistentAllocator());

	shaderGroupsByName.Initialize(16, GetPersistentAllocator());

	sets.Initialize(16, GetPersistentAllocator());
	
	for (uint32 i = 0; i < MAX_CONCURRENT_FRAMES; ++i)
	{
		descriptorsUpdates.EmplaceBack();
		descriptorsUpdates.back().Initialize(GetPersistentAllocator());
	}

	frame = 0;

	{
		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;

		//textures
		bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::COMBINED_IMAGE_SAMPLER,
			ShaderStage::ALL, 5, BindingFlags::PARTIALLY_BOUND });

		//attachments
		bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::STORAGE_IMAGE,
			ShaderStage::RAY_GEN | ShaderStage::CALLABLE,
			16, BindingFlags::PARTIALLY_BOUND });
		
		if (BE::Application::Get()->GetOption("rayTracing"))
		{
			//top level as
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::ACCELERATION_STRUCTURE,
				ShaderStage::RAY_GEN | ShaderStage::CALLABLE,
				1, 0 });

			//vertex buffers
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::STORAGE_BUFFER,
				ShaderStage::ANY_HIT | ShaderStage::CLOSEST_HIT | ShaderStage::INTERSECTION,
				16, BindingFlags::PARTIALLY_BOUND });

			//index buffers
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::STORAGE_BUFFER,
				ShaderStage::ANY_HIT | ShaderStage::CLOSEST_HIT | ShaderStage::INTERSECTION,
				16, BindingFlags::PARTIALLY_BOUND });
		}

		makeSetEx(renderSystem, "GlobalData", Id(), bindingDescriptors);
	}

	if (BE::Application::Get()->GetOption("rayTracing"))
	{
		auto* materialResorceManager = BE::Application::Get()->GetResourceManager<MaterialResourceManager>("MaterialResourceManager");

		Buffer::CreateInfo sbtCreateInfo;
		sbtCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { sbtCreateInfo.Name = GTSL::StaticString<32>("SBT Buffer. Material System"); }
		sbtCreateInfo.Size = materialResorceManager->GetRayTraceShaderCount() * renderSystem->GetShaderGroupHandleSize();
		sbtCreateInfo.BufferType = BufferType::RAY_TRACING;
		RenderSystem::BufferScratchMemoryAllocationInfo scratchMemoryInfo;
		scratchMemoryInfo.Buffer = &shaderBindingTableBuffer;
		scratchMemoryInfo.CreateInfo = &sbtCreateInfo;
		scratchMemoryInfo.Allocation = &shaderBindingTableAllocation;
		renderSystem->AllocateScratchBufferMemory(scratchMemoryInfo);

		GTSL::Vector<RayTracingPipeline::Group, BE::TAR> groups(16, GetTransientAllocator());
		GTSL::Vector<Pipeline::ShaderInfo, BE::TAR> shaderInfos(16, GetTransientAllocator());
		GTSL::Vector<Shader, BE::TAR> shaders(16, GetTransientAllocator());

		for (uint32 i = 0; i < materialResorceManager->GetRayTraceShaderCount(); ++i)
		{
			uint32 bufferSize = 0;
			bufferSize = materialResorceManager->GetRayTraceShaderSize(materialResorceManager->GetRayTraceShaderHandle(i));
			GTSL::Buffer shaderBuffer; shaderBuffer.Allocate(bufferSize, 8, GetTransientAllocator());

			shaderGroupsByName.Emplace(materialResorceManager->GetRayTraceShaderHandle(i)(), i);
			
			auto material = materialResorceManager->LoadRayTraceShaderSynchronous(materialResorceManager->GetRayTraceShaderHandle(i), GTSL::Range<byte*>(shaderBuffer.GetCapacity(), shaderBuffer.GetData())); //TODO: VIRTUAL BUFFER INTERFACE

			Shader::CreateInfo createInfo;
			createInfo.RenderDevice = renderSystem->GetRenderDevice();
			createInfo.ShaderData = GTSL::Range<const byte*>(material.BinarySize, shaderBuffer.GetData());

			shaders.EmplaceBack(createInfo);
			
			Pipeline::ShaderInfo shaderInfo;
			shaderInfo.Shader = shaders[i];
			shaderInfo.Type = ConvertShaderType(material.ShaderType);
			shaderInfos.EmplaceBack(shaderInfo);

			RayTracingPipeline::Group group{};

			group.GeneralShader = RayTracingPipeline::Group::SHADER_UNUSED; group.ClosestHitShader = RayTracingPipeline::Group::SHADER_UNUSED;
			group.AnyHitShader = RayTracingPipeline::Group::SHADER_UNUSED; group.IntersectionShader = RayTracingPipeline::Group::SHADER_UNUSED;

			switch (material.ShaderType)
			{
			case GAL::ShaderType::RAY_GEN: {
				group.ShaderGroup = GAL::VulkanShaderGroupType::GENERAL; group.GeneralShader = i;
				++rayGenShaderCount;
				break;
			}
			case GAL::ShaderType::MISS: {
				//generalShader is the index of the ray generation,miss, or callable shader from VkRayTracingPipelineCreateInfoKHR::pStages
				//in the group if the shader group has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_GENERAL_KHR, and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::VulkanShaderGroupType::GENERAL; group.GeneralShader = i;
				++missShaderCount;
				break;
			}
			case GAL::ShaderType::CALLABLE: {
				group.ShaderGroup = GAL::VulkanShaderGroupType::GENERAL; group.GeneralShader = i;
				++callableShaderCount;
				break;
			}
			case GAL::ShaderType::CLOSEST_HIT: {
				//closestHitShader is the optional index of the closest hit shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the shader group
				//has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_TRIANGLES_HIT_GROUP_KHR or VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR, and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::VulkanShaderGroupType::TRIANGLES; group.ClosestHitShader = i;
				++hitShaderCount;
				break;
			}
			case GAL::ShaderType::ANY_HIT: {
				//anyHitShader is the optional index of the any-hit shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the
				//shader group has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_TRIANGLES_HIT_GROUP_KHR or VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR,
				//and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::VulkanShaderGroupType::TRIANGLES; group.AnyHitShader = i;
				++hitShaderCount;
				break;
			}
			case GAL::ShaderType::INTERSECTION: {
				//intersectionShader is the index of the intersection shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the shader group
				//has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR, and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::VulkanShaderGroupType::PROCEDURAL; group.IntersectionShader = i;
				++hitShaderCount;
				break;
			}

			default: BE_LOG_MESSAGE("Non raytracing shader found in raytracing material");
			}

			groups.EmplaceBack(group);

			shaderBuffer.Free(8, GetTransientAllocator());
		}

		auto& set = sets[setHandlesByName.At(Id("GlobalData")())()];

		RayTracingPipeline::CreateInfo createInfo;
		createInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name("Ray Tracing Pipeline: "); createInfo.Name = name;
		}
		
		createInfo.MaxRecursionDepth = 3;
		createInfo.Stages = shaderInfos;
		createInfo.PipelineLayout = set.PipelineLayout;

		createInfo.Groups = groups;
		rayTracingPipeline.Initialize(createInfo);

		for (auto& s : shaders) { s.Destroy(renderSystem->GetRenderDevice()); }
		
		auto handleSize = renderSystem->GetShaderGroupHandleSize();
		auto alignedHandleSize = GTSL::Math::RoundUpByPowerOf2(handleSize, renderSystem->GetShaderGroupAlignment());

		GTSL::SmartBuffer<BE::TAR> handlesBuffer(groups.GetLength() * alignedHandleSize, renderSystem->GetShaderGroupAlignment(), GetTransientAllocator());

		rayTracingPipeline.GetShaderGroupHandles(renderSystem->GetRenderDevice(), 0, groups.GetLength(), GTSL::Range<byte*>(handlesBuffer->GetCapacity(), handlesBuffer->GetData()));

		auto* sbt = reinterpret_cast<byte*>(shaderBindingTableBuffer.GetAddress(renderSystem->GetRenderDevice()));

		for (uint32 h = 0; h < groups.GetLength(); ++h)
		{
			GTSL::MemCopy(handleSize, handlesBuffer->GetData() + h * handleSize, sbt + alignedHandleSize * h);
		}
	}
}

void MaterialSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* renderSystem = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
}

Pipeline MaterialSystem::GET_PIPELINE(MaterialHandle materialHandle)
{
	if (materials.IsSlotOccupied(materialHandle.Element))
	{
		return materials[materialHandle.Element].Pipeline;
	}

	return Pipeline();
}

void MaterialSystem::BIND_SET(RenderSystem* renderSystem, CommandBuffer commandBuffer, SetHandle setHandle, uint32 index)
{
	if constexpr (_DEBUG)
	{
		//if(!setHandlesByName.Find(setHandle())) { BE_LOG_ERROR("Tried to bind set which doesn't exist at render time!. ", BE::FIX_OR_CRASH_STRING) }
	}

	auto& set = sets[setHandle()];

	if (set.BindingsSet[frame].GetHandle())
	{
		GTSL::Array<uint32, 2> offsets;

		if (set.AllocatedInstances) { offsets.EmplaceBack(set.MemberSize * 0); }

		CommandBuffer::BindBindingsSetInfo bindBindingsSetInfo;
		bindBindingsSetInfo.RenderDevice = renderSystem->GetRenderDevice();
		bindBindingsSetInfo.FirstSet = set.Level;
		bindBindingsSetInfo.BoundSets = 1;
		bindBindingsSetInfo.BindingsSets = GTSL::Range<BindingsSet*>(1, &set.BindingsSet[frame]);
		bindBindingsSetInfo.PipelineLayout = &set.PipelineLayout;
		bindBindingsSetInfo.PipelineType = PipelineType::RASTER;
		bindBindingsSetInfo.Offsets = offsets;
		commandBuffer.BindBindingsSets(bindBindingsSetInfo);
	}

	CommandBuffer::UpdatePushConstantsInfo updatePush;
	updatePush.RenderDevice = renderSystem->GetRenderDevice();
	updatePush.Size = 4;
	updatePush.Offset = set.Level * 4;
	updatePush.Data = reinterpret_cast<byte*>(&index);
	updatePush.PipelineLayout = &set.PipelineLayout;
	updatePush.ShaderStages = ShaderStage::VERTEX | ShaderStage::FRAGMENT;
	commandBuffer.UpdatePushConstant(updatePush);
}

uint32 DataTypeSize(MaterialSystem::Member::DataType data)
{
	switch (data)
	{
	case MaterialSystem::Member::DataType::FLOAT32: return 4;
	case MaterialSystem::Member::DataType::UINT32: return 4;
	case MaterialSystem::Member::DataType::MATRIX4: return 4 * 4 * 4;
	case MaterialSystem::Member::DataType::FVEC4: return 4 * 4;
	case MaterialSystem::Member::DataType::INT32: return 4;
	case MaterialSystem::Member::DataType::FVEC2: return 4 * 2;
	default: return 0;
	}
}

SetHandle MaterialSystem::AddSet(RenderSystem* renderSystem, Id setName, Id parent, const SetInfo& setInfo)
{
	GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;

	uint32 structSize = 0;

	{
		for (auto& s : setInfo.Structs)
		{
			for (auto m : s.Members)
			{
				structSize += DataTypeSize(m.Type);
			}
		}
	}

	if (structSize)
	{
		bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::STORAGE_BUFFER_DYNAMIC, ShaderStage::ALL, 1, 0 });
	}

	auto setHandle = makeSetEx(renderSystem, setName, parent, bindingDescriptors);

	if (structSize)
	{
		auto& set = sets[setHandle()];

		{
			uint32 structSize = 0;

			for (auto& s : setInfo.Structs)
			{
				for (auto m : s.Members)
				{
					*m.Handle = MemberHandle(MemberDescription{ setHandle, static_cast<uint8>(structSize), static_cast<uint8>(m.Type) });
					
					structSize += DataTypeSize(m.Type);
				}

				//setBufferData.Structs.EmplaceBack(s);
				set.StructsSizes.EmplaceBack(structSize);
			}

			set.MemberSize = structSize;
		}

		uint32 newBufferSize = 0;
		set.AllocatedInstances = 16;

		for (uint32 i = 0; i < set.StructsSizes.GetLength(); ++i)
		{
			auto newStructSize = set.StructsSizes[i] * set.AllocatedInstances;
			newBufferSize += newStructSize;
		}

		Buffer::CreateInfo createInfo;
		createInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Set"); name += " "; name += setName.GetString();
			createInfo.Name = name;
		}
		createInfo.Size = newBufferSize;
		createInfo.BufferType = BufferType::ADDRESS;
		createInfo.BufferType |= BufferType::STORAGE;

		for (uint8 f = 0; f < queuedFrames; ++f)
		{
			RenderSystem::BufferScratchMemoryAllocationInfo allocationInfo;
			allocationInfo.CreateInfo = &createInfo;
			allocationInfo.Allocation = &set.Allocations[f];
			allocationInfo.Buffer = &set.Buffers[f];
			renderSystem->AllocateScratchBufferMemory(allocationInfo);
		}

		for (uint8 f = 0; f < queuedFrames; ++f)
		{
			auto updateHandle = descriptorsUpdates[f].AddSetToUpdate(setHandle, GetPersistentAllocator());

			BindingsSet::BufferBindingsUpdateInfo bufferBindingsUpdate;
			bufferBindingsUpdate.Buffer = set.Buffers[f];
			bufferBindingsUpdate.Offset = 0;
			bufferBindingsUpdate.Range = set.AllocatedInstances * set.StructsSizes[0];
			descriptorsUpdates[f].AddBufferUpdate(updateHandle, 0, bufferBindingsUpdate);
		}
	}

	return setHandle;
}

void MaterialSystem::AddObjects(RenderSystem* renderSystem, SetHandle setHandle, uint32 count)
{
	//GRAB ALL PER INSTANCE DATA
	//CALCULATE IF EXCEEDS CURRENT SIZE, IF IT DOES RESIZE

	//auto& renderGroupData = renderGroupsData.At(renderGroup);
	auto set = sets[setHandle()];

	if (set.MemberSize)
	{
		if (set.UsedInstances + count > set.AllocatedInstances)
		{
			resizeSet(renderSystem, setHandle); // Resize now

			queuedBufferUpdates.EmplaceBack(setHandle); //Defer resizing
		}

		set.UsedInstances += count;
	}
}

MaterialHandle MaterialSystem::CreateMaterial(const CreateMaterialInfo& info)
{
	uint32 material_size = 0;
	info.MaterialResourceManager->GetMaterialSize(info.MaterialName, material_size);

	GTSL::Buffer material_buffer; material_buffer.Allocate(material_size, 32, GetPersistentAllocator());
	
	const auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } };
	MaterialResourceManager::MaterialLoadInfo material_load_info;
	material_load_info.ActsOn = acts_on;
	material_load_info.GameInstance = info.GameInstance;
	material_load_info.Name = info.MaterialName;
	material_load_info.DataBuffer = GTSL::Range<byte*>(material_buffer.GetCapacity(), material_buffer.GetData());
	auto* matLoadInfo = GTSL::New<MaterialLoadInfo>(GetPersistentAllocator(), info.RenderSystem, MoveRef(material_buffer), matNum, info.TextureResourceManager);
	material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, matLoadInfo);
	material_load_info.OnMaterialLoad = GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onMaterialLoaded>(this);
	info.MaterialResourceManager->LoadMaterial(material_load_info);

	return MaterialHandle{ info.MaterialName, 0/*materials[comp].MaterialInstances*//*TODO: WHAT*/, matNum++ };
}

MaterialHandle MaterialSystem::CreateRayTracingMaterial(const CreateMaterialInfo& info)
{
	RayTracingPipeline rayTracingPipeline;

	return MaterialHandle();
}

void MaterialSystem::SetDynamicMaterialParameter(const MaterialHandle material, GAL::ShaderDataType type, Id parameterName, void* data)
{
	//auto& mat = materials[material.MaterialInstance];
	//
	//auto* matData = static_cast<byte*>(setsBufferData[mat.Set()].Allocations[frame].Data) + mat.DataSize * material.MaterialInstance;
	//
	////TODO: DEFER WRITING TO NOT OVERWRITE RUNNING FRAME
	//byte* FILL = matData + mat.DynamicParameters.At(parameterName);
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
	//FILL += GTSL::Math::RoundUpByPowerOf2(mat.DataSize, static_cast<uint32>(minUniformBufferOffset));
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
}

void MaterialSystem::SetMaterialParameter(const MaterialHandle material, GAL::ShaderDataType type, Id parameterName, void* data)
{
	//auto& mat = materials[material.MaterialInstance];
	//
	//auto* matData = static_cast<byte*>(mat.Allocation.Data) + mat.TextureParametersBindings.DataSize * material.MaterialInstance;
	//
	//byte* FILL = matData + mat.Parameters.At(parameterName);
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
	//FILL += GTSL::Math::RoundUpByPowerOf2(mat.TextureParametersBindings.DataSize, static_cast<uint32>(minUniformBufferOffset));
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
}

ComponentReference MaterialSystem::createTexture(const CreateTextureInfo& info)
{
	TextureResourceManager::TextureLoadInfo textureLoadInfo;
	textureLoadInfo.GameInstance = info.GameInstance;
	textureLoadInfo.Name = info.TextureName;

	textureLoadInfo.OnTextureLoadInfo = GTSL::Delegate<void(TaskInfo, TextureResourceManager::OnTextureLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onTextureLoad>(this);

	//const GTSL::Array<TaskDependency, 6> loadTaskDependencies{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } };
	const GTSL::Array<TaskDependency, 6> loadTaskDependencies;

	textureLoadInfo.ActsOn = loadTaskDependencies;

	auto component = textures.Emplace();

	{
		Buffer::CreateInfo scratchBufferCreateInfo;
		scratchBufferCreateInfo.RenderDevice = info.RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Scratch Buffer. Texture: "); name += info.TextureName.GetHash();
			scratchBufferCreateInfo.Name = name;
		}

		{
			uint32 textureSize; GAL::TextureFormat textureFormat; GTSL::Extent3D textureExtent;
			info.TextureResourceManager->GetTextureSizeFormatExtent(info.TextureName, &textureSize, &textureFormat, &textureExtent);

			RenderDevice::FindSupportedImageFormat findFormatInfo;
			findFormatInfo.TextureTiling = TextureTiling::OPTIMAL;
			findFormatInfo.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
			GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(textureFormat)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
			findFormatInfo.Candidates = candidates;
			const auto supportedFormat = info.RenderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormatInfo);

			scratchBufferCreateInfo.Size = textureExtent.Width * textureExtent.Depth * textureExtent.Height * FormatSize(supportedFormat);
		}

		scratchBufferCreateInfo.BufferType = BufferType::TRANSFER_SOURCE;

		Buffer scratchBuffer;

		HostRenderAllocation allocation;

		{
			RenderSystem::BufferScratchMemoryAllocationInfo scratchMemoryAllocation;
			scratchMemoryAllocation.Buffer = &scratchBuffer;
			scratchMemoryAllocation.CreateInfo = &scratchBufferCreateInfo;
			scratchMemoryAllocation.Allocation = &allocation;
			info.RenderSystem->AllocateScratchBufferMemory(scratchMemoryAllocation);
		}

		texturesRefTable.Emplace(info.TextureName(), component);
		
		auto* loadInfo = GTSL::New<TextureLoadInfo>(GetPersistentAllocator(), component, GTSL::MoveRef(scratchBuffer), info.RenderSystem, allocation);

		textureLoadInfo.DataBuffer = GTSL::Range<byte*>(allocation.Size, static_cast<byte*>(allocation.Data));

		textureLoadInfo.UserData = DYNAMIC_TYPE(TextureLoadInfo, loadInfo);
	}

	pendingMaterialsPerTexture.EmplaceAt(component, GetPersistentAllocator());
	pendingMaterialsPerTexture[component].Initialize(4, GetPersistentAllocator());
	
	info.TextureResourceManager->LoadTexture(textureLoadInfo);

	return ComponentReference(GetSystemId(), component);
}

void MaterialSystem::BindMaterial(MaterialHandle handle, CommandBuffer* commandBuffer, RenderSystem* renderSystem)
{
	auto pipeline = GET_PIPELINE(handle);

	if (pipeline.GetVkPipeline())
	{
		CommandBuffer::BindPipelineInfo bindPipelineInfo;
		bindPipelineInfo.RenderDevice = renderSystem->GetRenderDevice();
		bindPipelineInfo.PipelineType = PipelineType::RASTER;
		bindPipelineInfo.Pipeline = pipeline;
		commandBuffer->BindPipeline(bindPipelineInfo);

		BIND_SET(renderSystem, *commandBuffer, handle.MaterialType);
	}
}

void MaterialSystem::updateDescriptors(TaskInfo taskInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	for(uint32 p = 0; p < queuedBufferUpdates.GetReference().GetPageCount(); ++p)
	{
		for(uint32 i = 0; i < queuedBufferUpdates.GetReference().GetPage(p).ElementCount(); ++i)
		{
			resizeSet(renderSystem, queuedBufferUpdates.GetReference().GetPage(p)[i]);
		}
	}

	queuedBufferUpdates.Clear();

	for(auto e : latestLoadedTextures)
	{
		for (auto b : pendingMaterialsPerTexture[e])
		{
			if (++pendingMaterials[static_cast<uint32>(b)].Counter == pendingMaterials[static_cast<uint32>(b)].Target)
			{
				setMaterialAsLoaded(pendingMaterials[static_cast<uint32>(b)].Material, pendingMaterials[static_cast<uint32>(b)], pendingMaterials[static_cast<uint32>(b)].RenderGroup);
			}
		}
	}

	latestLoadedTextures.ResizeDown(0);
	
	BindingsSet::BindingsSetUpdateInfo bindingsUpdateInfo;
	bindingsUpdateInfo.RenderDevice = renderSystem->GetRenderDevice();

	{
		auto& descriptorsUpdate = descriptorsUpdates[frame];

		for(uint32 s = 0; s < descriptorsUpdate.setsToUpdate.GetLength(); ++s)
		{
			auto& bufferBindingsUpdate = descriptorsUpdate.PerSetToUpdateBufferBindingsUpdate[s];
			auto& textureBindingsUpdate = descriptorsUpdate.PerSetToUpdateTextureBindingsUpdate[s];
			
			if (bufferBindingsUpdate.GetGroupCount() || textureBindingsUpdate.GetGroupCount())
			{				
				Vector<BindingsSet::BindingUpdateInfo, BE::TAR> bindingUpdateInfos(4/*bindings sets*/, GetTransientAllocator());
				{
					for (uint32 i = 0; i < bufferBindingsUpdate.GetGroupCount(); ++i)
					{
						BindingsSet::BindingUpdateInfo bindingUpdateInfo;

						const auto& group = bufferBindingsUpdate.GetGroups()[i];
						
						bindingUpdateInfo.Type = BindingType::STORAGE_BUFFER_DYNAMIC;
						bindingUpdateInfo.ArrayElement = group.First;
						bindingUpdateInfo.Count = group.ElementCount;
						bindingUpdateInfo.BindingsUpdates = group.Elements;

						bindingUpdateInfos.EmplaceBack(bindingUpdateInfo);
					}

					for (uint32 i = 0; i < textureBindingsUpdate.GetGroupCount(); ++i)
					{
						BindingsSet::BindingUpdateInfo bindingUpdateInfo;

						const auto& group = textureBindingsUpdate.GetGroups()[i];
						
						bindingUpdateInfo.Type = BindingType::COMBINED_IMAGE_SAMPLER;
						bindingUpdateInfo.ArrayElement = group.First;
						bindingUpdateInfo.Count = group.ElementCount;
						bindingUpdateInfo.BindingsUpdates = group.Elements;

						bindingUpdateInfos.EmplaceBack(bindingUpdateInfo);
					}
				}

				bindingsUpdateInfo.BindingUpdateInfos = bindingUpdateInfos;

				sets[descriptorsUpdate.setsToUpdate[s]()].BindingsSet[frame].Update(bindingsUpdateInfo);
			}
		}

		descriptorsUpdate.Reset();
	}
}

void MaterialSystem::updateCounter(TaskInfo taskInfo)
{
	frame = (frame + 1) % queuedFrames;
}

void MaterialSystem::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo)
{	
	auto createMaterialInstance = [](TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo, MaterialSystem* materialSystem)
	{		
		auto loadInfo = DYNAMIC_CAST(MaterialLoadInfo, onMaterialLoadInfo.UserData);

		MaterialData material;
		
		auto* renderSystem = loadInfo->RenderSystem;

		//GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;

		GTSL::Array<BindingsPool::DescriptorPoolSize, 32> descriptorPoolSizes;

		{
			RasterizationPipeline::CreateInfo pipelineCreateInfo;
			pipelineCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> name("Raster pipeline. Material: "); name += onMaterialLoadInfo.ResourceName;
				pipelineCreateInfo.Name = name;
			}

			{
				GTSL::Array<ShaderDataType, 10> vertexDescriptor(onMaterialLoadInfo.VertexElements.GetLength());

				for (uint32 i = 0; i < onMaterialLoadInfo.VertexElements.GetLength(); ++i)
				{
					vertexDescriptor[i] = ConvertShaderDataType(onMaterialLoadInfo.VertexElements[i]);
				}

				pipelineCreateInfo.VertexDescriptor = vertexDescriptor;
			}

			MemberHandle textureHandle[8]; uint64 textureTableStructRef = ~(0ULL);
			
			{
				SetInfo setInfo;

				GTSL::Array<MemberInfo, 8> members;
				GTSL::Array<StructInfo, 8> structsInfos;
				
				for (uint32 t = 0; t < onMaterialLoadInfo.Textures.GetLength(); ++t)
				{
					MemberInfo textureHandles;
					textureHandles.Type = Member::DataType::UINT32;
					textureHandles.Handle = &textureHandle[t];
					members.EmplaceBack(textureHandles);
				}

				if (onMaterialLoadInfo.Textures.GetLength())
				{
					StructInfo structInfo;
					structInfo.Members = members;
					structInfo.Frequency = Frequency::PER_INSTANCE;
					structInfo.Handle = &textureTableStructRef;
					structsInfos.EmplaceBack(structInfo);
				}
				
				setInfo.Structs = structsInfos;

				
				if(!materialSystem->setHandlesByName.Find(onMaterialLoadInfo.ResourceName))
				{
					material.Set = materialSystem->AddSet(loadInfo->RenderSystem, onMaterialLoadInfo.ResourceName, onMaterialLoadInfo.RenderGroup, setInfo);
				}
				else
				{
					//material.Set = materialSystem->
				}
			}

			materialSystem->AddObjects(renderSystem, material.Set, 1); //Add current material to set

			for (uint32 t = 0; t < onMaterialLoadInfo.Textures.GetLength(); ++t)
			{
				material.TextureRefHandle[t] = textureHandle[t];
			}
			
			material.TextureRefsTableHandle = textureTableStructRef;
			
			//pipelineCreateInfo.IsInheritable = true;
			pipelineCreateInfo.PipelineDescriptor.BlendEnable = onMaterialLoadInfo.BlendEnable;
			pipelineCreateInfo.PipelineDescriptor.CullMode = onMaterialLoadInfo.CullMode;
			pipelineCreateInfo.PipelineDescriptor.DepthTest = onMaterialLoadInfo.DepthTest;
			pipelineCreateInfo.PipelineDescriptor.DepthWrite = onMaterialLoadInfo.DepthWrite;
			pipelineCreateInfo.PipelineDescriptor.StencilTest = onMaterialLoadInfo.StencilTest;
			pipelineCreateInfo.PipelineDescriptor.DepthCompareOperation = GAL::CompareOperation::LESS;
			pipelineCreateInfo.PipelineDescriptor.ColorBlendOperation = onMaterialLoadInfo.ColorBlendOperation;

			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.CompareOperation = onMaterialLoadInfo.Front.CompareOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.CompareMask = onMaterialLoadInfo.Front.CompareMask;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.DepthFailOperation = onMaterialLoadInfo.Front.DepthFailOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.FailOperation = onMaterialLoadInfo.Front.FailOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.PassOperation = onMaterialLoadInfo.Front.PassOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.Reference = onMaterialLoadInfo.Front.Reference;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.WriteMask = onMaterialLoadInfo.Front.WriteMask;

			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.CompareOperation = onMaterialLoadInfo.Back.CompareOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.CompareMask = onMaterialLoadInfo.Back.CompareMask;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.DepthFailOperation = onMaterialLoadInfo.Back.DepthFailOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.FailOperation = onMaterialLoadInfo.Back.FailOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.PassOperation = onMaterialLoadInfo.Back.PassOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.Reference = onMaterialLoadInfo.Back.Reference;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.WriteMask = onMaterialLoadInfo.Back.WriteMask;

			pipelineCreateInfo.SurfaceExtent = { 1, 1 };
			
			{
				GTSL::Array<Shader, 10> shaders; GTSL::Array<Pipeline::ShaderInfo, 16> shaderInfos;
				materialSystem->genShaderStages(loadInfo->RenderSystem->GetRenderDevice(), shaders, shaderInfos, onMaterialLoadInfo);
				
				pipelineCreateInfo.Stages = shaderInfos;

				auto* renderOrchestrator = taskInfo.GameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");

				auto renderPass = renderOrchestrator->getAPIRenderPass(onMaterialLoadInfo.RenderPass);
				pipelineCreateInfo.SubPass = renderOrchestrator->getAPISubPassIndex(onMaterialLoadInfo.RenderPass);
				pipelineCreateInfo.RenderPass = &renderPass;
				pipelineCreateInfo.PipelineLayout = materialSystem->sets[material.Set()].PipelineLayout;
				pipelineCreateInfo.PipelineCache = *renderSystem->GetPipelineCache();
				material.Pipeline = RasterizationPipeline(pipelineCreateInfo);
			}
		}

		auto matHandle = MaterialHandle{ onMaterialLoadInfo.ResourceName, 0/*TODO*/, loadInfo->Component };
		
		{
			uint32 targetValue = 0;

			if (onMaterialLoadInfo.Textures.GetLength())
			{
				auto place = materialSystem->pendingMaterials.Emplace(targetValue, GTSL::MoveRef(material));
				materialSystem->pendingMaterials[place].Material = matHandle;
				materialSystem->pendingMaterials[place].RenderGroup = onMaterialLoadInfo.RenderGroup;
				
				for (auto& e : onMaterialLoadInfo.Textures)
				{
					uint32 textureComp;

					uint32* textureComponent;

					if (!materialSystem->texturesRefTable.Find(e, textureComponent))
					{
						CreateTextureInfo createTextureInfo;
						createTextureInfo.RenderSystem = renderSystem;
						createTextureInfo.GameInstance = taskInfo.GameInstance;
						createTextureInfo.TextureResourceManager = loadInfo->TextureResourceManager;
						createTextureInfo.TextureName = e;
						createTextureInfo.MaterialHandle = matHandle;
						textureComp = materialSystem->createTexture(createTextureInfo).Component;
					}
					else
					{
						textureComp = *textureComponent;
					}

					materialSystem->addPendingMaterialToTexture(textureComp, PendingMaterialHandle(place));
					for (uint8 f = 0; f < materialSystem->queuedFrames; ++f)
					{
						*materialSystem->getSetMemberPointer<uint32>(material.TextureRefHandle[0](), 0, f) = textureComp; //TODO: FIX ACCESS
					}
					++materialSystem->pendingMaterials[place].Target;
				}
				
			}
			else
			{
				materialSystem->setMaterialAsLoaded(matHandle, material, onMaterialLoadInfo.RenderGroup);
			}
		}

		loadInfo->Buffer.Free(32, materialSystem->GetPersistentAllocator());
		GTSL::Delete(loadInfo, materialSystem->GetPersistentAllocator());
	};
	
	taskInfo.GameInstance->AddDynamicTask("createMatOnMatSystem", GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo, MaterialSystem*)>::Create(createMaterialInstance),
		GTSL::Array<TaskDependency, 2>{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } }, GTSL::MoveRef(onMaterialLoadInfo), this);
}

void MaterialSystem::setMaterialAsLoaded(const MaterialHandle matIndex, const MaterialData material, const Id renderGroup)
{
	materials.EmplaceAt(matIndex.Element, material);
	readyMaterialHandles.EmplaceBack(matIndex);

	GTSL::Vector<MaterialHandle, BE::PAR>* collection;

	if (!readyMaterialsPerRenderGroup.Find(renderGroup(), collection))
	{
		collection = &readyMaterialsPerRenderGroup.Emplace(renderGroup());
		collection->Initialize(8, GetPersistentAllocator());
		collection->EmplaceBack(matIndex);
	}
	else
	{
		collection->EmplaceBack(matIndex);
	}
}

SetHandle MaterialSystem::makeSetEx(RenderSystem* renderSystem, Id setName, Id parent, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDesc)
{
	SetHandle parentHandle, setHandle;
	uint32 level;

	if (parent.GetHash())
	{
		parentHandle = setHandlesByName.At(parent());
		level = sets[parentHandle()].Level + 1;
		setHandle = SetHandle(sets.Emplace());
	}
	else
	{
		parentHandle = SetHandle();
		setHandle = SetHandle(sets.Emplace());
		level = 0;
	}

	setHandlesByName.Emplace(setName(), setHandle);

	auto& set = sets[setHandle()];
	
	set.Parent = parentHandle;
	set.Level = level;
	
	GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts(level); //"Pre-Allocate" _level_ elements as to be able to place them in order while traversing tree upwards

	// Traverse tree to find parent's pipeline layouts
	{
		uint32 loopLevel = level;

		auto lastSet = setHandle;
		
		while (loopLevel)
		{
			lastSet = sets[lastSet()].Parent;
			bindingsSetLayouts[--loopLevel] = sets[lastSet()].BindingsSetLayout;
		}
	}

	{
		BindingsSetLayout::CreateInfo bindingsSetLayoutCreateInfo;
		bindingsSetLayoutCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		GTSL::StaticString<64> bindingsSetLayoutName("Bindings set layout. Set: "); bindingsSetLayoutName += setName.GetString();
		bindingsSetLayoutCreateInfo.Name = bindingsSetLayoutName;

		bindingsSetLayoutCreateInfo.BindingsDescriptors = bindingDesc;
		set.BindingsSetLayout = BindingsSetLayout(bindingsSetLayoutCreateInfo);

		bindingsSetLayouts.EmplaceBack(set.BindingsSetLayout);
	}
	
	if (bindingDesc.ElementCount())
	{
		{
			BindingsPool::CreateInfo bindingsPoolCreateInfo;
			bindingsPoolCreateInfo.RenderDevice = renderSystem->GetRenderDevice();

			if constexpr (_DEBUG)
			{
				GTSL::StaticString<64> name("Bindings pool. Set: "); name += setName.GetString();
				bindingsPoolCreateInfo.Name = name;
			}

			GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptorPoolSizes;

			for (auto e : bindingDesc)
			{
				descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ e.BindingType, e.UniformCount * queuedFrames });
			}

			bindingsPoolCreateInfo.DescriptorPoolSizes = descriptorPoolSizes;
			bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
			set.BindingsPool = BindingsPool(bindingsPoolCreateInfo);
		}

		{
			BindingsPool::AllocateBindingsSetsInfo allocateBindings;
			allocateBindings.RenderDevice = renderSystem->GetRenderDevice();

			for (uint8 f = 0; f < queuedFrames; ++f)
			{
				GTSL::Array<BindingsSet*, 8> bindingsSets;
				bindingsSets.EmplaceBack(&set.BindingsSet[f]);

				allocateBindings.BindingsSets = bindingsSets;

				{
					allocateBindings.BindingsSetLayouts = GTSL::Range<const BindingsSetLayout*>(1, &bindingsSetLayouts.back());
					GTSL::Array<uint32, 8> dynamicBindings;
					for(auto e : bindingDesc)
					{
						if (e.Flags & BindingFlags::VARIABLE_DESCRIPTOR_COUNT) { dynamicBindings.EmplaceBack(e.UniformCount); }
					}
					allocateBindings.BindingsSetDynamicBindingsCounts = dynamicBindings;

					GTSL::Array<GAL::VulkanCreateInfo, 1> bindingsSetsCreateInfo(1);

					if constexpr (_DEBUG)
					{
						GTSL::StaticString<64> name("BindingsSet. Set: "); name += setName.GetString();
						bindingsSetsCreateInfo[0].RenderDevice = renderSystem->GetRenderDevice();
						bindingsSetsCreateInfo[0].Name = name;
					}

					allocateBindings.BindingsSetCreateInfos = bindingsSetsCreateInfo;

					set.BindingsPool.AllocateBindingsSets(allocateBindings);
				}
			}
		}
	}

	{
		PipelineLayout::CreateInfo pipelineLayout;
		pipelineLayout.RenderDevice = renderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Pipeline layout. Set: "); name += setName.GetString();
			pipelineLayout.Name = name;
		}

		PipelineLayout::PushConstant pushConstant;
		pushConstant.ShaderStages = ShaderStage::VERTEX | ShaderStage::FRAGMENT;
		pushConstant.Offset = 0;
		pushConstant.Size = 16;

		pipelineLayout.PushConstant = &pushConstant;
		
		pipelineLayout.BindingsSetLayouts = bindingsSetLayouts;
		set.PipelineLayout.Initialize(pipelineLayout);
	}
	
	return setHandle;
}

void MaterialSystem::resizeSet(RenderSystem* renderSystem, SetHandle setHandle)
{
	auto& set = sets[setHandle()];
	
	//REALLOCATE
	uint32 newBufferSize = 0;
	Buffer newBuffer; HostRenderAllocation newAllocation;

	for (uint32 i = 0; i < set.StructsSizes.GetLength(); ++i)
	{
		auto newStructSize = set.StructsSizes[i] * set.AllocatedInstances * 2;
		newBufferSize += newStructSize;
	}

	Buffer::CreateInfo createInfo;
	createInfo.RenderDevice = renderSystem->GetRenderDevice();
	createInfo.Name = GTSL::StaticString<64>("undefined");
	createInfo.Size = newBufferSize;
	createInfo.BufferType = BufferType::ADDRESS;
	createInfo.BufferType |= BufferType::STORAGE;

	RenderSystem::BufferScratchMemoryAllocationInfo allocationInfo;
	allocationInfo.CreateInfo = &createInfo;
	allocationInfo.Allocation = &newAllocation;
	allocationInfo.Buffer = &newBuffer;
	renderSystem->AllocateScratchBufferMemory(allocationInfo);

	uint32 oldOffset = 0, newOffset = 0;

	for (uint32 i = 0; i < set.StructsSizes.GetLength(); ++i)
	{
		auto oldStructSize = set.StructsSizes[i] * set.AllocatedInstances;
		auto newStructSize = set.StructsSizes[i] * set.AllocatedInstances * 2;

		GTSL::MemCopy(oldStructSize, static_cast<byte*>(set.Allocations[frame].Data) + oldOffset, static_cast<byte*>(newAllocation.Data) + newOffset);

		oldOffset += oldStructSize;
		newOffset += newStructSize;
	}

	renderSystem->DeallocateScratchBufferMemory(set.Allocations[frame]);

	set.AllocatedInstances *= 2;
	set.Buffers[frame].Destroy(renderSystem->GetRenderDevice());
	set.Buffers[frame] = newBuffer;

	const auto setUpdateHandle = descriptorsUpdates[frame].AddSetToUpdate(setHandle, GetPersistentAllocator());

	BindingsSet::BufferBindingsUpdateInfo bufferBindingsUpdate;
	bufferBindingsUpdate.Buffer = set.Buffers[frame];
	bufferBindingsUpdate.Offset = 0;
	bufferBindingsUpdate.Range = newBufferSize;
	descriptorsUpdates[frame].AddBufferUpdate(setUpdateHandle, 0, bufferBindingsUpdate);
}

void MaterialSystem::onTextureLoad(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo onTextureLoadInfo)
{
	{
		auto* loadInfo = DYNAMIC_CAST(TextureLoadInfo, onTextureLoadInfo.UserData);

		RenderDevice::FindSupportedImageFormat findFormat;
		findFormat.TextureTiling = TextureTiling::OPTIMAL;
		findFormat.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
		GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(onTextureLoadInfo.TextureFormat)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
		findFormat.Candidates = candidates;
		auto supportedFormat = loadInfo->RenderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormat);

		GAL::Texture::ConvertTextureFormat(onTextureLoadInfo.TextureFormat, GAL::TextureFormat::RGBA_I8, onTextureLoadInfo.Extent, GTSL::AlignedPointer<byte, 16>(onTextureLoadInfo.DataBuffer.begin()), 1);

		{
			const GTSL::Array<TaskDependency, 6> loadTaskDependencies{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } };
			
			taskInfo.GameInstance->AddDynamicTask("onTextureProcessed", GTSL::Delegate<void(TaskInfo, TextureResourceManager::OnTextureLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onTextureProcessed>(this),
				loadTaskDependencies, GTSL::MoveRef(onTextureLoadInfo));
		}
	}
}

void MaterialSystem::onTextureProcessed(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo onTextureLoadInfo)
{
	auto* loadInfo = DYNAMIC_CAST(TextureLoadInfo, onTextureLoadInfo.UserData);

	RenderDevice::FindSupportedImageFormat findFormat;
	findFormat.TextureTiling = TextureTiling::OPTIMAL;
	findFormat.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
	GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(onTextureLoadInfo.TextureFormat)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
	findFormat.Candidates = candidates;
	auto supportedFormat = loadInfo->RenderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormat);
	
	TextureComponent textureComponent;

	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Texture. Texture: "); name += onTextureLoadInfo.ResourceName;
			textureCreateInfo.Name = name;
		}

		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.Uses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
		textureCreateInfo.Dimensions = ConvertDimension(onTextureLoadInfo.Dimensions);
		textureCreateInfo.Format = static_cast<GAL::VulkanTextureFormat>(supportedFormat);
		textureCreateInfo.Extent = onTextureLoadInfo.Extent;
		textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
		textureCreateInfo.MipLevels = 1;
		
		RenderSystem::AllocateLocalTextureMemoryInfo allocationInfo;
		allocationInfo.Allocation = &textureComponent.Allocation;
		allocationInfo.CreateInfo = &textureCreateInfo;
		allocationInfo.Texture = &textureComponent.Texture;

		loadInfo->RenderSystem->AllocateLocalTextureMemory(allocationInfo);
	}

	{
		TextureView::CreateInfo textureViewCreateInfo;
		textureViewCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Texture view. Texture: "); name += onTextureLoadInfo.ResourceName;
			textureViewCreateInfo.Name = name;
		}

		textureViewCreateInfo.Type = GAL::VulkanTextureType::COLOR;
		textureViewCreateInfo.Dimensions = ConvertDimension(onTextureLoadInfo.Dimensions);
		textureViewCreateInfo.Format = static_cast<GAL::VulkanTextureFormat>(supportedFormat);
		textureViewCreateInfo.Texture = textureComponent.Texture;
		textureViewCreateInfo.MipLevels = 1;

		textureComponent.TextureView = TextureView(textureViewCreateInfo);
	}

	{
		RenderSystem::TextureCopyData textureCopyData;
		textureCopyData.DestinationTexture = textureComponent.Texture;
		textureCopyData.SourceBuffer = loadInfo->Buffer;
		textureCopyData.Allocation = loadInfo->RenderAllocation;
		textureCopyData.Layout = TextureLayout::TRANSFER_DST;
		textureCopyData.Extent = onTextureLoadInfo.Extent;

		loadInfo->RenderSystem->AddTextureCopy(textureCopyData);
	}

	{
		TextureSampler::CreateInfo textureSamplerCreateInfo;
		textureSamplerCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Texture sampler. Texture: "); name += onTextureLoadInfo.ResourceName;
			textureSamplerCreateInfo.Name = name;
		}

		textureSamplerCreateInfo.Anisotropy = 0;

		textureComponent.TextureSampler = TextureSampler(textureSamplerCreateInfo);
	}

	textures[loadInfo->Component] = textureComponent;

	BE_LOG_MESSAGE("Loaded texture ", onTextureLoadInfo.ResourceName)

	BindingsSet::TextureBindingsUpdateInfo textureBindingsUpdateInfo;

	textureBindingsUpdateInfo.TextureView = textureComponent.TextureView;
	textureBindingsUpdateInfo.Sampler = textureComponent.TextureSampler;
	textureBindingsUpdateInfo.TextureLayout = TextureLayout::SHADER_READ_ONLY;

	for (uint8 f = 0; f < queuedFrames; ++f)
	{
		auto updateHandle = descriptorsUpdates[f].AddSetToUpdate(setHandlesByName.At(Id("GlobalData")()), GetPersistentAllocator());
		descriptorsUpdates[f].AddTextureUpdate(updateHandle, loadInfo->Component, textureBindingsUpdateInfo);
	}
	
	latestLoadedTextures.EmplaceBack(loadInfo->Component);
	
	GTSL::Delete(loadInfo, GetPersistentAllocator());
}
