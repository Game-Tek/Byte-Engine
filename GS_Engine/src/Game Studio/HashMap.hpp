#pragma once

#include "Core.h"

#include "SinglyLinkedList.hpp"

template <typename V, typename K = uint32>
GS_CLASS HashMapBucket
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
GS_CLASS HashMap
{
	SingleLinkList<HashMapBucket<V, K>>* m_BucketArray = nullptr;

	uint16 m_Length = 0;

	INLINE static HashMapBucket* Allocate(const uint16 _Count)
	{
		return new SingleLinkList<HashMapBucket<V, K>>[_Count];
	}

	INLINE static uint32 IndexFromKey(const uint32& _Key)
	{
		return _Key % m_Length;
	}

	//Returns an index to the bucket in the list that holds that value. If none is found -1 is returned.
	INLINE static int16 FindValueInList(const SingleLinkList<HashMapBucket<V, K>> & _List, const V & _Value)
	{
		for (uint8 i = 0; i < _List.GetLength(); i++)
		{
			if (_List[i].GetValue() == _Value)
			{
				return i;
			}
		}

		return -1;
	}

public:
	HashMap(const uint16 _BucketCount) : m_BucketArray(Allocate(_BucketCount)), m_Length(_BucketCount)
	{
	}

	void Insert(const V& _Value, const K& _Key)
	{
		//Get the last bucket int the chain, create a new bucket and set the pointer of the previous to that of the recently created.
		m_BucketArray[IndexFromKey(_Key)].PushBack(HashMapBucket<V, K>(_Value, _Key));
	}
	
	void Remove(const V & _Value, const K& _Key)
	{
		m_BucketArray[IndexFromKey(_Key)].Remove(m_BucketArray[IndexFromKey(_Key)].Find(HashMapBucket(_Value, _Key)));
	}

	//Looks for a value inside the map. Returns true if it was found and false if it wasn't.
	bool Find(const V& _Value, const K& _Key)
	{
		auto Result = m_BucketArray[IndexFromKey(_Key)].Find(HashMapBucket(_Value, _Key));

		if (Result == -1)
		{
			return false;
		}
		else
		{
			return true;
		}
	}
};