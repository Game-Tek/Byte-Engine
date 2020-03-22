#include "Stream.h"

#include <fstream>

using InStreamType = std::ifstream;
using OutStreamType = std::ofstream;

void InStream::readInternal(const size_t size, void* data) const
{
	auto stream_ = static_cast<InStreamType*>(stream);
	stream_->read(static_cast<char*>(data), size);
}

void OutStream::writeInternal(const size_t size, void* data) const
{
	auto stream_ = static_cast<OutStreamType*>(stream);
	stream_->write(static_cast<char*>(data), size);
}
