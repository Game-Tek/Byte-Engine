#pragma once
#include "FVector.hpp"

template<typename T>
class KVector
{
public:
	using length_type = typename FVector<T>::length_type;

private:
	FVector<T> objects;
	FVector<uint32> freeIndexes;

	bool findFreeIndex(length_type& index)
	{
		if (freeIndexes.getLength() == 0)
		{
			return false;
		}
		const auto free_index = freeIndexes[0];
		freeIndexes.pop(0);
		index = free_index;
		return true;
	}
public:
	
	auto begin() { return objects.begin(); }
	auto end() { return objects.end(); }

	length_type Place(const T& obj)
	{
		length_type index = 0;

		if(findFreeIndex(index))
		{
			objects.place(index, objects);
			return index;
		}

		return objects.push_back(obj);
	}

	template <typename... Args>
	length_type Emplace(Args&&... args)
	{
		length_type index = 0;

		if(findFreeIndex(index))
		{
			objects.emplace(index, std::forward<Args>(args) ...);
			return index;
		}

		return objects.emplace_back(std::forward<Args>(args) ...);
	}
	
	void Destroy(const length_type index)
	{
		freeIndexes.push_back(index);

		if(index == objects.getLength())
		{
			objects.pop_back();
		}
		
		objects.destroy(index);
	}
};
