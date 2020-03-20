#pragma once

#include "Core.h"

#include "FVector.hpp"
#include "Resources/Stream.h"

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

	explicit FString(char* const cstring);

	FString(const char* cstring) : data(StringLength(cstring), const_cast<char*>(cstring))
	{
	}

	explicit FString(const length_type length) : data(length)
	{
	}

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

	FString& operator=(const char*);
	FString& operator=(const FString& string) = default;
	FString operator+(const char* cstring) const;
	FString& operator+=(const char* cstring);
	FString operator+(const FString& string) const;

	string_type operator[](const length_type i) { return data[i]; }
	string_type operator[](const length_type i) const { return data[i]; }

	auto begin() { return data.begin(); }
	[[nodiscard]] auto begin()const { return data.begin(); }
	auto end() { return data.end(); }
	[[nodiscard]] auto end() const { return data.end(); }

	[[nodiscard]] length_type npos() const { return data.getLength() + 1; }
	
	//Returns true if the two FString's contents are the same. Comparison is case sensitive.
	bool operator==(const FString& Other) const;

	//Returns true if the two FString's contents are the same. Comparison is case insensitive.
	[[nodiscard]] bool NonSensitiveComp(const FString& _Other) const;

	//Returns the contents of this FString as a C-String.
	char* c_str() { return data.getData(); }

	//Returns the contents of this FString as a C-String.
	[[nodiscard]] const char* c_str() const { return data.getData(); }

	//Return the length of this FString. Does not take into account the null terminator character.
	INLINE length_type GetLength() const { return data.getLength() - 1; }
	//Returns whether this FString is empty.
	INLINE bool IsEmpty() const { return data.getLength() == 0; }

	//Places a the c-string after this FString with a space in the middle.
	void Append(const char* cstring);
	//Places the FString after this FString with a space in the middle.
	void Append(const FString& string);

	void Append(int_64 number);
	void Append(float number);

	//Places the passed in FString at the specified Index.
	void Insert(const char* In, size_t Index);

	//Returns the index to the last character in the string that is equal to _Char, if no matching character is found -1 is returned.
	[[nodiscard]] length_type FindLast(char _Char) const;

	[[nodiscard]] length_type FindFirst(char c) const;
	
	/**
	 * \brief Drops/removes the parts of the string from from forward.
	 * \param from index to cut forward from.
	 */
	void Drop(int64 from);

	void ReplaceAll(char a, char with);
	void ReplaceAll(const char* a, const char* with);

	//Returns the length of the C-String accounting for the null terminator character. C-String MUST BE NULL TERMINATED.
	constexpr static length_type StringLength(const char* In);

	static FString MakeString(const char* _Text, ...);
private:
	friend OutStream& operator<<(OutStream& _Archive, FString& _String);

	friend InStream& operator>>(InStream& _Archive, FString& _String);

	FVector<string_type> data;

	static char ToLowerCase(char _Char);
	static char ToUpperCase(char _Char);
};
