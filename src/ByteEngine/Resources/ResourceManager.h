#pragma once

#include "ByteEngine/Game/System.hpp"
#include "ByteEngine/Game/Tasks.h"

#include <GTSL/Vector.hpp>

#include "GTSL/HashMap.hpp"

/**
 * \brief Used to specify a type of resource loader.
 *
 * This class will be instanced sometime during the application's lifetime to allow loading of some type of resource made possible by extension of this class.
 *
 * Every extension will allow for loading of 1 type of resource.
 */
class ResourceManager : public BE::System {
public:
	ResourceManager() = default;

	ResourceManager(const InitializeInfo& info, const utf8* name) : System(info, name) {}

	static GTSL::StaticString<512> GetUserResourcePath(const GTSL::Range<const utf8*> fileWithExtension);
	static GTSL::StaticString<512> GetUserResourcePath(const GTSL::Range<const utf8*> fileName, const GTSL::Range<const utf8*> extension);
	GTSL::StaticString<512> GetResourcePath(const GTSL::Range<const utf8*> fileName, const GTSL::Range<const utf8*> extension);
	GTSL::StaticString<512> GetResourcePath(const GTSL::Range<const utf8*> fileWithExtension);

	//DATA
	//DATA SERIALIZE : DATA
	//INFO : DATA SERIALIZE

	struct Data{};

	template<class I>
	struct DataSerialize : public I
	{
		/**
		 * \brief Byte offset to the start of the resource binary data into the package file.
		 */
		uint32 ByteOffset = 0;

		template<class ALLOCATOR>
		friend void Insert(const DataSerialize& textureInfoSerialize, GTSL::Buffer<ALLOCATOR>& buffer)
		{
			Insert(textureInfoSerialize.ByteOffset, buffer);
		}

		template<class ALLOCATOR>
		friend void Extract(DataSerialize& textureInfoSerialize, GTSL::Buffer<ALLOCATOR>& buffer)
		{
			Extract(textureInfoSerialize.ByteOffset, buffer);
		}
	};

	template<typename I>
	struct Info : public I
	{
		Info() = default;
		Info(const Id name, const I& i) : I(i), Name(name) {}
		Info(const I& i) : I(i) {}
		
		/**
		 * \brief Name of the resource.
		 */
		Id Name;
	};

#define DECL_INFO_CONSTRUCTOR(className, inheritsFrom) className() = default; className(const Id name, const inheritsFrom& i) : inheritsFrom(name, i) {}

	#define INSERT_START(className)\
	template<class ALLOCATOR>\
	friend void Insert(const className& insertInfo, GTSL::Buffer<ALLOCATOR>& buffer)

	#define INSERT_BODY Insert(insertInfo.ByteOffset, buffer);

	#define EXTRACT_START(className)\
	template<class ALLOCATOR>\
	friend void Extract(className& extractInfo, GTSL::Buffer<ALLOCATOR>& buffer)

	#define EXTRACT_BODY Extract(extractInfo.ByteOffset, buffer);

	static constexpr uint8 MAX_THREADS = 32;

protected:
	void initializePackageFiles(GTSL::StaticVector<GTSL::File, MAX_THREADS>& filesPerThread, GTSL::Range<const utf8*> path);
};

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

		if (!(a && b && c) or !table.GetSize() or !index.GetSize() or !data.GetSize()) {
			table.Resize(0); index.Resize(0); data.Resize(0);

			table.Write(5, reinterpret_cast<const byte*>("BETBL"));
			index.Write(5, reinterpret_cast<const byte*>("BEIDX"));
			data.Write(5, reinterpret_cast<const byte*>("BEPKG"));
		}
		else {
			table.SetPointer(5); //skip name

			while (true) {
				GTSL::StaticBuffer<1024> buffer(1024, 16);
				auto read = table.Read(buffer, 1024);
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
		auto hashedName = GTSL::Hash(name);
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
	bool LoadEntry(const GTSL::StringView name, T& entry) {
		if (!tableMap.Find(static_cast<uint64>(Id(name)))) { return false; }

		auto indexDataOffset = tableMap.At(static_cast<uint64>(Id(name)));
		index.SetPointer(indexDataOffset);
		index.Read(sizeof(T), &entry);

		return true;
	}

	bool Exists(const Id name) {
		return tableMap.Find(static_cast<uint64>(name));
	}

	bool LoadData(auto& info, auto& buffer) {
		data.SetPointer(info.Header.DataOffset);
		data.Read(buffer, info.Header.DataSize);
		return true;
	}
	
	bool LoadData(auto& info, GTSL::Range<byte*> buffer) {
		data.SetPointer(info.Header.DataOffset);
		data.Read(info.Header.DataSize, buffer.begin());
		return true;
	}

	//bool LoadData(auto& info, GTSL::Range<byte*> buffer) {
	//	data.SetPointer(info.Header.DataOffset);
	//	data.Read(info.Header.DataSize, buffer.begin());
	//	return true;
	//}

	bool LoadData(auto& info, GTSL::Range<byte*> buffer, uint32 offset, uint32 size) {
		data.SetPointer(info.Header.DataOffset + offset);
		data.Read(size, buffer.begin());
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

	A& front() { return array[0]; }
	A& back() { return array[Length - 1]; }
	void PopBack() { --Length; }

	operator GTSL::Range<const A*>() const { return GTSL::Range<const A*>(Length, array); }

	operator bool() const { return Length; }

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