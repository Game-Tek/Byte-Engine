#pragma once

#include "Core.h"

/**
 * \brief Is an interface for de-serialization of data.
 * Provides methods to read data from disk.
 * Is a wrapper for the actual implementation of the stream code.
 */
class InStream
{
public:
	//Default constructor is deleted since this class cannot exist without a pointer to a file stream.
	InStream() = delete;

	/**
	 * \brief Constructs an Archive which can be used for writing/reading to/from disk.
	 * \param _Stream Pointer to a file stream of the type the Archive implementation uses.
	 */
	explicit InStream(void* _Stream) : stream(_Stream)
	{
	}

	explicit InStream(const InStream& _Archive) = default;

	explicit InStream(InStream&& _Archive) = default;

	~InStream() = default;

	InStream& operator=(const InStream& _Other) = default;
	InStream& operator=(InStream&& _Other) = default;

	void operator>>(uint8& in) const { readInternal(sizeof(uint8), &in); }
	void operator>>(int8& in) const { readInternal(sizeof(int8), &in); }

	/**
	 * \brief Writes an int8 to memory from disk.
	 * \param _In Pointer to an int8 the data will be read to.
	 */
	void Read(int8* _In) const { readInternal(sizeof(int8), _In); }

	/**
	* \brief Writes an uint8 to memory from disk.
	* \param _In Pointer to an uint8 the data will be read to.
	*/
	void Read(uint8* _In) const { readInternal(sizeof(uint8), _In); }

	/**
	* \brief Writes an int16 to memory from disk.
	* \param _In Pointer to an int16 the data will be read to.
	*/
	void Read(int16* _In) const { readInternal(sizeof(int16), _In); }

	/**
	* \brief Writes an uint16 to memory from disk.
	* \param _In Pointer to an uint16 the data will be read to.
	*/
	void Read(uint16* _In) const { readInternal(sizeof(uint16), _In); }

	/**
	* \brief Writes an int32 to memory from disk.
	* \param _In Pointer to an int32 the data will be read to.
	*/
	void Read(int32* _In) const { readInternal(sizeof(int32), _In); }

	/**
	* \brief Writes an uint32 to memory from disk.
	* \param _In Pointer to an uint32 the data will be read to.
	*/
	void Read(uint32* _In) const { readInternal(sizeof(uint32), _In); }

	/**
	* \brief Writes an int64 to memory from disk.
	* \param _In Pointer to an int64 the data will be read to.
	*/
	void Read(int64* _In) const { readInternal(sizeof(int64), _In); }

	/**
	* \brief Writes an uint64 to memory from disk.
	* \param _In Pointer to an uint64 the data will be read to.
	*/
	void Read(uint64* _In) const { readInternal(sizeof(uint64), _In); }

	/**
	* \brief Writes the data starting At _In up to _Size bytes from disk to memory.
	* \param _Size Bytes to be read from disk.
	* \param _Data Pointer to data to be read from disk to memory.
	*/
	void Read(const size_t _Size, void* _Data) const { readInternal(_Size, _Data); }

private:
	/**
	 * \brief
	 * Pointer to the stream implementation. I.E: std::fstream; boost::stream.
	 * Used to hide the actual library used, this for use simplicity and to avoid inclusion of
	 * huge library headers which pollute the namespace.
	 */
	void* stream = nullptr;

	/**
	 * \brief Actual implementation of the reading from disk functionality.
	 * Library code should be inside this function.
	 * \param size Size of the data being read into memory.
	 * \param data Pointer to the location the read data must be copied to.
	 */
	void readInternal(size_t size, void* data) const;
};

/**
 * \brief Is an interface for serialization of data.
 * Provides methods to write data to disk.
 * Is a wrapper for the actual implementation of the stream code.
 */
class OutStream
{
public:
	//Default constructor is deleted since this class cannot exist without a pointer to a file stream.
	OutStream() = delete;

	/**
	 * \brief Constructs an Archive which can be used for writing/reading to/from disk.
	 * \param _Stream Pointer to a file stream of the type the Archive implementation uses.
	 */
	explicit OutStream(void* _Stream) : stream(_Stream)
	{
	}

	explicit OutStream(const OutStream& _Archive) = default;

	explicit OutStream(OutStream&& _Archive) = default;

	~OutStream() = default;

	OutStream& operator=(const OutStream& _Other) = default;
	OutStream& operator=(OutStream&& _Other) = default;

	void operator<<(int8 in) const { writeInternal(sizeof(int8), &in); }
	void operator<<(uint8 in) const { writeInternal(sizeof(uint8), &in); }

	/**
	 * \brief Writes an int8 to disk.
	 * \param _In int8 to be written to disk.
	 */
	void Write(int8 _In) const { writeInternal(sizeof(int8), &_In); }

	/**
	* \brief Writes an uint8 to disk.
	* \param _In uint8 to be written to disk.
	*/
	void Write(uint8 _In) const { writeInternal(sizeof(uint8), &_In); }

	/**
	* \brief Writes an int16 to disk.
	* \param _In int16 to be written to disk.
	*/
	void Write(int16 _In) const { writeInternal(sizeof(int16), &_In); }

	/**
	* \brief Writes an uint16 to disk.
	* \param _In uint16 to be written to disk.
	*/
	void Write(uint16 _In) const { writeInternal(sizeof(uint16), &_In); }

	/**
	* \brief Writes an int32 to disk.
	* \param _In int32 to be written to disk.
	*/
	void Write(int32 _In) const { writeInternal(sizeof(int32), &_In); }

	/**
	* \brief Writes an uint32 to disk.
	* \param _In uint32 to be written to disk.
	*/
	void Write(uint32 _In) const { writeInternal(sizeof(uint32), &_In); }

	/**
	* \brief Writes an int64 to disk.
	* \param _In int64 to be written to disk.
	*/
	void Write(int64 _In) const { writeInternal(sizeof(int64), &_In); }

	/**
	* \brief Writes an uint64 to disk.
	* \param _In uint64 to be written to disk.
	*/
	void Write(uint64 _In) const { writeInternal(sizeof(uint64), &_In); }

	/**
	* \brief Writes the data starting At _In up to _Size bytes to disk.
	* \param _Size Bytes to be written to disk.
	* \param _In Pointer to data to be written to disk.
	*/
	void Write(const size_t _Size, void* _In) const { writeInternal(_Size, _In); }

private:
	/**
	 * \brief
	 * Pointer to the stream implementation. I.E: std::fstream; boost::stream.
	 * Used to hide the actual library used, this for use simplicity and to avoid inclusion of
	 * huge library headers which pollute the namespace.
	 */
	void* stream = nullptr;

	/**
	 * \brief Actual implementation of the writing to disk functionality.
	 * Library code should be inside this function.
	 * \param size Size of the data being written.
	 * \param data Pointer to the data being copied.
	 */
	void writeInternal(size_t size, void* data) const;
};
