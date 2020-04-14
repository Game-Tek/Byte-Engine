#pragma once

#include "Stream.h"

template <typename T>
void operator<<(OutStream& outStream, GTSL::Vector<T>& vector)
{
	outStream.Write(vector.GetLength());

	for (auto& e : vector) { outStream << e; }
}

template <typename T>
void operator>>(InStream& inStream, GTSL::Vector<T>& vector)
{
	typename GTSL::Vector<T>::length_type length = 0;

	inStream.Read(&length);

	vector.Resize(length);

	for (auto& e : vector) { inStream >> e; }
}