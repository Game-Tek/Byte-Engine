#include "StaticMeshResourceManager.h"

#include <GTSL/Buffer.h>

#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>
#include <assimp/scene.h>
#include <GAL/RenderCore.h>


#include "ByteEngine/Application/Application.h"

#include "ByteEngine/Vertex.h"

#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.h>

#include "ByteEngine/Debug/Assert.h"

StaticMeshResourceManager::StaticMeshResourceManager() : SubResourceManager("Static Mesh"), meshInfos(4, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, package_path, resources_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	package_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	query_path += "/resources/*.obj";
	package_path += "/resources/StaticMeshes.bepkg";
	resources_path += "/resources/";

	GTSL::Buffer source_file_buffer; source_file_buffer.Allocate(2048 * 2048, 32, GetTransientAllocator());
	GTSL::Buffer file_buffer; file_buffer.Allocate(2048 * 2048, 32, GetTransientAllocator());

	staticMeshPackage.OpenFile(package_path, (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::CLEAR);
	
	auto load = [&](const GTSL::FileQuery::QueryResult& queryResult)
	{
		auto file_path = resources_path;
		file_path += queryResult.FileNameWithExtension;
		auto name = queryResult.FileNameWithExtension; name.Drop(name.FindLast('.'));
		const auto hashed_name = GTSL::Id64(name.operator GTSL::Ranger<const char>());

		GTSL::File query_file;
		query_file.OpenFile(file_path, static_cast<uint8>(GTSL::File::AccessMode::READ), GTSL::File::OpenMode::LEAVE_CONTENTS);

		query_file.ReadFile(source_file_buffer);

		MeshInfo mesh_info; Mesh mesh;
		mesh.Indeces.Initialize(1024, GetTransientAllocator());
		mesh.VertexElements.Initialize(1024, GetTransientAllocator());

		loadMesh(source_file_buffer, mesh_info, mesh, GetTransientAllocator());

		mesh_info.ByteOffsetFromEndOfFile = static_cast<uint32>(staticMeshPackage.GetFileSize());

		Insert(mesh, file_buffer, GetTransientAllocator());
		staticMeshPackage.WriteToFile(file_buffer);
		
		meshInfos.Emplace(GetPersistentAllocator(), hashed_name, mesh_info);

		mesh.Indeces.Free(GetTransientAllocator());
		mesh.VertexElements.Free(GetTransientAllocator());

		query_file.CloseFile();
	};
	
	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, load);
	
	source_file_buffer.Free(32, GetTransientAllocator());
	file_buffer.Free(32, GetTransientAllocator());
}

StaticMeshResourceManager::~StaticMeshResourceManager()
{
	staticMeshPackage.CloseFile();
	meshInfos.Free(GetPersistentAllocator());
}

void StaticMeshResourceManager::LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo)
{
	const auto meshInfo = meshInfos.At(loadStaticMeshInfo.Name);

	staticMeshPackage.SetPointer(-(int64)meshInfo.ByteOffsetFromEndOfFile, GTSL::File::MoveFrom::END);

	byte* vertices = loadStaticMeshInfo.MeshDataBuffer, *indices = static_cast<byte*>(GTSL::AlignPointer(loadStaticMeshInfo.IndicesAlignment, vertices + meshInfo.VerticesSize));
	
	staticMeshPackage.ReadFromFile(loadStaticMeshInfo.MeshDataBuffer); 

	GTSL::MemCopy(meshInfo.IndecesSize, vertices + meshInfo.VerticesSize, indices);
	
	//OnStaticMeshLoad on_static_mesh_load;
	//on_static_mesh_load.Vertex = vertices;
	//on_static_mesh_load.Indices = indeces;
	//on_static_mesh_load.IndexCount = index_count;
	//on_static_mesh_load.VertexCount = InMesh->mNumVertices;
	//on_static_mesh_load.MeshDataBuffer = GTSL::Ranger<byte>(range.begin(), reinterpret_cast<byte*>(indeces + index_count));
	//loadStaticMeshInfo.OnStaticMeshLoad(on_static_mesh_load);
}

void StaticMeshResourceManager::GetMeshSize(const GTSL::Id64 name, const uint32 alignment, uint32& meshSize)
{
	auto& mesh = meshInfos.At(name);
	meshSize = GTSL::Math::PowerOf2RoundUp(mesh.VerticesSize, alignment) + mesh.IndecesSize;
}

void StaticMeshResourceManager::loadMesh(const GTSL::Buffer& sourceBuffer, MeshInfo& meshInfo, Mesh& mesh, const GTSL::AllocatorReference& allocatorReference)
{
	Assimp::Importer importer;
	const auto* const ai_scene = importer.ReadFileFromMemory(sourceBuffer.GetData(), sourceBuffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
		aiProcess_JoinIdenticalVertices | aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_ImproveCacheLocality);

	BE_ASSERT(ai_scene != nullptr && !(ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE), "Error interpreting file!");

	aiMesh* in_mesh = ai_scene->mMeshes[0];

	//MESH ALWAYS HAS POSITIONS
	meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3));
	for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
	{
		mesh.VertexElements.EmplaceBack(allocatorReference, in_mesh->mVertices[vertex].x);
		mesh.VertexElements.EmplaceBack(allocatorReference, in_mesh->mVertices[vertex].y);
		mesh.VertexElements.EmplaceBack(allocatorReference, in_mesh->mVertices[vertex].z);
	}
	meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;

	if (in_mesh->HasNormals())
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3));

		for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
		{
			mesh.VertexElements.EmplaceBack(allocatorReference, in_mesh->mNormals[vertex].x);
			mesh.VertexElements.EmplaceBack(allocatorReference, in_mesh->mNormals[vertex].y);
			mesh.VertexElements.EmplaceBack(allocatorReference, in_mesh->mNormals[vertex].z);

		}

		meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;
	}

	if (in_mesh->HasTangentsAndBitangents())
	{
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3));
		meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3));

		for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
		{
		}
		meshInfo.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices * 2;
	}

	for (uint8 tex_coords = 0; tex_coords < 8; ++tex_coords)
	{
		if (in_mesh->HasTextureCoords(tex_coords))
		{
			meshInfo.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT2));

			for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
			{
				mesh.VertexElements.EmplaceBack(allocatorReference, in_mesh->mTextureCoords[tex_coords][vertex].x);
				mesh.VertexElements.EmplaceBack(allocatorReference, in_mesh->mTextureCoords[tex_coords][vertex].y);
			}

			meshInfo.VerticesSize += sizeof(GTSL::Vector2) * in_mesh->mNumVertices;
		}
	}

	for (uint32 face = 0; face < in_mesh->mNumFaces; ++face)
	{
		for (uint32 index = 0; index < in_mesh->mFaces[face].mNumIndices; ++index)
		{
			mesh.Indeces.EmplaceBack(allocatorReference, in_mesh->mFaces[face].mIndices[index]);
		}
	}

	meshInfo.IndecesSize = in_mesh->mNumFaces * 3 * sizeof(uint32);
}

void Insert(const StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference)
{
	GTSL::Insert(meshInfo.VerticesSize, buffer, allocatorReference);
	GTSL::Insert(meshInfo.IndecesSize, buffer, allocatorReference);
	GTSL::Insert(meshInfo.ByteOffsetFromEndOfFile, buffer, allocatorReference);
}

void Extract(StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference)
{
	GTSL::Extract(meshInfo.VerticesSize, buffer, allocatorReference);
	GTSL::Extract(meshInfo.IndecesSize, buffer, allocatorReference);
	GTSL::Extract(meshInfo.ByteOffsetFromEndOfFile, buffer, allocatorReference);
}

void Insert(const StaticMeshResourceManager::Mesh& mesh, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference)
{
	buffer.WriteBytes(mesh.VertexElements.GetLengthSize(), (byte*)mesh.VertexElements.begin());
	buffer.WriteBytes(mesh.Indeces.GetLengthSize(), (byte*)mesh.Indeces.begin());
}