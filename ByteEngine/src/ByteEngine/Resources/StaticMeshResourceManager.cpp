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

	GTSL::Vector<Mesh> meshes(4, GetTransientAllocator());
	
	GTSL::StaticString<512> query_path, package_path, resources_path;
	query_path += BE::Application::Get()->GetPathToApplication(); package_path += BE::Application::Get()->GetPathToApplication(); resources_path += BE::Application::Get()->GetPathToApplication();
	query_path += "/resources/"; package_path += "/resources/"; resources_path += "/resources/";
	query_path += "*.obj"; package_path += "static_meshes.smbepkg";

	GTSL::Buffer file_buffer;
	file_buffer.Allocate(1024 * 1024, 8, GetTransientAllocator());

	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, [&](const GTSL::FileQuery::QueryResult& queryResult)
	{
		auto file_path = resources_path;
		file_path += queryResult.FilePath;
		
		auto name = queryResult.FilePath; name.Drop(name.FindLast('.'));
		model_names.EmplaceBack(GetTransientAllocator(), name.operator GTSL::Ranger<const char>());
		model_files[model_files.EmplaceBack(GetTransientAllocator())].OpenFile(file_path, GTSL::File::OpenFileMode::READ);
	});
	
	GTSL::Buffer assimp_file_buffer; assimp_file_buffer.Allocate(1024 * 512, 8, GetTransientAllocator());
	
	for (uint32 mesh = 0; mesh < model_files.GetLength(); ++mesh)
	{
		
		meshes.EmplaceBack(GetTransientAllocator());
		meshes[mesh].Indeces.Initialize(255, GetTransientAllocator());
		meshes[mesh].VertexElements.Initialize(255, GetTransientAllocator());

		//auto file = model_files[mesh];
		
		model_files[mesh].ReadFile(assimp_file_buffer);

		Assimp::Importer importer;
		const auto* const ai_scene = importer.ReadFileFromMemory(assimp_file_buffer.GetData(), assimp_file_buffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
			aiProcess_JoinIdenticalVertices | aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_ImproveCacheLocality);
		
		BE_ASSERT(ai_scene != nullptr && !(ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE), "Error interpreting file!");
		
		aiMesh* in_mesh = ai_scene->mMeshes[0];
		
		MeshInfo mesh_info;
		
		//MESH ALWAYS HAS POSITIONS
		mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3)); mesh_info.VerticesSize = sizeof(GTSL::Vector3);
		for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
		{
			meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mVertices[vertex].x);
			meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mVertices[vertex].y);
			meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mVertices[vertex].z);
		}
		
		if (in_mesh->HasNormals())
		{
			mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3)); mesh_info.VerticesSize += sizeof(GTSL::Vector3);
			
			for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
			{
				meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mNormals[vertex].x);
				meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mNormals[vertex].y);
				meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mNormals[vertex].z);
			}
		}
		
		if (in_mesh->HasTangentsAndBitangents())
		{
			mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3)); mesh_info.VerticesSize += sizeof(GTSL::Vector3);
			mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT3)); mesh_info.VerticesSize += sizeof(GTSL::Vector3);
			
			for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
			{
			}
		}
			
		for (uint8 tex_coords = 0; tex_coords < 8; ++tex_coords)
		{
			if (in_mesh->HasTextureCoords(tex_coords))
			{
				mesh_info.VertexDescriptor.EmplaceBack(static_cast<uint8>(GAL::ShaderDataTypes::FLOAT2)); mesh_info.VerticesSize += sizeof(GTSL::Vector2);
				
				for (uint32 vertex = 0; vertex < in_mesh->mNumVertices; ++vertex)
				{
					meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mTextureCoords[tex_coords][vertex].x);
					meshes[mesh].VertexElements.EmplaceBack(GetTransientAllocator(), in_mesh->mTextureCoords[tex_coords][vertex].y);
				}
			}
		}
		
		for (uint32 face = 0; face < in_mesh->mNumFaces; ++face)
		{
			for (uint32 index = 0; index < in_mesh->mFaces[face].mNumIndices; index++)
			{
				meshes[mesh].Indeces.EmplaceBack(GetTransientAllocator(), in_mesh->mFaces[face].mIndices[index]);
			}
		}
		
		mesh_info.IndecesSize = in_mesh->mNumFaces * 3 * sizeof(uint32);
		mesh_info.MeshSize = meshes[mesh].SerializedSize();
		
		meshInfos.Emplace(GetPersistentAllocator(), model_names[mesh], mesh_info);
		
		assimp_file_buffer.Resize(0);
	}
	assimp_file_buffer.Free(8, GetTransientAllocator());

	//GTSL::Insert(meshInfos, file_buffer, GetTransientAllocator());
	for (auto& e : meshes) { Insert(e, file_buffer, GetTransientAllocator()); }

	staticMeshPackage.OpenFile(package_path, GTSL::File::OpenFileMode::WRITE);
	staticMeshPackage.WriteToFile(file_buffer);
	staticMeshPackage.CloseFile();
	staticMeshPackage.OpenFile(package_path, GTSL::File::OpenFileMode::READ);

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
}

StaticMeshResourceManager::~StaticMeshResourceManager()
{
	staticMeshPackage.CloseFile();
}

void StaticMeshResourceManager::LoadStaticMesh(const LoadStaticMeshInfo& loadStaticMeshInfo)
{
	const auto mesh_info = meshInfos.At(GTSL::Id64(loadStaticMeshInfo.Name.begin()));

	uint64 n;
	staticMeshPackage.SetPointer(mesh_info.MeshByteOffset, n, GTSL::File::MoveFrom::BEGIN);

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

void StaticMeshResourceManager::GetMeshSize(const GTSL::StaticString<256>& name, uint32& meshSize)
{
	meshSize = meshInfos.At(GTSL::Id64(name.begin())).MeshSize;
}

void Insert(const StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference)
{
	GTSL::Insert(meshInfo.MeshSize, buffer, allocatorReference);
	GTSL::Insert(meshInfo.VerticesSize, buffer, allocatorReference);
	GTSL::Insert(meshInfo.IndecesSize, buffer, allocatorReference);
	GTSL::Insert(meshInfo.MeshByteOffset, buffer, allocatorReference);
}

void Extract(StaticMeshResourceManager::MeshInfo& meshInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference)
{
	GTSL::Extract(meshInfo.MeshSize, buffer, allocatorReference);
	GTSL::Extract(meshInfo.VerticesSize, buffer, allocatorReference);
	GTSL::Extract(meshInfo.IndecesSize, buffer, allocatorReference);
	GTSL::Extract(meshInfo.MeshByteOffset, buffer, allocatorReference);
}

void Insert(const StaticMeshResourceManager::Mesh& mesh, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference)
{
	buffer.WriteBytes(mesh.VertexElements.GetLengthSize(), (byte*)mesh.VertexElements.begin());
	buffer.WriteBytes(mesh.Indeces.GetLengthSize(), (byte*)mesh.Indeces.begin());
}