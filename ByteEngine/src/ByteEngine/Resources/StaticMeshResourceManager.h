#pragma once

#include "ResourceManager.h"

#include <GTSL/Delegate.hpp>
#include <GTSL/FlatHashMap.h>
#include <GTSL/File.h>
#include <GTSL/Vector.hpp>

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
		GTSL::uint32 VertexCount;
		/**
		 * \brief Number of indeces the loaded mesh contains. Every face can only have three indeces.
		 */
		GTSL::uint16 IndexCount;
	};

	struct LoadStaticMeshInfo : ResourceLoadInfo
	{
		GTSL::Delegate<void(OnStaticMeshLoad)> OnStaticMeshLoad;
		uint32 IndicesAlignment = 0;
	};
	void LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo);

	void GetMeshSize(GTSL::Id64 name, uint32 alignment, uint32& meshSize);

	struct Mesh
	{
		GTSL::Vector<float32> VertexElements;
		GTSL::Vector<uint32> Indeces;
		
		friend void Insert(const Mesh& mesh, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference);	
	};

	struct MeshInfo
	{
		GTSL::Array<uint8, 32> VertexDescriptor;
		uint32 VerticesSize = 0;
		uint32 IndecesSize = 0;
		uint32 ByteOffset = 0;

		static uint64 VertexDescriptorHash(GTSL::Ranger<uint8> descriptors)
		{
			for(auto e : descriptors)
			{
				
			}
		}

		[[nodiscard]] uint32 MeshSize()const { return VerticesSize + IndecesSize; }
		
		friend void Insert(const MeshInfo& meshInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference);
		friend void Extract(MeshInfo& meshInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference);
	};
	
private:
	GTSL::FlatHashMap<OnStaticMeshLoad> resources;
	GTSL::File staticMeshPackage, indexFile;
	
	GTSL::FlatHashMap<MeshInfo> meshInfos;

	static void loadMesh(const GTSL::Buffer& sourceBuffer, MeshInfo& meshInfo, Mesh& mesh, const GTSL::AllocatorReference& allocatorReference);
};
