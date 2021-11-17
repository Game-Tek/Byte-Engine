#include "StaticMeshResourceManager.h"

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"

#include <assimp/Importer.hpp>
#include <assimp/postprocess.h>
#include <assimp/scene.h>
#include <GAL/Pipelines.h>

#include <GTSL/Buffer.hpp>
#include <GAL/RenderCore.h>
#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.hpp>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Math/Vectors.h>

static GTSL::Vector4 toAssimp(const aiColor4D assimpVector) {
	return GTSL::Vector4(assimpVector.r, assimpVector.g, assimpVector.b, assimpVector.a);
}

static GTSL::Vector3 toAssimp(const aiVector3D assimpVector) {
	return GTSL::Vector3(assimpVector.x, assimpVector.y, assimpVector.z);
}

static GTSL::Vector2 toAssimp(const aiVector2D assimpVector) {
	return GTSL::Vector2(assimpVector.x, assimpVector.y);
}

using ShaderDataTypeType = GTSL::UnderlyingType<GAL::ShaderDataType>;

StaticMeshResourceManager::StaticMeshResourceManager(const InitializeInfo& initialize_info) : ResourceManager(initialize_info, u8"StaticMeshResourceManager"), meshInfos(4, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, resources_path, index_path;
	query_path += BE::Application::Get()->GetPathToApplication();
	resources_path += BE::Application::Get()->GetPathToApplication();
	index_path += BE::Application::Get()->GetPathToApplication();
	query_path += u8"/resources/*.obj";
	index_path += u8"/resources/StaticMesh.beidx";
	resources_path += u8"/resources/";

	auto package_path = GetResourcePath(GTSL::ShortString<32>(u8"StaticMesh"), GTSL::ShortString<32>(u8"bepkg"));
	
	switch (indexFile.Open(index_path, GTSL::File::WRITE | GTSL::File::READ, true))
	{
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	default: ;
	}
	
	if (indexFile.GetSize())
	{
		GTSL::Buffer meshInfosFileBuffer(indexFile.GetSize(), 16, GetTransientAllocator());
		indexFile.Read(meshInfosFileBuffer);
		Extract(meshInfos, meshInfosFileBuffer);
	} else {
		GTSL::File staticMeshPackage;
		switch (staticMeshPackage.Open(package_path, GTSL::File::WRITE, true))
		{
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		default: ;
		}

		GTSL::FileQuery file_query;
		while(auto queryResult = file_query.DoQuery(query_path))
		{
			auto file_path = resources_path;
			file_path += queryResult.Get();
			auto fileName = queryResult.Get(); DropLast(fileName, u8'.');
			const auto hashed_name = GTSL::Id64(fileName);

			if (!meshInfos.Find(hashed_name)) {
				GTSL::File queryFile;
				queryFile.Open(file_path, GTSL::File::READ, false);
				
				GTSL::Buffer meshFileBuffer(queryFile.GetSize(), 32, GetTransientAllocator());
				queryFile.Read(meshFileBuffer);

				GTSL::Buffer meshDataBuffer(2048 * 2048, 8, GetTransientAllocator());

				StaticMeshDataSerialize meshInfo;

				loadMesh(meshFileBuffer, meshInfo, meshDataBuffer);

				meshInfo.ByteOffset = static_cast<uint32>(staticMeshPackage.GetSize());

				staticMeshPackage.Write(meshDataBuffer);

				meshInfos.Emplace(hashed_name, meshInfo);
			}
		}

		GTSL::Buffer<BE::TAR> meshInfosFileBuffer(4096, 16, GetTransientAllocator());
		Insert(meshInfos, meshInfosFileBuffer);

		indexFile.Write(meshInfosFileBuffer);
	}

	mappedFile.Open(package_path, 1024*1024*1024, GTSL::File::READ);
}

StaticMeshResourceManager::~StaticMeshResourceManager()
{
}

void StaticMeshResourceManager::loadMesh(const GTSL::Buffer<BE::TAR>& sourceBuffer, StaticMeshDataSerialize& meshInfo, GTSL::Buffer<BE::TAR>& meshDataBuffer)
{
	Assimp::Importer importer;
	const auto* const ai_scene = importer.ReadFileFromMemory(sourceBuffer.GetData(), sourceBuffer.GetLength(), aiProcess_Triangulate | aiProcess_FlipUVs |
		aiProcess_CalcTangentSpace | aiProcess_GenSmoothNormals | aiProcess_JoinIdenticalVertices | aiProcess_MakeLeftHanded | aiProcess_FlipWindingOrder, "obj");

	if (!ai_scene || (ai_scene->mFlags & AI_SCENE_FLAGS_INCOMPLETE)) {
		BE_LOG_ERROR(reinterpret_cast<const char8_t*>(importer.GetErrorString()));
		BE_ASSERT(false, "Error interpreting file!");
	}

	if (!ai_scene->mMeshes) { BE_ASSERT(false, ""); return; }

	aiMesh* inMesh = ai_scene->mMeshes[0];

	meshInfo.VertexCount = inMesh->mNumVertices;

	//MESH ALWAYS HAS POSITIONS
	meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);

	if (inMesh->HasNormals()) {
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
	}

	if (inMesh->HasTangentsAndBitangents())
	{
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
	}

	for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(inMesh->GetNumUVChannels()); ++tex_coords) {
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT2);
	}

	for (uint8 colors = 0; colors < static_cast<uint8>(inMesh->GetNumColorChannels()); ++colors) {
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT4);
	}

	if (false) {
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::INT);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::INT);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::INT);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::INT);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT);
	}

	meshInfo.VertexSize = GAL::GraphicsPipeline::GetVertexSize(meshInfo.VertexDescriptor);

	meshInfo.BoundingBox = GTSL::Vector3(); meshInfo.BoundingRadius = 0.0f;

	meshDataBuffer.Resize(meshInfo.VertexSize * inMesh->mNumVertices);

	byte* dataPointer = meshDataBuffer.GetData(); uint32 elementIndex = 0;
	meshDataBuffer.AddBytes(meshInfo.VertexSize * inMesh->mNumVertices);

	auto advanceVertexElement = [&]() {
		return dataPointer + GAL::GraphicsPipeline::GetByteOffsetToMember(elementIndex++, meshInfo.VertexDescriptor);
	};

	auto writeElement = [&]<typename T>(T* elementPointer, const T& obj, const uint32 elementIndex) -> void {
		*reinterpret_cast<T*>(reinterpret_cast<byte*>(elementPointer) + (elementIndex * meshInfo.VertexSize)) = obj;
	};

	{
		auto* positions = reinterpret_cast<GTSL::Vector3*>(advanceVertexElement());
		
		for (uint64 vertex = 0; vertex < inMesh->mNumVertices; ++vertex)
		{
			auto vertexPosition = toAssimp(inMesh->mVertices[vertex]);
			meshInfo.BoundingBox = GTSL::Math::Max(meshInfo.BoundingBox, GTSL::Math::Abs(vertexPosition));
			meshInfo.BoundingRadius = GTSL::Math::Max(meshInfo.BoundingRadius, GTSL::Math::Length(vertexPosition));
			writeElement(positions, vertexPosition, vertex);
		}
	}

	if (inMesh->HasNormals()) {
		GTSL::Vector3* normals = reinterpret_cast<GTSL::Vector3*>(advanceVertexElement());
		
		for (uint64 vertex = 0; vertex < inMesh->mNumVertices; ++vertex) {
			writeElement(normals, toAssimp(inMesh->mNormals[vertex]), vertex);
		}
	}

	if (inMesh->HasTangentsAndBitangents()) {
		GTSL::Vector3* tangents = reinterpret_cast<GTSL::Vector3*>(advanceVertexElement());
		
		for (uint64 vertex = 0; vertex < inMesh->mNumVertices; ++vertex) {
			writeElement(tangents, toAssimp(inMesh->mTangents[vertex]), vertex);
		}
		
		GTSL::Vector3* bitangents = reinterpret_cast<GTSL::Vector3*>(advanceVertexElement());

		for (uint64 vertex = 0; vertex < inMesh->mNumVertices; ++vertex) {
			writeElement(bitangents, toAssimp(inMesh->mBitangents[vertex]), vertex);
		}
	}

	for(uint8 i = 0; i < 8; ++i) {
		if (inMesh->HasTextureCoords(i)) {
			GTSL::Vector2* textureCoordinates = reinterpret_cast<GTSL::Vector2*>(advanceVertexElement());
			
			for (uint64 vertex = 0; vertex < inMesh->mNumVertices; ++vertex) {
				writeElement(textureCoordinates, GTSL::Vector2(toAssimp(inMesh->mTextureCoords[i][vertex])), vertex);
			}
		}
	}

	for(uint8 i = 0; i < 8; ++i) {
		if (inMesh->HasVertexColors(i)) {
			GTSL::Vector4* colors = reinterpret_cast<GTSL::Vector4*>(advanceVertexElement());
			
			for (uint64 vertex = 0; vertex < inMesh->mNumVertices; ++vertex) {
				writeElement(colors, toAssimp(inMesh->mColors[i][vertex]), vertex);
			}
		}
	}

	//if(false) {
	//	uint32* index[4];
	//	float32* weight[4];
	//	index[0] = reinterpret_cast<uint32*>(advanceVertexElement());
	//	weight[0] = reinterpret_cast<float32*>(advanceVertexElement());
	//	
	//	index[1] = reinterpret_cast<uint32*>(advanceVertexElement());
	//	weight[1] = reinterpret_cast<float32*>(advanceVertexElement());
	//	
	//	index[2] = reinterpret_cast<uint32*>(advanceVertexElement());
	//	weight[2] = reinterpret_cast<float32*>(advanceVertexElement());
	//	
	//	index[3] = reinterpret_cast<uint32*>(advanceVertexElement());
	//	weight[3] = reinterpret_cast<float32*>(advanceVertexElement());
	//
	//	for (uint64 vertex = 0; vertex < inMesh->mNumVertices; ++vertex) {
	//		for (uint8 i = 0; i < 4; ++i) {
	//			getElementPointer(index[i], 0xFFFFFFFF, vertex);
	//			getElementPointer(weight[i], 0.0f, vertex);
	//		}
	//	}
	//	
	//	for (uint32 b = 0; b < inMesh->mNumBones; ++b) {
	//		const auto& assimpBone = inMesh->mBones[b];
	//	
	//		for (uint32 w = 0; w < assimpBone->mNumWeights; ++w) {
	//			auto vertexIndex = assimpBone->mWeights[w].mVertexId;
	//			
	//			for (uint8 i = 0; i < 4; ++i) {
	//				if (getElementPointer(index[i], vertexIndex) == 0xFFFFFFFF) {
	//					getElementPointer(index[i], vertexIndex) = b;
	//					getElementPointer(weight[i], vertexIndex) = assimpBone->mWeights[w].mWeight;
	//					break;
	//				}
	//			}
	//		}
	//	}
	//}
	
	uint16 indexSize = 0;
	
	if(inMesh->mNumFaces * 3 < 0xFFFF)
	{
		//if (inMesh->mNumFaces * 3 < 0xFF) {
		//	indexSize = 1;
		//
		//	for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
		//		for (uint32 index = 0; index < 3; ++index) {
		//			uint8 idx = static_cast<uint8>(inMesh->mFaces[face].mIndices[index]);
		//			meshDataBuffer.CopyBytes(indexSize, reinterpret_cast<byte*>(&idx));
		//		}
		//	}
		//}
		//else {
			indexSize = 2;

			for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
				for (uint32 index = 0; index < 3; ++index) {
					meshDataBuffer.CopyBytes(indexSize, reinterpret_cast<byte*>(&inMesh->mFaces[face].mIndices[index]));
				}
			}
		//}
	}
	else
	{
		indexSize = 4;

		for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
			for (uint32 index = 0; index < 3; ++index) {
				meshDataBuffer.CopyBytes(indexSize, reinterpret_cast<byte*>(inMesh->mFaces[face].mIndices + index));
			}
		}
	}

	meshInfo.IndexCount = inMesh->mNumFaces * 3;
	meshInfo.IndexSize = indexSize;
}