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
	//FString(const char(&_Literal)[N]) : Data(N, *_Literal)
	//{
	//}

	explicit FString(char* const _In);
	
	FString(const char* In);

	explicit FString(length_type length);

	FString(length_type Length, const char* In);

	FString(const length_type length, const FString& string) : Data(length, string.Data.getData())
	{
	}

	FString(const length_type length, const FString& string, const length_type offset) : Data(length, string.Data.getData() + offset)
	{
	}

	FString(const FString& Other) = default;

	~FString() = default;

	FString& operator=(const char*);
	FString& operator=(const FString& Other) = default;
	FString operator+(const char* _In) const;
	FString& operator+=(const char* _In);
	FString operator+(const FString& Other) const;

	string_type operator[](const length_type _Index) { return Data[_Index]; }
	string_type operator[](const length_type _Index) const { return Data[_Index]; }

	auto begin() { return Data.begin(); }
	[[nodiscard]] auto begin()const { return Data.begin(); }
	auto end() { return Data.end(); }
	[[nodiscard]] auto end() const { return Data.end(); }

	[[nodiscard]] length_type npos() const { return Data.getLength() + 1; }
	
	//Returns true if the two FString's contents are the same. Comparison is case sensitive.
	bool operator==(const FString& Other) const;

	//Returns true if the two FString's contents are the same. Comparison is case insensitive.
	[[nodiscard]] bool NonSensitiveComp(const FString& _Other) const;

	//Returns the contents of this FString as a C-String.
	char* c_str() { return Data.getData(); }

	//Returns the contents of this FString as a C-String.
	[[nodiscard]] const char* c_str() const { return Data.getData(); }

	//Return the length of this FString. Does not take into account the null terminator character.
	INLINE size_t GetLength() const { return Data.getLength() - 1; }
	//Returns whether this FString is empty.
	INLINE bool IsEmpty() const { return Data.getLength() == 0; }

	//Places a the C-FString after this FString with a space in the middle.
	void Append(const char* In);
	//Places the FString after this FString with a space in the middle.
	void Append(const FString& In);

	//Places the passed in FString at the specified Index.
	void Insert(const char* In, size_t Index);

	//Returns the index to the last character in the string that is equal to _Char, if no matching character is found -1 is returned.
	[[nodiscard]] int64 FindLast(char _Char) const;

	length_type FindFirst(char c) const;
	
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

	FVector<string_type> Data;

	static char ToLowerCase(char _Char);
	static char ToUpperCase(char _Char);
};
