#pragma once

#include "Core.h"

#include "FVector.hpp"

template<class T>
GS_STRUCT SingleLinkListNode
{
	SingleLinkListNode();

	SinglyLinkedListNode * GetChild() { return Child; }
	T & GetElement() { return Element; }

protected:
	SinglyLinkedListNode<T> * Child;

	T Element;
};

template <class T>
GS_CLASS SingleLinkList
{
public:
	SingleLinkList();

	SingleLinkList(const size_t Length);

	SingleLinkListNode & operator[](const size_t Index);
protected:
	SingleLinkListNode<T> Root;
};

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