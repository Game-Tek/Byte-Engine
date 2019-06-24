#pragma once

#include "Core.h"

#include "FVector.hpp"
#include "String.h"

GS_STRUCT DataType
{
	//Size of the data type in bytes.
	uint8 Size = 0;
};

GS_CLASS VertexDescriptor
{
	FVector<DataType> Elements;
public:
	void AddElement(const DataType & _Element);
	const FVector<DataType>& GetElements() const { return Elements; }
};

struct Vertex;

//Describes all data necessary to create a mesh.
//    Pointer to an array holding the vertices that describe the mesh.
//        Vertex* VertexData;
//    Total number of vertices found in the VertexData array.
//        uint16 VertexCount;
//    Pointer to an array holding the indices that describe the mesh.
//        uint16* IndexData;
//    Total number of indices found in the IndexData array.
//        uint16 IndexCount;
//    A vertex descriptor that defines the layout of the vertices found in VertexData.
//        VertexDescriptor VertexLayout;
GS_STRUCT MeshCreateInfo
{
	//Pointer to an array holding the vertices that describe the mesh.
	Vertex* VertexData = nullptr;
	//Total number of vertices found in the VertexData array.
	uint16 VertexCount = 0;
	//Pointer to an array holding the indices that describe the mesh.
	uint16* IndexData = nullptr;
	//Total number of indices found in the IndexData array.
	uint16 IndexCount = 0;
	//A vertex descriptor that defines the layout of the vertices found in VertexData.
	VertexDescriptor VertexLayout;
};

GS_CLASS Mesh
{
public:
	virtual ~Mesh();
};