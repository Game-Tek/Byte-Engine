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
	GTSL::Vector<GTSL::File> model_files(4, GetTransientAllocator());
	GTSL::Vector<GTSL::Id64> model_names(4, GetTransientAllocator());
	GTSL::Vector<MeshInfo> mesh_infos(4, GetTransientAllocator());
	GTSL::Vector<Mesh> meshes(4, GetTransientAllocator());
	
	GTSL::StaticString<512> query_path, package_path, resources_path;
	query_path += BE::Application::Get()->GetPathToApplication(); package_path += BE::Application::Get()->GetPathToApplication(); resources_path += BE::Application::Get()->GetPathToApplication();
	query_path += "/resources/"; package_path += "/resources/"; resources_path += "/resources/";
	query_path += "*.obj"; package_path += "static_meshes.smbepkg";

	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, [&](const GTSL::FileQuery::QueryResult& queryResult)
	{
		auto file_path = resources_path;
		file_path += queryResult.FilePath;
		
		auto name = queryResult.FilePath; name.Drop(name.FindLast('.'));
		model_names.EmplaceBack(GetTransientAllocator(), name.operator GTSL::Ranger<const char>());
		model_files[model_files.EmplaceBack(GetTransientAllocator())].OpenFile(file_path, (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	});
	
	GTSL::Buffer assimp_file_buffer; assimp_file_buffer.Allocate(1024 * 512, 8, GetTransientAllocator());
	
	for (uint32 mesh = 0; mesh < model_files.GetLength(); ++mesh)
	{
		meshes.EmplaceBack(GetTransientAllocator());
		meshes[mesh].VertexElements.Initialize(1024, GetTransientAllocator());
		meshes[mesh].Indeces.Initialize(1024, GetTransientAllocator());
		
		model_files[mesh].ReadFile(assimp_file_buffer);

		Assimp::Importer importer;
		const auto* const ai_scene = importer.ReadFileFromMemory(assimp_file_buffer.GetData(), assimp_file_buffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
			aiProcess_JoinIdenticalVertices | aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_ImproveCacheLocality);
		
		BE_ASSERT(ai_scene != nullptr && !(ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE), "Error interpreting file!");
		
		aiMesh* in_mesh = ai_scene->mMeshes[0];
		
		MeshInfo mesh_info;
		
		//MESH ALWAYS HAS POSITIONS
		mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3));
		for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
		{
			meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mVertices[vertex].x);
			meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mVertices[vertex].y);
			meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mVertices[vertex].z);
		}
		mesh_info.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;
		
		if (in_mesh->HasNormals())
		{
			mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3));
			
			for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
			{
				meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mNormals[vertex].x);
				meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mNormals[vertex].y);
				meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mNormals[vertex].z);

			}
			
			mesh_info.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices;
		}
		
		if (in_mesh->HasTangentsAndBitangents())
		{
			mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3));
			mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3));
			
			for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
			{
			}
			mesh_info.VerticesSize += sizeof(GTSL::Vector3) * in_mesh->mNumVertices * 2;
		}
			
		for (uint8 tex_coords = 0; tex_coords < 8; ++tex_coords)
		{
			if (in_mesh->HasTextureCoords(tex_coords))
			{
				mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT2));
				
				for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
				{
					meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mTextureCoords[tex_coords][vertex].x);
					meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mTextureCoords[tex_coords][vertex].y);
				}
				
				mesh_info.VerticesSize += sizeof(GTSL::Vector2) * in_mesh->mNumVertices;
			}
		}
		
		for (uint32 face = 0; face < in_mesh->mNumFaces; ++face)
		{
			for (uint32 index = 0; index < in_mesh->mFaces[face].mNumIndices; ++index)
			{
				meshes[mesh].Indeces.EmplaceBack(GetTransientAllocator(), in_mesh->mFaces[face].mIndices[index]);
			}
		}
		
		mesh_info.IndecesSize = in_mesh->mNumFaces * 3 * sizeof(uint32);
		
		mesh_infos.EmplaceBack(GetPersistentAllocator(), mesh_info);
		
		assimp_file_buffer.Resize(0);
	}
	assimp_file_buffer.Free(8, GetTransientAllocator());

	GTSL::Buffer file_buffer; file_buffer.Allocate(1024 * 1024, 8, GetTransientAllocator());
	
	uint64 mesh_infos_size = 0;
	
	for(uint32 i = meshes.GetLength() - 1; i > 0; --i)
	{
		auto& e = mesh_infos[i];
		mesh_infos_size += e.MeshSize();
		e.ByteOffsetFromEndOfFile = mesh_infos_size;
	}

	for (uint32 i = 0; i < meshes.GetLength(); ++i) { meshInfos.Emplace(GetPersistentAllocator(), model_names[i], mesh_infos[i]); }
	
	GTSL::Insert(meshInfos, file_buffer, GetTransientAllocator());
	for (auto& e : meshes) { Insert(e, file_buffer, GetTransientAllocator()); }

	staticMeshPackage.OpenFile(package_path, (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::LEAVE_CONTENTS);
	staticMeshPackage.WriteToFile(file_buffer);
	staticMeshPackage.CloseFile();
	staticMeshPackage.OpenFile(package_path, (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);

	for (auto& mesh : meshes)
	{
		mesh.Indeces.Free(GetTransientAllocator());
		mesh.VertexElements.Free(GetTransientAllocator());
	}
	meshes.Free(GetTransientAllocator());
	
	file_buffer.Free(8, GetTransientAllocator());
	for (auto& e : model_files) { e.CloseFile(); }
	model_files.Free(GetTransientAllocator());
	model_names.Free(GetTransientAllocator());
	mesh_infos.Free(GetTransientAllocator());
}

StaticMeshResourceManager::~StaticMeshResourceManager()
{
	staticMeshPackage.CloseFile();
	meshInfos.Free(GetPersistentAllocator());
}

void StaticMeshResourceManager::LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo)
{
	const auto mesh_info = meshInfos.At(loadStaticMeshInfo.Name);

	staticMeshPackage.SetPointer(-(int64)mesh_info.ByteOffsetFromEndOfFile, GTSL::File::MoveFrom::END);

	byte* vertices = loadStaticMeshInfo.MeshDataBuffer, *indices = static_cast<byte*>(GTSL::AlignPointer(loadStaticMeshInfo.IndicesAlignment, vertices + mesh_info.VerticesSize));
	
	staticMeshPackage.ReadFromFile(loadStaticMeshInfo.MeshDataBuffer); 

	GTSL::MemCopy(mesh_info.IndecesSize, vertices + mesh_info.VerticesSize, indices);
	
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