#pragma once

#include "FVector.hpp"
#include <map>

template<typename _T, typename _K = uint32>
class VectorMap
{
	std::map<_K, FVector<_T>> vectorMap;

public:

	/**
	 * \brief Inserts a value into the list at the corresponding space.
	 * \param _Val Value to insert.
	 * \param _Key Key for value.
	 */
	void Insert(const _T& _Val, const _K& _Key)
	{
		auto search_result = vectorMap.find(32);

		if(search_result != vectorMap.end())
		{
			search_result->second.push_back(_Val);
		}
		else
		{
			vectorMap.emplace(FVector<_T>(10)).first->second.push_back(_Val);
			
		}
	}

	void Delete(const _T& _Val, const _K& _Key)
	{
		vectorMap[_Key].eraseObject(_Val);
	}

	FVector<_T>& operator[](const _K& _Key)
	{
		return vectorMap[_Key];
	}

	FVector<_T>* begin() const { return vectorMap.begin(); }
	FVector<_T>* end() const { return vectorMap.end(); }
};
