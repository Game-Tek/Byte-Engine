#pragma once
#include "FVector.hpp"

/**
 * \brief A vector that maintains indices for placed objects during their lifetime.
 * \tparam T Type of the object this KVector will store.
 */
template<typename T>
class KVector
{
public:
	using length_type = typename FVector<T>::length_type;

private:
	FVector<T> objects;
	FVector<uint32> freeIndeces;

	/**
	 * \brief Tries to find a free index.
	 * \param index reference to a length_type variable to store the free index.
	 * \return A bool indicating whether or not a free index was found.
	 */
	bool findFreeIndex(length_type& index)
	{
		if (freeIndeces.getLength() == 0) //If there aren't any free indeces return false,
		{
			return false;
		}	//if there are, grab the bottom one and pop it from the list so it is no longer available.
		const auto free_index = freeIndeces[0];
		freeIndeces.pop(0);
		index = free_index;
		return true;
	}
public:
	auto begin() { return objects.begin(); }
	auto end() { return objects.end(); }

	explicit KVector(const length_type min) : objects(min), freeIndeces(min, min)
	{
		//Allocate objects space for min objects
		//Allocate min indeces and set it's length as min so they are marked as used

		//Fill every element in freeIndeces with it's corresponding index so they are available for using(marked as free)(because every index/element in objects is now free).
		
		length_type i = 0;
		for (auto& e : freeIndeces)	{ e = i; }
	}
	
	/**
	 * \brief Inserts an object into the vector at the first free slot available.
	 * \param obj Object reference to insert.
	 * \return Index at which the object was inserted.
	 */
	length_type Insert(const T& obj)
	{
		length_type index = 0;

		if(findFreeIndex(index)) //If there is a free index insert there,
		{
			objects.place(index, obj);
			return index;
		}

		//if there wasn't a free index place a the back of the array.
		return objects.push_back(obj);
	}

	/**
	 * \brief Emplaces an object into the vector at the first free slot available.
	 * \param args Arguments for construction of object of type T.
	 * \return Index at which it was emplaced.
	 */
	template <typename... Args>
	length_type Emplace(Args&&... args)
	{
		length_type index = 0;

		if(findFreeIndex(index)) //If there is a free index insert there,
		{
			objects.emplace(index, std::forward<Args>(args) ...);
			return index;
		}
		
		//if there wasn't a free index place a the back of the array.
		return objects.emplace_back(std::forward<Args>(args) ...);
	}
	
	/**
	 * \brief Destroys the object at the specified index which makes space for another object to take it's place.
	 * \param index Index of the object to remove.
	 */
	void Destroy(const length_type index)
	{
		freeIndeces.push_back(index); //index is now free, make it available

		if(index == objects.getLength()) //If the object is the last in the array pop it,
		{
			objects.pop_back();
		}

		//if it isn't (it's somewhere in the middle) destroy the object at that index.
		objects.destroy(index);
	}
};
