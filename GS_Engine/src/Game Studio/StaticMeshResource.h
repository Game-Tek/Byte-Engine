#pragma once

#include "Core.h"

#include "Resource.h"

#include "Vertex.h"

GS_CLASS StaticMeshResource : public Resource
{
public:
	StaticMeshResource(const char * FilePath);
	~StaticMeshResource();

	unsigned int GetIndexCount() const { return IndexCount; };
	unsigned int GetVertexCount() const { return VertexCount; }

private:
	unsigned int IndexCount;
	unsigned int VertexCount;

	void * Load(const char * FilePath) override;
	Vertex * AllocateNewArray(unsigned int NumberOfVertices);
};