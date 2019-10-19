#include "Archive.h"

#include <fstream>

using StreamType = std::fstream;

void Archive::writeInternal(const size_t _Size, void* _Data) const
{
	auto stream_ = SCAST(StreamType*, stream);

	//stream_->write(reinterpret_cast<char*>(const_cast<size_t*>(&_Size)), sizeof(size_t));
	stream_->write(static_cast<char*>(_Data), _Size);
}

void Archive::readInternal(size_t _Size, void* _Data) const
{
	auto stream_ = SCAST(StreamType*, stream);

	stream_->read(static_cast<char*>(_Data), _Size);
}
