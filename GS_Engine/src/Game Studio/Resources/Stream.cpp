#include "Stream.h"

#include <fstream>

using InStreamType  = std::ifstream;
using OutStreamType = std::ofstream;

void InStream::readInternal(size_t _Size, void* _Data) const
{
	auto stream_ = SCAST(InStreamType*, stream);

	stream_->read(static_cast<char*>(_Data), _Size);
}

void OutStream::writeInternal(const size_t _Size, void* _Data) const
{
	auto stream_ = SCAST(OutStreamType*, stream);

	//stream_->write(reinterpret_cast<char*>(const_cast<size_t*>(&_Size)), sizeof(size_t));
	stream_->write(static_cast<char*>(_Data), _Size);
}