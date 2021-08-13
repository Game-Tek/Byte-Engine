#pragma once

#include <GTSL/Vector.hpp>
#include <GTSL/Buffer.hpp>

#include "ResourceManager.h"

#include <GTSL/Delegate.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/File.h>
#include <GTSL/MappedFile.hpp>
#include <GTSL/Math/Vectors.h>


#include "ByteEngine/Game/ApplicationManager.h"

namespace GAL {
	enum class ShaderDataType : unsigned char;
}

namespace GTSL {
	class Vector2;
}

class StaticMeshResourceManager final : public ResourceManager
{
public:
	StaticMeshResourceManager();
	~StaticMeshResourceManager();

	struct StaticMeshData : Data
	{
		/**
		* \brief Number of vertices the loaded mesh contains.
		*/
		uint32 VertexCount;

		/**
		 * \brief Number of indeces the loaded mesh contains. Every face can only have three indeces.
		 */
		uint16 IndexCount;

		/**
		 * \brief Size of a single vertex.
		 */
		uint16 VertexSize;

		/**
		 * \brief Size of a single index to determine whether to use uint16 or uint32.
		 */
		uint8 IndexSize;

		GTSL::Vector3 BoundingBox; float32 BoundingRadius;
		
		GTSL::StaticVector<GAL::ShaderDataType, 20> VertexDescriptor;
	};
	
	struct StaticMeshDataSerialize : DataSerialize<StaticMeshData>
	{
		INSERT_START(StaticMeshDataSerialize)
		{
			INSERT_BODY;
			Insert(insertInfo.VertexSize, buffer);
			Insert(insertInfo.VertexCount, buffer);
			Insert(insertInfo.IndexSize, buffer);
			Insert(insertInfo.IndexCount, buffer);
			Insert(insertInfo.BoundingBox, buffer);
			Insert(insertInfo.BoundingRadius, buffer);
			Insert(insertInfo.VertexDescriptor, buffer);
		}

		EXTRACT_START(StaticMeshDataSerialize)
		{
			EXTRACT_BODY;
			Extract(extractInfo.VertexSize, buffer);
			Extract(extractInfo.VertexCount, buffer);
			Extract(extractInfo.IndexSize, buffer);
			Extract(extractInfo.IndexCount, buffer);
			Extract(extractInfo.BoundingBox, buffer);
			Extract(extractInfo.BoundingRadius, buffer);
			Extract(extractInfo.VertexDescriptor, buffer);
		}
	};

	struct StaticMeshInfo : Info<StaticMeshDataSerialize>
	{
		DECL_INFO_CONSTRUCTOR(StaticMeshInfo, Info<StaticMeshDataSerialize>);

		uint32 GetVerticesSize() const { return VertexSize * VertexCount; }
		uint32 GetIndicesSize() const { return IndexSize * IndexCount; }
	};

	template<typename... ARGS>
	void LoadStaticMeshInfo(ApplicationManager* gameInstance, Id meshName, DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadStaticMeshInfo = [](TaskInfo taskInfo, StaticMeshResourceManager* resourceManager, Id meshName, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			auto staticMeshInfoSerialize = resourceManager->meshInfos.At(meshName);

			StaticMeshInfo staticMeshInfo(meshName, staticMeshInfoSerialize);
			
			taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(staticMeshInfo), GTSL::ForwardRef<ARGS>(args)...);
		};

		gameInstance->AddDynamicTask(u8"StaticMeshResourceManager::loadStaticMeshInfo", Task<StaticMeshResourceManager*, Id, decltype(dynamicTaskHandle), ARGS...>::Create(loadStaticMeshInfo), {}, this, GTSL::MoveRef(meshName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadStaticMesh(ApplicationManager* gameInstance, StaticMeshInfo staticMeshInfo, uint32 indicesAlignment, GTSL::Range<byte*> buffer, DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS&&... args)
	{
		auto loadMesh = [](TaskInfo taskInfo, StaticMeshResourceManager* resourceManager, StaticMeshInfo staticMeshInfo, uint32 indicesAlignment, GTSL::Range<byte*> buffer, decltype(dynamicTaskHandle) dynamicTaskHandle, ARGS&&... args)
		{
			auto verticesSize = staticMeshInfo.GetVerticesSize(); auto indicesSize = staticMeshInfo.GetIndicesSize();
			
			BE_ASSERT(buffer.Bytes() >= GTSL::Math::RoundUpByPowerOf2(verticesSize, indicesAlignment) + indicesSize, u8"")
			
			byte* vertices = buffer.begin();
			byte* indices = GTSL::AlignPointer(indicesAlignment, vertices + verticesSize);
			
			GTSL::MemCopy(verticesSize, resourceManager->mappedFile.GetData() + staticMeshInfo.ByteOffset, vertices);
			GTSL::MemCopy(indicesSize, resourceManager->mappedFile.GetData() + staticMeshInfo.ByteOffset + verticesSize, indices);
			
			taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(resourceManager), GTSL::MoveRef(staticMeshInfo), GTSL::ForwardRef<ARGS>(args)...);
		};

		gameInstance->AddDynamicTask(u8"StaticMeshResourceManager::loadStaticMesh", Task<StaticMeshResourceManager*, StaticMeshInfo, uint32, GTSL::Range<byte*>, decltype(dynamicTaskHandle), ARGS...>::Create(loadMesh), {}, this, GTSL::MoveRef(staticMeshInfo), GTSL::MoveRef(indicesAlignment), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
private:
	GTSL::File indexFile;
	GTSL::MappedFile mappedFile;
	
	GTSL::HashMap<Id, StaticMeshDataSerialize, BE::PersistentAllocatorReference> meshInfos;

	void loadMesh(const GTSL::Buffer<BE::TAR>& sourceBuffer, StaticMeshDataSerialize& meshInfo, GTSL::Buffer<BE::TAR>& meshDataBuffer);
};
