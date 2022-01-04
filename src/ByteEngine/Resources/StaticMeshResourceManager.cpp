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
#include <GTSL/Math/Vectors.hpp>

static GTSL::Vector4 ToGTSL(const aiColor4D assimpVector) {
	return GTSL::Vector4(assimpVector.r, assimpVector.g, assimpVector.b, assimpVector.a);
}

static GTSL::Vector3 ToGTSL(const aiVector3D assimpVector) {
	return GTSL::Vector3(assimpVector.x, assimpVector.y, assimpVector.z);
}

static GTSL::Vector2 ToGTSL(const aiVector2D assimpVector) {
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

template<typename T, uint8 N>
struct nEl {
	T e[N];
};

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

	if (!(inMesh->mPrimitiveTypes & aiPrimitiveType_TRIANGLE)) { BE_ASSERT(false, ""); return; }

	meshInfo.VertexCount = inMesh->mNumVertices;

	//MESH ALWAYS HAS POSITIONS
	meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);

	if (inMesh->HasNormals()) {
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
	}

	if (inMesh->HasTangentsAndBitangents()) {
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT3);
	}

	for (uint8 tex_coords = 0; tex_coords < static_cast<uint8>(inMesh->GetNumUVChannels()); ++tex_coords) {
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT2);
	}

	for (uint8 colors = 0; colors < static_cast<uint8>(inMesh->GetNumColorChannels()); ++colors) {
		meshInfo.VertexDescriptor.EmplaceBack(GAL::ShaderDataType::FLOAT4);
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

	auto doWrite = [writeElement]<typename T>(const GAL::ShaderDataType format, T value, byte* byteData, uint32 vertexIndex) {
		switch (format) {
		case GAL::ShaderDataType::FLOAT3: {
			if constexpr (std::is_same_v<GTSL::Vector3, T>) {
				auto* positions = reinterpret_cast<GTSL::Vector3*>(byteData);
				writeElement(positions, value, vertexIndex);
			}
			break;
		}
		case GAL::ShaderDataType::UINT16: {
			if constexpr (std::is_same_v<GTSL::Vector3, T>) {
				auto* positions = reinterpret_cast<nEl<int16, 3>*>(byteData);
				writeElement(positions, { GAL::FloatToSNORM(value[0]), GAL::FloatToSNORM(value[1]), GAL::FloatToSNORM(value[2]) }, vertexIndex);
			}

			break;
		}
		case GAL::ShaderDataType::UINT32: {
			if constexpr (std::is_same_v<GTSL::Vector3, T>) {
				auto* positions = reinterpret_cast<nEl<uint16, 2>*>(byteData);
				auto textureCoordinate = GTSL::Vector2(value);
				writeElement(positions, { GAL::FloatToUNORM(textureCoordinate[0]), GAL::FloatToUNORM(textureCoordinate[1]) }, vertexIndex);
			}
			break;
		}
		case GAL::ShaderDataType::UINT64: {
			if constexpr (std::is_same_v<GTSL::Vector3, T>) {
				auto* positions = reinterpret_cast<nEl<int8, 3>*>(byteData);
				writeElement(positions, { static_cast<int8>(value[0] * static_cast<int8>(127)), static_cast<int8>(value[1] * static_cast<int8>(127)), static_cast<int8>(value[2] * static_cast<int8>(127)) }, vertexIndex);
			}
			break;
		}
		case GAL::ShaderDataType::FLOAT4: {
			if constexpr (std::is_same_v<GTSL::Vector4, T>) {
				auto* positions = reinterpret_cast<GTSL::Vector4*>(byteData);
				writeElement(positions, value, vertexIndex);
			}
			break;
		}
		}
	};

	auto writeAndReadVertexComponent = [&]<typename T>(GAL::ShaderDataType format, T* assimpDataArray, auto&& perVertexFunc) {
		auto* byteData = advanceVertexElement();

		for (uint64 vertexIndex = 0; vertexIndex < inMesh->mNumVertices; ++vertexIndex) {
			auto assimpAttribute = ToGTSL(assimpDataArray[vertexIndex]);
			perVertexFunc(assimpAttribute);
			doWrite(format, assimpAttribute, byteData, vertexIndex);
		}
	};

	auto writeVertexComponent = [&]<typename T>(GAL::ShaderDataType format, T* assimpDataArray) -> void {
		auto* byteData = advanceVertexElement();

		for (uint64 vertexIndex = 0; vertexIndex < inMesh->mNumVertices; ++vertexIndex) {
			doWrite(format, ToGTSL(assimpDataArray[vertexIndex]), byteData, vertexIndex);
		}
	};

	writeAndReadVertexComponent(GAL::ShaderDataType::FLOAT3, inMesh->mVertices, [&](GTSL::Vector3 position) {
		meshInfo.BoundingBox = GTSL::Math::Max(meshInfo.BoundingBox, GTSL::Math::Abs(position));
		meshInfo.BoundingRadius = GTSL::Math::Max(meshInfo.BoundingRadius, GTSL::Math::Length(position));
	});

	if (inMesh->HasNormals()) {
		writeVertexComponent(GAL::ShaderDataType::FLOAT3, inMesh->mNormals);
	}

	if (inMesh->HasTangentsAndBitangents()) {
		writeVertexComponent(GAL::ShaderDataType::FLOAT3, inMesh->mTangents);
		writeVertexComponent(GAL::ShaderDataType::FLOAT3, inMesh->mBitangents);
	}

	for(uint8 i = 0; i < 8 && inMesh->HasTextureCoords(i); ++i) {
		writeVertexComponent(GAL::ShaderDataType::FLOAT2, inMesh->mTextureCoords[i]);
	}

	for(uint8 i = 0; i < 8 && inMesh->HasVertexColors(i); ++i) {
		writeVertexComponent(GAL::ShaderDataType::FLOAT4, inMesh->mColors[i]);
	}
	
	uint16 indexSize = 0;
	
	if(inMesh->mNumFaces * 3 < 0xFFFF) {
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
					meshDataBuffer.Write(indexSize, reinterpret_cast<byte*>(&inMesh->mFaces[face].mIndices[index]));
				}
			}
		//}
	} else {
		indexSize = 4;

		for (uint32 face = 0; face < inMesh->mNumFaces; ++face) {
			for (uint32 index = 0; index < 3; ++index) {
				meshDataBuffer.Write(indexSize, reinterpret_cast<byte*>(inMesh->mFaces[face].mIndices + index));
			}
		}
	}

	meshInfo.IndexCount = inMesh->mNumFaces * 3;
	meshInfo.IndexSize = indexSize;
}