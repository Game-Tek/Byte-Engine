#pragma once

#include "Core.h"

template<class T>
GS_STRUCT SingleLinkListNode
{
	SingleLinkListNode();

	SingleLinkListNode * GetChild() { return Child; }
	T & GetElement() { return Element; }

protected:
	SingleLinkListNode<T> * Child;

	T Element;
};

template <class T>
GS_CLASS SingleLinkList
{
public:
	SingleLinkList();

	explicit SingleLinkList(const size_t Length);

	SingleLinkListNode<T> & operator[](const size_t Index);

	void PushBack();

	int32 Find(const T& _Obj)
	{
		SingleLinkListNode* l_Next;
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
protected:
	SingleLinkListNode<T> Root;

	uint32 m_Length = 0;
};

template <class T>
SingleLinkList<T>::SingleLinkList()
{
}

template <class T>
SingleLinkList<T>::SingleLinkList(const size_t Length)
{
}

template <class T>
SingleLinkListNode<T> & SingleLinkList<T>::operator[](const size_t Index)
{
	SingleLinkListNode<T> * Result = &Root;

	for (size_t i = 0; i < Index; i++)
	{
		Result = Result->GetChild();
	}

	return *Result;
}
