#pragma once

const int DEFAULT_ARRAY_SIZE = 5;

template <typename ArrayElementType>
class GSArray
{
public:
	GSArray(int N);
	~GSArray();
	void AddElement(ArrayElementType Element);
	int GetLastIndex();
	int GetArrayLength();

private:
	ArrayElementType * Array;																			//Pointer to the array.
	int NumberOfElements;																				//Number of elements the array currently holds occupied. NOT BASE 0.
	int TotalNumberOfElements;																			//The size of the array including the unnocuppied elements. NOT BASE 0.

	ArrayElementType * AllocateNewArray(int N);
	void FillArray(GSArray & ArrayToFill, GSArray & SourceArray);
};

template <typename ArrayElementType>
GSArray<ArrayElementType>::GSArray(int N)
{
	Array = AllocateNewArray((N < DEFAULT_ARRAY_SIZE) ? DEFAULT_ARRAY_SIZE : N);
}

template <typename ArrayElementType>
GSArray<ArrayElementType>::~GSArray()
{
	delete[] Array;
}

template<typename ArrayElementType>
inline void GSArray<ArrayElementType>::AddElement(ArrayElementType Element)
{
	if (NumberOfElements + 1 > TotalNumberOfElements)													//We check if adding a new element will exceed the allocated elements.
	{
		ArrayElementType * NewArray = AllocateNewArray(NumberOfElements + 1 + DEFAULT_ARRAY_SIZE);		//We allocate a new array to a temp pointer.

		FillArray(NewArray, Array);																		//We fill the new array with the contents of the old/current one.

		delete[] Array;																					//We delete the old array which Array is pointing to.

		Array = NewArray;																				//We set the Array pointer to the recently created and filled array.

		TotalNumberOfElements = NumberOfElements + 1 + DEFAULT_ARRAY_SIZE;								//We update the total number of elements count.
	}

	else
	{
		Array[NumberOfElements] = Element;																//We set the last index + 1 as the Element parameter.
	}

	NumberOfElements += 1;																				//We update the number of elements count.

	return;
}

template<typename ArrayElementType>
inline int GSArray<ArrayElementType>::GetLastIndex()
{
	return NumberOfElements - 1;
}

template<typename ArrayElementType>
inline int GSArray<ArrayElementType>::GetArrayLength()
{
	return NumberOfElements;
}

template<typename ArrayElementType>
inline ArrayElementType * GSArray<ArrayElementType>::AllocateNewArray(int N)
{
	return new ArrayElementType[N];
}

template<typename ArrayElementType>
inline void GSArray<ArrayElementType>::FillArray(GSArray & ArrayToFill, GSArray & SourceArray)
{
	for (i = 0; i < NumberOfElements, i++)
	{
		ArrayToFill[i] = SourceArray[i];
	}
}
