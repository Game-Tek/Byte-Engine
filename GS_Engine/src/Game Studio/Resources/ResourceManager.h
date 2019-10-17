#pragma once

#include "Core.h"

#include "Object.h"

#include "Resource.h"

#include <unordered_map>
#include "Containers/FString.h"
#include "Containers/Id.h"

#include "Debug/Logger.h"

class ResourceManager : public Object
{
public:
	class ResourcePush
	{
		FVector<SaveResourceElementDescriptor> FileElements;

	public:
		ResourcePush() : FileElements(5)
		{
		}

		INLINE ResourcePush& operator+=(const SaveResourceElementDescriptor& _FED)
		{
			FileElements.push_back(_FED);
			return *this;
		}

		const SaveResourceElementDescriptor& operator[](uint64 _I) const { return FileElements[_I]; }

		[[nodiscard]] uint64 GetElementCount() const { return FileElements.length(); }
	};

private:
	//mutable std::unordered_map<Resource*, Resource*> ResourceMap;
	mutable FVector<Resource*> R;

	static FString GetBaseResourcePath() { return FString("resources/"); }
	void SaveFile(const FString& _ResourceName, void (*f)(std::ostream& _OS));

	void GetResourceInternal(const FString& _ResourceName, Resource* _Resource) const;

public:
	ResourceManager() : R(50)
	{
	}

	template<class T>
	T* GetResource(const FString& _ResourceName) const
	{
		//auto HashedName = Id(_Name);
		//
		//auto Loc = ResourceMap.find(HashedName.GetID());
		//
		//if(Loc != ResourceMap.cend())
		//{
		//	return StaticMeshResourceHandle(&ResourceMap[HashedName.GetID()]);
		//}
		
		//auto Path = GetBaseResourcePath() + "static meshes/" + _Name + ".obj";

		Resource* resource = new T();

		GetResourceInternal(_ResourceName, resource);

		//return nullptr;
		return SCAST(T*, resource);
	}

	template<class T>
	void CreateResource(const FString& _Path, void (*f)(std::ostream& _OS))
	{
		Resource* resource = new T();
		FString path = _Path + resource->GetResourceTypeExtension();
		SaveFile(path, f);
		GetResourceInternal(_Path, resource);
	}

	void ReleaseResource(Resource* _Resource) const;

	[[nodiscard]] const char* GetName() const override { return "Resource Manager"; }
};
