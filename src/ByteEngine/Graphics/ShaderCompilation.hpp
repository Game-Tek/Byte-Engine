#pragma once

#include "ByteEngine/Graph.hpp"

enum class State : GTSL::uint8
{
	NONE, ADDED, MODIFIED, DELETED
};

struct FileChangeNotification
{
	State state;
	GTSL::uint64 FileNameHash = 0ull, FileHash = 0ull;
	GTSL::String<BE::TAR> Name;
	GTSL::uint64 Pointer = 0ull, ParentFileNameHash = 0ull;
};

auto GetChangedFiles(const auto& allocator, const GTSL::File& file, const GTSL::Range<const GTSL::StringView*> paths)
{
	GTSL::Buffer cacheBuffer(128 * 1024, 16, allocator);

	file.Read(cacheBuffer);

	GTSL::uint32 cacheEntryCount = cacheBuffer.GetLength() / 512;
	GTSL::uint8* buffer = cacheBuffer.begin();

	GTSL::HashMap<GTSL::uint64, GTSL::Tuple<GTSL::uint64, bool, GTSL::uint64, GTSL::uint64>, BE::TAR> entriesMap(64, allocator);

	GTSL::Vector<FileChangeNotification, BE::TAR> files(64, allocator);

	for (GTSL::uint32 i = 0; i < cacheEntryCount; ++i) {
		GTSL::uint8* entry = buffer + 512 * i;

		auto currentNodeHash = *reinterpret_cast<const GTSL::uint64*>(entry);
		auto currentNodeFileHash = *(reinterpret_cast<const GTSL::uint64*>(entry) + 1);
		auto parentHash = *(reinterpret_cast<const GTSL::uint64*>(entry) + 2);

		GTSL::StaticString<256> fileName(GTSL::StringView{ reinterpret_cast<const char8_t*>(entry + 24) });

		entriesMap.Emplace(currentNodeHash, GTSL::MoveRef(currentNodeFileHash), false, GTSL::MoveRef(parentHash), i * 512ull);
	}

	for (auto path : paths) {
		GTSL::FileQuery fileQuery(path);

		while (auto fileRef = fileQuery()) {
			GTSL::uint64 fileNameHash = GTSL::Hash(fileRef.Get());

			if (auto res = entriesMap.TryGet(fileNameHash)) { // If file by that name exists
				GTSL::Get<1>(res.Get()) = true; // File was seen in this visit

				const GTSL::uint64 trackedHash = GTSL::Get<0>(res.Get());

				if (trackedHash != fileQuery.GetFileHash()) { // If file has new hash, then it is has been modified
					files.EmplaceBack(State::MODIFIED, fileNameHash, fileQuery.GetFileHash(), GTSL::String{ fileRef.Get(), allocator }, GTSL::Get<3>(res.Get()), GTSL::Get<2>(res.Get()));
				}
			}
			else { // File was not being tracked, then is new				
				files.EmplaceBack(State::ADDED, fileNameHash, fileQuery.GetFileHash(), GTSL::String{ fileRef.Get(), allocator });

				entriesMap.Emplace(fileNameHash, fileQuery.GetFileHash(), true, 0ull, 0ull);
			}
		}
	}

	if (true) { // If we visited less items than those that we were tracking then something was deleted
		for (auto& e : entriesMap) {
			if (!GTSL::Get<1>(e)) { // If file was not seen in this iteration then it must have been deleted (or renamed which we can't easily identify)
				files.EmplaceBack(State::DELETED, 0ull, GTSL::Get<0>(e), GTSL::String{ u8"", allocator }, GTSL::Get<3>(e), GTSL::Get<2>(e));
			}
		}
	}

	return files;
}

auto GetTree(const auto& allocator, GTSL::File& file) {
	file.SetPointer(0);

	GTSL::Buffer cacheBuffer(128 * 1024, 16, allocator);

	file.Read(cacheBuffer);

	GTSL::uint32 cacheEntryCount = cacheBuffer.GetLength() / 512;
	GTSL::uint8* buffer = cacheBuffer.begin();

	GTSL::HashMap<GTSL::uint64, Graph<FileChangeNotification>, BE::TAR> tree(64, allocator);

	GTSL::Vector<GTSL::Pair<GTSL::uint64, GTSL::uint64>, BE::TAR> pendingInserts(128, allocator);

	for (GTSL::uint32 i = 0; i < cacheEntryCount; ++i) {
		GTSL::uint8* entry = buffer + 512 * i;

		auto currentNodeHash = *reinterpret_cast<const GTSL::uint64*>(entry);
		auto currentNodeFileHash = *(reinterpret_cast<const GTSL::uint64*>(entry) + 1);
		auto parentHash = *(reinterpret_cast<const GTSL::uint64*>(entry) + 2);

		auto& currentNode = tree.Emplace(currentNodeHash, FileChangeNotification{ State::NONE, currentNodeHash, currentNodeFileHash, { GTSL::StringView{ reinterpret_cast<const char8_t*>(entry + 24) }, allocator }, i * 512, parentHash });

		if (!parentHash) { continue; }

		pendingInserts.EmplaceBack(currentNodeHash, parentHash);
	}

	while (pendingInserts) {
		auto& last = pendingInserts.back();
		if (tree.Find(last.Second)) { // Node could not exist if it was not inserted by client
			tree[last.Second].Connect(tree[last.First]); // Connect parent to children
		}
		pendingInserts.PopBack();
	}

	return tree;
}

inline GTSL::uint64 CommitFileChangeToCache(GTSL::File& file, GTSL::StringView file_name, GTSL::uint64 fileHash, GTSL::uint64 parent_file_name_hash) {
	GTSL::uint64 pointer = file.GetSize();
	file << GTSL::Hash(file_name).value; // Add file name hash
	file << fileHash; // Add file hash
	file << parent_file_name_hash;
	file.Write(file_name.GetBytes(), reinterpret_cast<const GTSL::uint8*>(file_name.GetData()));
	GTSL::uint8 pad = 0;
	for (GTSL::uint32 i = 0; i < (512 - 8 - 8 - 8) - file_name.GetBytes(); ++i) { file << pad; }
	return pointer;
}

inline void UpdateFileHashCache(GTSL::uint64 po, GTSL::File& file, GTSL::uint64 file_hash) {
	file.SetPointer(po + 8);
	file << file_hash;
}

inline void UpdateParentFileNameCache(GTSL::uint64 po, GTSL::File& file, GTSL::uint64 parent_file_name_hash)
{
	file.SetPointer(po + 8 * 2);
	file << parent_file_name_hash;
}

template<class A>
auto operator<<(A& buffer, const GTSL::StringView string_view) -> A& 
{
	buffer << string_view.GetBytes() << string_view.GetCodepoints();
	buffer.Write(string_view.GetBytes(), reinterpret_cast<const GTSL::uint8*>(string_view.GetData()));
	return buffer;
}

template<class A>
auto operator>>(auto& buffer, GTSL::String<A>& vector) -> decltype(buffer)& 
{
	GTSL::uint32 length, codepoints;
	buffer >> length >> codepoints;
	for (GTSL::uint32 i = 0; i < length; ++i) {
		char8_t c;
		buffer >> c;
		vector += c;
	}
	return buffer;
}

inline GTSL::uint64 WriteIndexEntry(GTSL::File& file, GTSL::uint64 pointer, GTSL::uint64 data_pointer, GTSL::StringView string_view)
{
	GTSL::uint64 p = 0;
	if (pointer != ~0ULL) { file.SetPointer(pointer); p = pointer; }
	else { p = file.GetSize(); }
	file << data_pointer;
	file << string_view;
	GTSL::uint8 pad = 0;
	for (GTSL::uint32 i = 0; i < (128 - 8 - 4 - 4 - string_view.GetBytes()); ++i) { file << pad; }
	BE_ASSERT(p % 128 == 0, u8"uh oh");
	return p;
}

inline GTSL::uint64 ReadIndexEntry(GTSL::File& file, GTSL::uint64 pointer, auto&& f)
{
	file.SetPointer(pointer);

	GTSL::StaticBuffer<256> buffer;
	auto readBytes = file.Read(buffer, 128);

	GTSL::uint64 offset = 0; // Points to the data
	buffer >> offset;

	GTSL::StaticString<120> string;
	buffer >> string;

	f(offset, GTSL::StringView(string));

	return pointer + readBytes;
}

inline void UpdateIndexEntry(GTSL::File& file, GTSL::uint64 pointer, GTSL::uint64 new_pointer)
{
	file.SetPointer(pointer);

	file << new_pointer;
}

inline GAL::ShaderType ShaderTypeFromString(GTSL::StringView string)
{
	// TODO: Check
	switch (GTSL::Hash(string)) {
	case u8"VERTEX": return GAL::ShaderType::VERTEX;
	case u8"FRAGMENT": return GAL::ShaderType::FRAGMENT;
	case u8"COMPUTE": return GAL::ShaderType::COMPUTE;
	case u8"RAY_GEN": return GAL::ShaderType::RAY_GEN;
	case u8"CLOSEST_HIT": return GAL::ShaderType::CLOSEST_HIT;
	case u8"ANY_HIT": return GAL::ShaderType::ANY_HIT;
	case u8"MISS": return GAL::ShaderType::MISS;
	}
}

#include "ByteEngine/Render/ShaderGenerator.h"

inline Class ShaderClassFromString(GTSL::StringView string)
{
	switch (GTSL::Hash(string)) {
	case GTSL::Hash(u8"VERTEX"): return Class::VERTEX;
	case GTSL::Hash(u8"SURFACE"): return Class::SURFACE;
	case GTSL::Hash(u8"COMPUTE"): return Class::COMPUTE;
	case GTSL::Hash(u8"RAY_GEN"): return Class::RAY_GEN;
	case GTSL::Hash(u8"CLOSEST_HIT"): return Class::CLOSEST_HIT;
	case GTSL::Hash(u8"MISS"): return Class::MISS;
	}
}