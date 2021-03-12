#pragma once

#include "ByteEngine/Object.h"

#include <GTSL/Id.h>
#include <GTSL/DynamicType.h>
#include <GTSL/Array.hpp>

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
	
	GTSL::StaticString<512> GetResourcePath(const GTSL::Range<const utf8*> fileName);
	
	struct ResourceLoadInfo
	{
		/**
		 * \brief Name of the resource to load. Must be unique and match the name used in the editor. Is case sensitive.
		 */
		Id Name;
		
		/**
		 * \brief Pointer to some data to potentially be retrieved on resource load for the client to identify the resource. Can be NULL.
		 */
		SAFE_POINTER UserData;

		/**
		 * \brief Buffer to write the loaded data to.
		 */
		GTSL::Range<byte*> DataBuffer;
		
		/**
		 * \brief Instance of game instance to call to dispatch task when resource is loaded.
		 */
		class GameInstance* GameInstance = nullptr;
		
		GTSL::Array<TaskDependency, 64> ActsOn;
	};

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
	
	struct OnResourceLoad
	{
		OnResourceLoad& operator=(const ResourceLoadInfo& resourceLoadInfo)
		{
			UserData = resourceLoadInfo.UserData;
			DataBuffer = resourceLoadInfo.DataBuffer;
			ResourceName = resourceLoadInfo.Name;
		}
		
		/**
		 * \brief Pointer to the user provided data on the resource request.
		 */
		SAFE_POINTER UserData;
		
		/**
		 * \brief Buffer where the loaded data was written to.
		 */
		GTSL::Range<byte*> DataBuffer;

		Id ResourceName;
	};

	static constexpr uint8 MAX_THREADS = 32;


protected:
	GTSL::File& getFile() { return packageFiles[getThread()]; }
	void initializePackageFiles(GTSL::Range<const utf8*> path);
	
	GTSL::Array<GTSL::File, MAX_THREADS> packageFiles;
};