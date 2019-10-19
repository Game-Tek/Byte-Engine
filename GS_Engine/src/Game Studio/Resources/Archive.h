#pragma once

#include "Core.h"

class Archive
{
public:
	explicit Archive(void* _Stream) : stream(_Stream) {}

	void Write(int8 _In) const { writeInternal(sizeof(int8), &_In); }

	void Write(uint8 _In) const { writeInternal(sizeof(uint8), &_In); }

	void Write(int16 _In) const { writeInternal(sizeof(int16), &_In); }

	void Write(uint16 _In) const { writeInternal(sizeof(uint16), &_In); }

	void Write(int32 _In) const { writeInternal(sizeof(int32), &_In); }

	void Write(uint32 _In) const { writeInternal(sizeof(uint32), &_In); }

	void Write(int64 _In) const { writeInternal(sizeof(int64), &_In); }

	void Write(uint64 _In) const { writeInternal(sizeof(uint64), &_In); }

	void Write(int_64 _In) const { writeInternal(sizeof(int_64), &_In); }

	void Write(uint_64 _In) const { writeInternal(sizeof(uint_64), &_In); }

	void Write(size_t _Size, void* _In) const { writeInternal(_Size, _In); }

	
	void Read(int8* _In) const { readInternal(sizeof(int8), _In); }

	void Read(uint8* _In) const { readInternal(sizeof(uint8), _In); }

	void Read(int16* _In) const { readInternal(sizeof(int16), _In); }

	void Read(uint16* _In) const { readInternal(sizeof(uint16), _In); }

	void Read(int32* _In) const	{ readInternal(sizeof(int32), _In); }

	void Read(uint32* _In) const { readInternal(sizeof(uint32), _In); }

	void Read(int64* _In) const	{ readInternal(sizeof(int64), _In); }

	void Read(uint64* _In) const { readInternal(sizeof(uint64), _In); }

	void Read(int_64* _In) const { readInternal(sizeof(int_64), _In); }

	void Read(uint_64* _In) const { readInternal(sizeof(uint_64), _In); }
	
private:
	/**
	 * \brief
	 * Pointer to the stream implementation. I.E: std::fstream; boost::fstream.
	 * Used to hide the actual library used, this for use simplicity and to avoid inclusion of
	 * huge library headers which pollute the namespace.
	 */
	void* stream = nullptr;

	void writeInternal(size_t _Size, void* _Data) const;
	void readInternal(size_t _Size, void* _Data) const;
};