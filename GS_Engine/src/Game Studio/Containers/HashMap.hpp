#pragma once

#include "Core.h"

#include "SinglyLinkedList.hpp"
#include "FVector.hpp"

template <typename V, typename K = uint32>
class GS_API HashMapBucket
{
	V Value;
	K Key;

public:
	HashMapBucket(V _Value, K _Key) : Value(Value), Key(_Key)
	{
	}

	~HashMapBucket()
	{
	}

	V& GetValue() const { return Value; }
	K& GetKey() const { return Key; }
};

template <typename V, typename K = uint32>
class GS_API HashMap
{
	FVector<SingleLinkList<HashMapBucket<V, K>>> List;

	uint16 m_Length = 0;

	INLINE static SingleLinkList<HashMapBucket<V, K>> * Allocate(const uint16 _Count)
	{
		return new SingleLinkList<HashMapBucket<V, K>>[_Count];
	}

	INLINE uint32 IndexFromKey(const uint32& _Key)
	{
		return _Key % m_Length;
	}

	//Returns an index to the bucket in the list that holds that value. If none is found -1 is returned.
	INLINE int16 FindValueInList(const V & _Value, const K & _Key)
	{
		for (uint8 i = 0; i < List[IndexFromKey(_Key)].getLength(); i++)
		{
			if (List[IndexFromKey(_Key)][i].GetElement() == _Value)
			{
				return i;
			}
		}

		return -1;
	}

public:
	HashMap(const uint16 _BucketCount) : List(_BucketCount), m_Length(_BucketCount)
	{
	}

	void Insert(const V& _Value, const K& _Key)
	{
		//Get the last bucket int the chain, create a new bucket and set the pointer of the previous to that of the recently created.
		List[IndexFromKey(_Key)].PushBack(HashMapBucket<V, K>(_Value, _Key));
	}
	
	void Remove(const V & _Value, const K& _Key)
	{
		List[IndexFromKey(_Key)].Remove(List[IndexFromKey(_Key)].Find(HashMapBucket(_Value, _Key)));
	}

	//Looks for a value inside the map. Returns true if it was found and false if it wasn't.
	bool Find(const V& _Value, const K& _Key)
	{
		auto Result = List[IndexFromKey(_Key)].Find(HashMapBucket(_Value, _Key));

		if (Result == -1)
		{
			return false;
		}
		else
		{
			return true;
		}
	}

	V& Get(const K& _Key)
	{
		return List[IndexFromKey(_Key)].Get();
	}
};