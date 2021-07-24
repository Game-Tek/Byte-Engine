#pragma once

#include "ByteEngine/Object.h"

#include <GTSL/Vector.hpp>

#include "ByteEngine/Game/Tasks.h"

/**
 * \brief Used to specify a type of resource loader.
 *
 * This class will be instanced sometime during the application's lifetime to allow loading of some type of resource made possible by extension of this class.
 *
 * Every extension will allow for loading of 1 type of resource.
 */
class ResourceManager : public Object
{
public:
	ResourceManager() = default;

	ResourceManager(const utf8* name) : Object(name) {}
	
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