#pragma once

#include <GTSL/Vector.hpp>
#include <GTSL/Buffer.hpp>

#include "ResourceManager.h"

#include <GTSL/Delegate.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/File.h>
#include <GTSL/MappedFile.hpp>
#include <GTSL/Math/Vectors.hpp>


#include "ByteEngine/Game/ApplicationManager.h"
#include "GAL/Pipelines.h"

namespace GAL {
	enum class ShaderDataType : unsigned char;
}

namespace GTSL {
	class Vector2;
}

struct EntryHeader {
	uint64 DataOffset = 0, DataSize = 0;
	GTSL::ShortString<96> Name;
};

struct SData {
	EntryHeader Header;

	GTSL::StringView GetName() const { return Header.Name; }
};

struct ResourceFiles {
	void Start(const GTSL::StringView string) {
		bool a = false, b = false, c = false;

		switch (table.Open(GTSL::StaticString<512>(string) + u8".betbl", GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: a = true; break;
		case GTSL::File::OpenResult::CREATED: a = true; break;
		case GTSL::File::OpenResult::ERROR: break;
		}

		switch (index.Open(GTSL::StaticString<512>(string) + u8".beidx", GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: b = true; break;
		case GTSL::File::OpenResult::CREATED: b = true; break;
		case GTSL::File::OpenResult::ERROR: break;
		}

		switch (data.Open(GTSL::StaticString<512>(string) + u8".bepkg", GTSL::File::READ | GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: c = true; break;
		case GTSL::File::OpenResult::CREATED: c = true; break;
		case GTSL::File::OpenResult::ERROR: break;
		}

		if(!(a && b && c) or !table.GetSize() or !index.GetSize() or !data.GetSize()) {
			table.Resize(0); index.Resize(0); data.Resize(0);

			table.Write(5, reinterpret_cast<const byte*>("BETBL"));
			index.Write(5, reinterpret_cast<const byte*>("BEIDX"));
			data.Write(5, reinterpret_cast<const byte*>("BEPKG"));
		} else {
			table.SetPointer(5); //skip name

			while (true) {
				GTSL::StaticBuffer<1024> buffer(1024, 16);
				auto read = table.Read(1024, buffer);
				if (!read) { break; }
				for (uint32 i = 0; i < buffer.GetLength() / sizeof(TableEntry); ++i) {
					TableEntry table_entry;
					buffer.Read(sizeof(TableEntry), reinterpret_cast<byte*>(&table_entry));

					tableMap.Emplace(table_entry.Name, table_entry.Offset);
				}
			}
		}
	}

	template<typename T>
	bool AddEntry(const GTSL::StringView name, T* indexDataPointer, GTSL::Range<const byte*> dataPointer) {
		auto hashedName = Hash(name);
		if (tableMap.Find(hashedName)) { return false; }
		TableEntry table_entry;
		table_entry.Name = hashedName;
		table_entry.Offset = index.GetSize();
		table.Write(sizeof(TableEntry), reinterpret_cast<const byte*>(&table_entry));
		tableMap.Emplace(hashedName, table_entry.Offset);

		indexDataPointer->Header.DataOffset = data.GetSize();
		indexDataPointer->Header.DataSize = dataPointer.Bytes();
		indexDataPointer->Header.Name = name;
		index.Write(GTSL::Math::RoundUpByPowerOf2(sizeof(T), BLOCK_SIZE), reinterpret_cast<const byte*>(indexDataPointer));

		data.Write(GTSL::Math::RoundUpByPowerOf2(dataPointer.Bytes(), BLOCK_SIZE), dataPointer.begin());

		return true;
	}

	template<class T>
	bool LoadEntry(const Id name, T& entry) {
		if (!tableMap.Find(static_cast<uint64>(name))) { return false; }

		auto indexDataOffset = tableMap.At(static_cast<uint64>(name));
		index.SetPointer(indexDataOffset);
		index.Read(sizeof(T), reinterpret_cast<byte*>(&entry));

		return true;
	}

	bool Exists(const Id name) {
		return tableMap.Find(static_cast<uint64>(name));
	}

	bool LoadData(auto& info, GTSL::Range<byte*> buffer) {
		data.SetPointer(info.Header.DataOffset);
		data.Read(info.Header.DataSize, buffer.begin());
		return true;
	}

private:
	static constexpr uint64 BLOCK_SIZE = 1024u;

	struct TableEntry {
		uint64 Name; uint64 Offset;
	};

	GTSL::File table, index, data;
	GTSL::HashMap<uint64, uint64, GTSL::DefaultAllocatorReference> tableMap;
};

template<typename A, size_t SIZE>
struct Array
{
	A& EmplaceBack(const A val = A()) {
		array[Length] = val;
		return array[Length++];
	}

	operator GTSL::Range<const A*>() const { return GTSL::Range<const A*>(Length, array); }

	uint32 Length = 0;
	A array[SIZE];
};

#define DEFINE_MEMBER(type, name) type name;\
type Get##name() const { return name; }\
type& Get##name() { return name; }
#define DEFINE_ARRAY_MEMBER(type, name, size) Array<type, size> name;\
Array<type, size>& Get##name() { return name; }\
const Array<type, size>& Get##name() const { return name; }

struct SubData {
};

class StaticMeshResourceManager final : public ResourceManager
{
public:
	StaticMeshResourceManager(const InitializeInfo&);
	~StaticMeshResourceManager();

	struct StaticMeshInfo : SData {
		struct SubMeshData : SubData {
			/**
			* \brief Number of vertices the loaded mesh contains.
			*/
			DEFINE_MEMBER(uint32, VertexCount)

			/**
			 * \brief Number of indeces the loaded mesh contains. Every face can only have three indeces.
			 */
			DEFINE_MEMBER(uint32, IndexCount)
			DEFINE_MEMBER(uint32, MaterialIndex)
			DEFINE_MEMBER(GTSL::Vector3, BoundingBox)
			DEFINE_MEMBER(float32, BoundingRadius)
		};;

		DEFINE_MEMBER(uint32, VertexCount)
		DEFINE_MEMBER(uint32, IndexCount)


		/**
		 * \brief Size of a single index to determine whether to use uint16 or uint32.
		 */
		DEFINE_MEMBER(uint8, IndexSize)
		DEFINE_MEMBER(GTSL::Vector3, BoundingBox)
		DEFINE_MEMBER(float32, BoundingRadius)
		DEFINE_ARRAY_MEMBER(GAL::ShaderDataType, VertexDescriptor, 16)
		DEFINE_ARRAY_MEMBER(SubMeshData, SubMeshes, 16)

		uint8 GetVertexSize() const { return GAL::GraphicsPipeline::GetVertexSize(GetVertexDescriptor()); }
	};

	template<typename... ARGS>
	void LoadStaticMeshInfo(ApplicationManager* gameInstance, Id meshName, DynamicTaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"StaticMeshResourceManager::loadStaticMeshInfo", {}, &StaticMeshResourceManager::loadStaticMeshInfo<ARGS...>, {}, {}, GTSL::MoveRef(meshName), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}

	template<typename... ARGS>
	void LoadStaticMesh(ApplicationManager* gameInstance, StaticMeshInfo staticMeshInfo, uint32 indicesAlignment, GTSL::Range<byte*> buffer, DynamicTaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS&&... args) {
		gameInstance->AddDynamicTask(this, u8"StaticMeshResourceManager::loadStaticMesh", {}, &StaticMeshResourceManager::loadMesh<ARGS...>, {}, {}, GTSL::MoveRef(staticMeshInfo), GTSL::MoveRef(indicesAlignment), GTSL::MoveRef(buffer), GTSL::MoveRef(dynamicTaskHandle), GTSL::ForwardRef<ARGS>(args)...);
	}
	
private:
	ResourceFiles resource_files_;

	template<typename... ARGS>
	void loadStaticMeshInfo(TaskInfo taskInfo, Id meshName, DynamicTaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS... args)
	{
		StaticMeshInfo static_mesh_info;
		resource_files_.LoadEntry(meshName, static_mesh_info);

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(static_mesh_info), GTSL::ForwardRef<ARGS>(args)...);
	};

	template<typename... ARGS>
	void loadMesh(TaskInfo taskInfo, StaticMeshInfo staticMeshInfo, uint32 indicesAlignment, GTSL::Range<byte*> buffer, DynamicTaskHandle<StaticMeshInfo, ARGS...> dynamicTaskHandle, ARGS... args)
	{
		auto verticesSize = staticMeshInfo.GetVertexSize() * staticMeshInfo.GetVertexCount(); auto indicesSize = staticMeshInfo.GetIndexSize() * staticMeshInfo.GetIndexCount();

		BE_ASSERT(buffer.Bytes() >= GTSL::Math::RoundUpByPowerOf2(verticesSize, indicesAlignment) + indicesSize, u8"")

		byte* vertices = buffer.begin();
		byte* indices = GTSL::AlignPointer(indicesAlignment, vertices + verticesSize);

		resource_files_.LoadData(staticMeshInfo, buffer); //TODO: CUSTOM LOGIC

		//GTSL::MemCopy(verticesSize, mappedFile.GetData() + staticMeshInfo.ByteOffset, vertices);
		//GTSL::MemCopy(indicesSize, mappedFile.GetData() + staticMeshInfo.ByteOffset + verticesSize, indices);

		taskInfo.ApplicationManager->AddStoredDynamicTask(dynamicTaskHandle, GTSL::MoveRef(staticMeshInfo), GTSL::ForwardRef<ARGS>(args)...);
	};

	bool loadMesh(const GTSL::Buffer<BE::TAR>& sourceBuffer, StaticMeshInfo& meshInfo, GTSL::Buffer<BE::TAR>& meshDataBuffer);
};
