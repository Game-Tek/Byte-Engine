#pragma once

#include <GTSL/Array.hpp>

#include "ResourceManager.h"

#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/File.h>
#include <GTSL/Vector.hpp>

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
	
	struct OnStaticMeshLoad : OnResourceLoad
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

		GTSL::Array<GAL::ShaderDataType, 20> VertexDescriptor;
	};

	struct LoadStaticMeshInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(TaskInfo, OnStaticMeshLoad)> OnStaticMeshLoad;
		uint32 IndicesAlignment = 0;
	};
	void LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo);

	void GetMeshSize(const GTSL::Id64 name, uint32* vertexCount, uint32* vertexSize, uint32* indexCount, uint32* indexSize);

	struct MeshInfo
	{
		GTSL::Array<GAL::ShaderDataType, 20> VertexDescriptor;
		uint32 VertexCount = 0;
		uint32 IndexCount = 0;
		uint32 ByteOffset = 0;
		uint8 IndexSize = 0;

		[[nodiscard]] uint32 MeshSize()const { return VertexCount + IndexCount; }
		
		friend void Insert(const MeshInfo& meshInfo, GTSL::Buffer& buffer);
		friend void Extract(MeshInfo& meshInfo, GTSL::Buffer& buffer);
	};
	
private:
	GTSL::FlatHashMap<OnStaticMeshLoad, BE::PersistentAllocatorReference> resources;
	GTSL::File staticMeshPackage, indexFile;
	
	GTSL::FlatHashMap<MeshInfo, BE::PersistentAllocatorReference> meshInfos;

	static void loadMesh(const GTSL::Buffer& sourceBuffer, MeshInfo& meshInfo, GTSL::Buffer& mesh);
};
