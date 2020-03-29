#pragma once

#include "Core.h"
#include "Pair.h"

template <class _T>
class SingleLinkList
{
	struct SingleLinkListNode
	{
		friend class SingleLinkList;

		SingleLinkListNode() = default;

		explicit SingleLinkListNode(const _T& _Obj) : element(_Obj)
		{
		}

		_T& GetElement() { return element; }
		[[nodiscard]] const _T& GetElement() const { return element; }

	protected:
		SingleLinkListNode* GetNext() { return next; }

		SingleLinkListNode* next = nullptr;

		_T element;
	};

public:
	using ResultPair = Pair<bool, SingleLinkListNode*>;

	SingleLinkList() = default;

	//Preallocate
	explicit SingleLinkList(const size_t _Length) : length(_Length)
	{
	}

	SingleLinkListNode& operator[](const size_t Index)
	{
		SingleLinkListNode* result = &root;

		for (size_t i = 0; i < Index; ++i)
		{
			result = result->GetChild();
		}

		return *result;
	}

	void PushBack(const _T& _Obj)
	{
		auto new_node = new SingleLinkListNode(_Obj);
		lastNode->next = new_node; //<== Now last node
		lastNode = new_node;
	}

	void PopBack()
	{
		auto new_last = (*this)[length - 1];
		new_last.next = nullptr;
		delete lastNode;
		lastNode = new_last;
	}

	ResultPair Find(const _T& _Obj)
	{
		SingleLinkListNode* next = &root;
		uint32 i = 0;

		while (next->GetChild() != nullptr)
		{
			if (*next == _Obj)
			{
				return {true, next};
			}

			next = next->GetChild();
			i++;
		}

		return {false, nullptr};
	}

	INLINE uint32 Length() const { return length; }

protected:
	SingleLinkListNode root;
	SingleLinkListNode* lastNode = &root;

	uint32 length = 0;
};
