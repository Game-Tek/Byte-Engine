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
	unsigned int * IndexArray = nullptr;

	unsigned int VertexCount = 0;
	unsigned int IndexCount = 0;
};

GS_CLASS StaticMeshResource : public Resource<Mesh>
{
public:
	StaticMeshResource(const char * FilePath);
	~StaticMeshResource();

	unsigned int GetMeshIndexCount(unsigned int MeshIndex) const { return Data[MeshIndex].IndexCount; };
	unsigned int GetMeshVertexCount(unsigned int MeshIndex) const { return Data[MeshIndex].VertexCount; }

protected:
	Mesh * LoadFallbackResource() override;

private:
	Mesh * Load(const char * FilePath) override;
	Mesh ** ProcessNode(aiNode * Node, const aiScene * Scene);
	Mesh * ProcessMesh(aiMesh * Mesh);
};