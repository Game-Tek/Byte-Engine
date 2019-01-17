#include "StaticMeshResource.h"

StaticMeshResource::StaticMeshResource(const char * FilePath)
{
	Data = Load(FilePath);
}

StaticMeshResource::~StaticMeshResource()
{
	delete Data;
}

void * StaticMeshResource::Load(const char * FilePath)
{
	((Vertex*)Data)[5];
}

Vertex * StaticMeshResource::AllocateNewArray(unsigned int NumberOfVertices)
{
	return new Vertex[NumberOfVertices];
}