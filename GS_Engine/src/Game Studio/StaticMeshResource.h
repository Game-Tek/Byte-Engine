#pragma once

#include "Core.h"

#include "Resource.h"

#include "Vertex.h"

#include <assimp/Importer.hpp>
#include <assimp/scene.h>
#include <assimp/postprocess.h>

//Used to specify a single mesh. Contains a pointer to an array of vertices, and a pointer to an array of indices.
struct Mesh
{
	Vertex * VertexArray = nullptr;
	uint32 * IndexArray = nullptr;

	uint32 VertexCount = 0;
	uint32 IndexCount = 0;
};

GS_CLASS StaticMeshResource : public Resource
{
public:
	StaticMeshResource(const std::string & Path);
	~StaticMeshResource();

	Mesh * GetMeshData() const { return ((Mesh *)Data); }
	size_t GetDataSize() const override { return sizeof(*((Mesh*)(Data))); }

	uint32 GetMeshIndexCount(uint8 MeshIndex) const { return ((Mesh *)Data)[MeshIndex].IndexCount; };
	uint32 GetMeshVertexCount(uint8 MeshIndex) const { return ((Mesh *)Data)[MeshIndex].VertexCount; }

private:
	Mesh * Load(const char * FilePath);
	Mesh * LoadFallbackResource();
	Mesh ** ProcessNode(aiNode * Node, const aiScene * Scene);
	Mesh * ProcessMesh(aiMesh * Mesh);
};