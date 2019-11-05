#pragma once

#include "FVector.hpp"
#include <map>

template<typename _T, typename _P, typename _K = uint32>
class VectorMap
{
	FVector<Pair<_P, FVector<_T>>> vectorMap;
	
public:

	VectorMap() : vectorMap(10)
	{
	}
	
	/**
	 * \brief Inserts a value into the list at the corresponding space.
	 * \param _Pair Value to insert.
	 * \param _Key Key for value.
	 */
	void Insert(const _P& _Identifier, const _T& _Value, const _K& _Key)
	{
		auto search_result = vectorMap.find(_Value);

		if(search_result.First)
		{
			//search_result->second.Second.push_back(_Value);
			vectorMap[search_result->Second].Second.push_back(_Value);
		}
		else
		{
			//vectorMap.emplace(Pair<_P, FVector<_T>>(_Identifier, FVector<_T>(10))).first->second.Second.push_back(_Value);
			vectorMap.push_back(Pair<_P, FVector<_T>>(_Identifier, FVector<_T>(10)));
			vectorMap[vectorMap.getLength()].Second.push_back(_Value);
		}
	}

	void Delete(const _T& _Val, const _K& _Key)
	{
		vectorMap[vectorMap.find(_Key).Second].eraseObject(_Val);
	}

	FVector<_T>& operator[](const _K& _Key)
	{
		return vectorMap[_Key];
	}

	[[nodiscard]] Pair<_P, FVector<_T>>* begin() { return vectorMap.begin(); }
	[[nodiscard]] Pair<_P, FVector<_T>>* end() { return vectorMap.end(); }
};
