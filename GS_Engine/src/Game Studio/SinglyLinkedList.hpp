#pragma once

#include "Core.h"

#include "FVector.hpp"

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
protected:
	SingleLinkListNode<T> Root;
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
