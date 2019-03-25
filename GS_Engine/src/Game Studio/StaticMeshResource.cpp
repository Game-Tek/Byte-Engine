#include "StaticMeshResource.h"

#include "Logger.h"

#include "String.h"

#include <assimp/scene.h>
#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>

StaticMeshResource::StaticMeshResource(const String & Path) : Resource(Path)
{
	Data = Load(FilePath);
}

StaticMeshResource::~StaticMeshResource()
{
	delete static_cast<Mesh *>(Data);
}

Mesh * StaticMeshResource::Load(const String & Path)
{
	//Create Importer.

	Assimp::Importer Importer;

	//Create Scene and import file.
	const aiScene * Scene = Importer.ReadFile(FilePath.c_str(), aiProcess_Triangulate | aiProcess_FlipUVs |	aiProcess_JoinIdenticalVertices | aiProcess_GenSmoothNormals);

	if (!Scene || Scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE || !Scene->mRootNode)
	{
		GS_LOG_WARNING("Failed to load StaticMesh: %s", FilePath.c_str());
		return LoadFallbackResource();
	}

	//Create pointer for return.
	Mesh * Result = new Mesh[Scene->mNumMeshes];	//Create new array of meshes.

	for (uint32 i = 0; i < Scene->mNumMeshes; i++)
	{
		Result[i] = ProcessMesh(Scene->mMeshes[i]);
	}

	return Result;
}

Mesh * StaticMeshResource::ProcessNode(aiNode * Node, const aiScene * Scene)
{
	//Store inside MeshData a new Array of meshes.
	Mesh * MeshData = new Mesh[Node->mNumMeshes];

	// Loop through each of the node's meshes (if any)
	for (unsigned int m = 0; m < Node->mNumMeshes; m++)
	{
		//Create a insertholder to store the this scene's mesh at [m].
		aiMesh * Mesh = Scene->mMeshes[Node->mMeshes[m]];

		//Store in Data at [m] a pointer to the array of vertices created for this mesh.
		MeshData[m] = ProcessMesh(Mesh);

	}

	return MeshData;
}

Mesh StaticMeshResource::ProcessMesh(aiMesh * InMesh)
{
	//Create a mesh object to hold the mesh currently being processed.
	Mesh Result;

	//------------MESH SETUP------------

	//We allocate a new array of vertices big enough to hold the number of vertices in this mesh and assign it to the
	//pointer found inside the mesh.
	Result.VertexArray = new Vertex[InMesh->mNumVertices];

	//Set this mesh's vertex count as the number of vertices found in this mesh.
	Result.VertexCount = InMesh->mNumVertices;

	//------------MESH SETUP------------

	// Loop through each vertex.
	for (uint32 i = 0; i < InMesh->mNumVertices; i++)
	{
		// Positions
		Result.VertexArray[i].Position.X = InMesh->mVertices[i].x;
		Result.VertexArray[i].Position.Y = InMesh->mVertices[i].y;
		Result.VertexArray[i].Position.Z = InMesh->mVertices[i].z;

		// Normals
		Result.VertexArray[i].Normal.X = InMesh->mNormals[i].x;
		Result.VertexArray[i].Normal.Y = InMesh->mNormals[i].y;
		Result.VertexArray[i].Normal.Z = InMesh->mNormals[i].z;

		// Texture Coordinates
		if (InMesh->mTextureCoords[0]) //We check if the pointer to texture coords is valid. (Could be NULLPTR)
		{
			//A vertex can contain up to 8 different texture coordinates.
			//Here, we are making the asumption we won't be using a model with more than one texture coordinates.
			Result.VertexArray[i].TextCoord.U = InMesh->mTextureCoords[0][i].x;
			Result.VertexArray[i].TextCoord.V = InMesh->mTextureCoords[0][i].y;
		}

		// Tangent
		//Result.VertexArray[i].Tangent.X = InMesh->mTangents[i].x;
		//Result.VertexArray[i].Tangent.Y = InMesh->mTangents[i].y;
		//Result.VertexArray[i].Tangent.Z = InMesh->mTangents[i].z;
		
		/*
		// BiTangent
		Result.VertexArray[i].BiTangent.X = InMesh->mBitangents[i].x;
		Result.VertexArray[i].BiTangent.Y = InMesh->mBitangents[i].y;
		Result.VertexArray[i].BiTangent.Z = InMesh->mBitangents[i].z;
		*/
	}

	//We allocate a new array of unsigned ints big enough to hold the number of indices in this mesh and assign it to the
	//pointer found inside the mesh.
	Result.IndexArray = new uint32[InMesh->mNumFaces * 3];

	//Wow loop through each of the mesh's faces and retrieve the corresponding vertex indices.
	for (uint32 f = 0; f < InMesh->mNumFaces; f++)
	{
		const aiFace Face = InMesh->mFaces[f];

		// Retrieve all indices of the face and store them in the indices array.
		for (uint32 i = 0; i < Face.mNumIndices; i++)
		{
			Result.IndexArray[Result.IndexCount + i] = Face.mIndices[i];
		}

		//Update the vertex count by summing the number of indices that each face we loop through has.
		Result.IndexCount += Face.mNumIndices;
	}

	return Result;
}

Mesh * StaticMeshResource::LoadFallbackResource() const
{
	return new Mesh;
}