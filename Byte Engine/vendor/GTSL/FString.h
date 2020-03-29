#pragma once

#include "Core.h"

#include "FVector.hpp"

class FString
{
	using string_type = char;
	using length_type = FVector<string_type>::length_type;
public:
	//Constructs an empty FString.
	FString();

	//template<size_t N>
	//FString(const char(&_Literal)[N]) : data(N, *_Literal)
	//{
	//}

	FString(const char* cstring) : data(StringLength(cstring), const_cast<char*>(cstring)) {}

	/**
	 * \brief Creates an FString with enough space allocated for length elements.
	 * \param length Amount of elements to allocate.
	 */
	explicit FString(const length_type length) : data(length) {}

	/**
	 * \brief Creates an FString from a length and an array. Assumes the array has no null terminator character. If the array you pass happens to have a null terminator you should insert one character less.
	 * \param length length to use from the cstring array
	 * \param cstring array to copy from 
	 */
	FString(const length_type length, const char* cstring) : data(length + 1, const_cast<char*>(cstring)) { data.push_back('\0'); }

	/**
	 * \brief Creates an FString from a length and an FString. Assumes the string has no null terminator character. If the string you pass happens to have a null terminator you should insert one character less.
	 * \param length Length to use from the FString.
	 * \param string String to copy characters from.
	 */
	FString(const length_type length, const FString& string) : data(length, string.data.getData()) { data.push_back('\0'); }

	/**
	 * \brief Creates an FString from a length an FString and an offset. Assumes the string has no null terminator character. If the string you pass happens to have a null terminator you should insert one character less.
	 * \param length Length to use from the FString.
	 * \param string String to copy from.
	 * \param offset Offset from the start of the string to start copying from.
	 */
	FString(const length_type length, const FString& string, const length_type offset) : data(length, string.data.getData() + offset) { data.push_back('\0'); }

	FString(const FString& other) = default;

	~FString() = default;

	FString& operator=(const char* cstring);
	FString& operator=(const FString& string) = default;
	FString& operator+=(char c);	
	FString& operator+=(const char* cstring);
	FString& operator+=(const FString& string);

	string_type operator[](const length_type i) { return data[i]; }
	string_type operator[](const length_type i) const { return data[i]; }

	auto begin() { return data.begin(); }
	[[nodiscard]] auto begin()const { return data.begin(); }
	auto end() { return data.end(); }
	[[nodiscard]] auto end() const { return data.end(); }

	[[nodiscard]] length_type npos() const { return data.getLength() + 1; }
	
	//Returns true if the two FString's contents are the same. Comparison is case sensitive.
	bool operator==(const FString& other) const;

	//Returns true if the two FString's contents are the same. Comparison is case insensitive.
	[[nodiscard]] bool NonSensitiveComp(const FString& other) const;

	//Returns the contents of this FString as a C-String.
	char* c_str() { return data.getData(); }

	//Returns the contents of this FString as a C-String.
	[[nodiscard]] const char* c_str() const { return data.getData(); }

	//Return the length of this FString. Does not take into account the null terminator character.
	[[nodiscard]] length_type GetLength() const { return data.getLength() - 1; }
	//Returns whether this FString is empty.
	[[nodiscard]] bool IsEmpty() const { return data.getLength() == 0; }

	//Places a the c-string after this FString with a space in the middle.
	void Append(const char* cstring);
	//Places the FString after this FString with a space in the middle.
	void Append(const FString& string);

	void Append(uint8 number);
	void Append(int8 number);
	void Append(uint16 number);
	void Append(int16 number);
	void Append(uint32 number);
	void Append(int32 number);
	void Append(uint64 number);
	void Append(int64 number);
	void Append(float number);
	void Append(double number);

	/**
	 * \brief Places cstring at the specified index.
	 * \param cstring C-String to insert in the string.
	 * \param index Index at which to place the cstring.
	 */
	void Insert(const char* cstring, const length_type index);

	/**
	* \brief Returns an index to the first char in the string that is equal to c. If no such character is found npos() is returned.
	* \param c Char to find.
	* \return Index to found char.
	*/
	[[nodiscard]] length_type FindFirst(char c) const;
	
	/**
	 * \brief Returns an index to the last char in the string that is equal to c. If no such character is found npos() is returned.
	 * \param c Char to find.
	 * \return Index to found char.
	 */
	[[nodiscard]] length_type FindLast(char c) const;
	
	/**
	 * \brief Drops/removes the parts of the string from from forward.
	 * \param from index to cut forward from.
	 */
	void Drop(length_type from);

	void ReplaceAll(char a, char with);
	void ReplaceAll(const char* a, const char* with);

	//Returns the length of the C-String accounting for the null terminator character. C-String MUST BE NULL TERMINATED.
	constexpr static length_type StringLength(const char* cstring);

	static FString MakeString(const char* cstring, ...);
private:
	FVector<string_type> data;

	friend class InStream;
	friend class OutStream;

	friend class OutStream& operator<<(OutStream& archive, FString& string);
	friend class InStream& operator>>(InStream& archive, FString& string);
	
	static char toLowerCase(char c);
	static char toUpperCase(char c);
};
