#pragma once

#include "Core.h"

#include "Resource.h"

#include "Vertex.h"

//Used to specify a single mesh. Contains a pointer to an array of vertices, and a pointer to an array of indices.
struct Mesh
{
	//Pointer to Vertex Array.
	Vertex * VertexArray = nullptr;
	//Pointer to index array.
	uint32 * IndexArray = nullptr;

	//Vertex Count.
	uint32 VertexCount = 0;
	//Index Count.
	uint32 IndexCount = 0;
};

class String;

struct aiNode;
struct aiMesh;
class aiScene;

GS_CLASS StaticMeshResource : public Resource
{
public:
	explicit StaticMeshResource(const String & Path);
	~StaticMeshResource();

	Vertex * GetVertexArray() const { return static_cast<Mesh *>(Data)->VertexArray; }
	uint32 * GetIndexArray() const { return static_cast<Mesh *>(Data)->IndexArray; }

	size_t GetDataSize() const override { return sizeof(*static_cast<Mesh*>(Data)); }

	uint32 GetMeshIndexCount(uint8 MeshIndex) const { return static_cast<Mesh *>(Data)[MeshIndex].IndexCount; };
	uint32 GetMeshVertexCount(uint8 MeshIndex) const { return static_cast<Mesh *>(Data)[MeshIndex].VertexCount; }

private:
	Mesh * Load(const char * FilePath);
	Mesh * LoadFallbackResource() const;
	Mesh * ProcessNode(aiNode * Node, const aiScene * Scene);
	Mesh ProcessMesh(aiMesh * Mesh);
};