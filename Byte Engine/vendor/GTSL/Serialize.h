#pragma once

#include "FString.h"
#include "Stream.h"

template <typename T>
void operator<<(OutStream& outStream, FVector<T>& vector)
{
	outStream.Write(vector.getLength());

	for (auto& e : vector) { outStream << e; }
}

template <typename T>
void operator>>(InStream& inStream, FVector<T>& vector)
{
	typename FVector<T>::length_type length = 0;

	inStream.Read(&length);

	vector.resize(length);

	for (auto& e : vector) { inStream >> e; }
}

inline OutStream& operator<<(OutStream& archive, FString& string)
{
	archive << string.data;	return archive;
}

inline InStream& operator>>(InStream& archive, FString& string)
{
	archive >> string.data; return archive;
}