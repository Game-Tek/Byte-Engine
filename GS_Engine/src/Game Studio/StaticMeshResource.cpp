#include "StaticMeshResource.h"

#include "Logger.h"

#include <assimp/Importer.hpp>
#include <assimp/scene.h>
#include <assimp/postprocess.h>

StaticMeshResource::StaticMeshResource(const std::string & Path)
{
	Data = Load(Path.c_str());
}

StaticMeshResource::~StaticMeshResource()
{
	delete Data;
}

Mesh * StaticMeshResource::Load(const char * FilePath)
{
	//Create Importer.
	Assimp::Importer Importer;

	//Create Scene and import file.
	const aiScene * Scene = Importer.ReadFile(FilePath, aiProcess_Triangulate | aiProcess_FlipUVs);

	if (!Scene || Scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE || !Scene->mRootNode)
	{
		GS_LOG_WARNING("Failed to load StaticMesh: %s", FilePath);
		return LoadFallbackResource();
	}

	return (*ProcessNode(Scene->mRootNode, Scene));
}

Mesh ** StaticMeshResource::ProcessNode(aiNode * Node, const aiScene * Scene)
{
	//Store inside MeshData a pointer to a new Array of pointers to meshes.
	Mesh ** MeshData = new Mesh * [Node->mNumMeshes];

	// Loop through each of the node’s meshes (if any)
	for (unsigned int m = 0; m < Node->mNumMeshes; m++)
	{
		//Create a placeholder to store the this scene's mesh at [m].
		aiMesh * Mesh = Scene->mMeshes[Node->mMeshes[m]];

		//Store in Data at [m] a pointer to the array of vertices created for this mesh.
		MeshData[m] = ProcessMesh(Mesh);

	}

	return MeshData;
}

Mesh * StaticMeshResource::ProcessMesh(aiMesh * InMesh)
{
	Mesh * LocMesh = new Mesh();
	LocMesh->VertexArray = new Vertex[InMesh->mNumVertices];

	// Loop through each vertex.
	for (unsigned int i = 0; i < InMesh->mNumVertices; i++)
	{
		// Positions
		LocMesh->VertexArray[i].Position.X = InMesh->mVertices[i].x;
		LocMesh->VertexArray[i].Position.Y = InMesh->mVertices[i].y;
		LocMesh->VertexArray[i].Position.Z = InMesh->mVertices[i].z;

		// Normals
		LocMesh->VertexArray[i].Normal.X = InMesh->mNormals[i].x;
		LocMesh->VertexArray[i].Normal.Y = InMesh->mNormals[i].y;
		LocMesh->VertexArray[i].Normal.Z = InMesh->mNormals[i].z;

		// texture coordinates
		if (InMesh->mTextureCoords[0]) // does the mesh contain texture coordinates?
		{
			// a vertex can contain up to 8 different texture coordinates. We thus make the assumption that we won't 
			// use models where a vertex can have multiple texture coordinates so we always take the first set (0).
			LocMesh->VertexArray[i].TextCoord.U = InMesh->mTextureCoords[0][i].x;
			LocMesh->VertexArray[i].TextCoord.V = InMesh->mTextureCoords[0][i].y;
		}

		// Tangent
		LocMesh->VertexArray[i].Tangent.X = InMesh->mTangents[i].x;
		LocMesh->VertexArray[i].Tangent.Y = InMesh->mTangents[i].y;
		LocMesh->VertexArray[i].Tangent.Z = InMesh->mTangents[i].z;

		// BiTangent
		LocMesh->VertexArray[i].BiTangent.X = InMesh->mBitangents[i].x;
		LocMesh->VertexArray[i].BiTangent.Y = InMesh->mBitangents[i].y;
		LocMesh->VertexArray[i].BiTangent.Z = InMesh->mBitangents[i].z;
	}

	LocMesh->IndexArray = new unsigned int[InMesh->mNumFaces];

	// now wak through each of the mesh's faces (a face is a mesh its triangle) and retrieve the corresponding vertex indices.
	for (unsigned int f = 0; f < InMesh->mNumFaces; f++)
	{
		aiFace Face = InMesh->mFaces[f];

		// retrieve all indices of the face and store them in the indices vector
		for (unsigned int j = 0; j < Face.mNumIndices; j++)
		{
			LocMesh->IndexArray[j] = Face.mIndices[j];
		}
	}

	return LocMesh;
}

Mesh * StaticMeshResource::LoadFallbackResource()
{
	return new Mesh;
}