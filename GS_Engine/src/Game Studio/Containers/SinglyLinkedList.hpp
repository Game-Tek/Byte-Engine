#pragma once

#include "Core.h"

template<class T>
GS_STRUCT SingleLinkListNode
{
	SingleLinkListNode() = default;

	SingleLinkListNode(const T& _Obj) : Element(_Obj)
	{
	}

	SingleLinkListNode * GetChild() { return Child; }
	T & GetElement() { return Element; }

protected:
	SingleLinkListNode<T> * Child = nullptr;

	T Element;
};

template <class T>
GS_CLASS SingleLinkList
{
public:
	SingleLinkList() = default;

	//Preallocate
	explicit SingleLinkList(const size_t _Length) : m_Length(_Length)
	{
	}

	SingleLinkListNode<T> & operator[](const size_t Index)
	{
		SingleLinkListNode<T>* Result = &Root;

		for (size_t i = 0; i < Index; i++)
		{
			Result = Result->GetChild();
		}

		return *Result;
	}

	void PushBack(const T & _Obj)
	{
		SingleLinkListNode<T>* Next = &Root;

		uint32 i = 0;
		while (Next->GetChild() != nullptr)
		{
			Next = Next->GetChild();
			i++;
		}

		Next->Child = new SingleLinkListNode<T>(_Obj);
	}

	int32 Find(const T& _Obj)
	{
		SingleLinkListNode<T>* l_Next;
		uint32 i = 0;

		while (l_Next->GetChild() != nullptr)
		{
			if (*l_Next == _Obj)
			{
				return i;
			}

			l_Next = l_Next->GetChild();
			i++;
		}

		return -1;
	}

	INLINE uint32 Length() const { return m_Length; }
protected:
	SingleLinkListNode<T> Root;

	uint32 m_Length = 0;
};