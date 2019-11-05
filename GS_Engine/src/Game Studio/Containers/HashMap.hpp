#pragma once

#include "Core.h"

#include "SinglyLinkedList.hpp"
#include "FVector.hpp"


template <typename _Val, typename _Ky = uint32>
class HashMap
{
	using hash_type = uint32;
	
	class HashMapBucket
	{
		_Val value;
		_Ky key;

	public:
		HashMapBucket(_Val _Value, _Ky _Key) : value(value), key(_Key)
		{
		}

		~HashMapBucket()
		{
		}

		_Val& GetValue() const { return value; }
		const _Ky& GetKey() const { return key; }
	};
	
	FVector<SingleLinkList<HashMapBucket>> List;

	/**
	 * \brief Stores the length of the hash map. Must always be a power of 2.
	 */
	uint32 mapLength = 0;
	
	/**
	 * \brief Makes an index into the map from the hash of a key.
	 * \param _Hash The result of the hash for the key type.
	 * \return Returns an index into the map.
	 */
	INLINE uint32 indexFromHash(const hash_type _Hash) const { return _Hash & mapLength; }

	//Returns an index to the bucket in the list that holds that value. If none is found -1 is returned.
	INLINE int16 FindValueInList(const _Val & _Value, const _Ky & _Key)
	{
		for (uint8 i = 0; i < List[IndexFromHash(_Key)].getLength(); i++)
		{
			if (List[IndexFromHash(_Key)][i].GetElement() == _Value)
			{
				return i;
			}
		}

		return -1;
	}

public:
	explicit HashMap(const uint16 _BucketCount) : List(_BucketCount), m_Length(_BucketCount)
	{
	}

	void Insert(const _Val& _Value, const _Ky& _Key)
	{
		//Get the last bucket int the chain, create a new bucket and set the pointer of the previous to that of the recently created.
		List[IndexFromHash(_Key)].PushBack(HashMapBucket<_Val, _Key>(_Value, _Key));
	}
	
	void Remove(const _Val & _Value, const _Ky& _Key)
	{
		List[IndexFromHash(_Key)].Remove(List[IndexFromHash(_Key)].Find(HashMapBucket(_Value, _Key)));
	}

	//Looks for a value inside the map. Returns true if it was found and false if it wasn't.
	bool Find(const _Val& _Value, const _Ky& _Key)
	{
		auto Result = List[IndexFromHash(_Key)].Find(HashMapBucket(_Value, _Key));

		if (Result == -1)
		{
			return false;
		}
		else
		{
			return true;
		}
	}

	_Val& Get(const _Ky& _Key)
	{
		return List[IndexFromHash(_Key)].Get();
	}
};