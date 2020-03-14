#pragma once

#include "Core.h"

#include "Containers/FString.h"

#include "Stream.h"

using ResourceHeaderType = uint64;
using ResourceSegmentType = uint64;

template <typename T>
void SerializeFVector(OutStream& outStream, FVector<T>& vector)
{
	outStream.Write(vector.getLength());

	for (uint_64 i = 0; i < vector.getLength(); ++i)
	{
		outStream << vector[i];
	}
}

template <typename T>
void operator<<(OutStream& outStream, FVector<T>& vector)
{
	outStream.Write(vector.getLength());

	for (uint_64 i = 0; i < vector.getLength(); ++i)
	{
		outStream << vector[i];
	}
}

template <typename T>
void operator>>(InStream& inStream, FVector<T>& vector)
{
	typename FVector<T>::length_type length = 0;

	inStream.Read(&length);

	vector.init(length);
	vector.resize(length);

	for (uint_64 i = 0; i < length; ++i)
	{
		inStream >> vector[i];
	}
}

template <typename T>
void DeserializeFVector(InStream& inStream, FVector<T>& vector)
{
	typename FVector<T>::length_type length = 0;

	inStream.Read(&length);

	vector.resize(length);

	for (uint_64 i = 0; i < length; ++i)
	{
		inStream >> vector[i];
	}
}