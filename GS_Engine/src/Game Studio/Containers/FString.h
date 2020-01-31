#pragma once

#include "Core.h"

#include "FVector.hpp"
#include "Resources/Stream.h"

class GS_API FString
{
	using string_type = char;
public:
	//Constructs an empty FString.
	FString();

	//template<size_t N>
	//FString(const char(&_Literal)[N]) : Data(N, *_Literal)
	//{
	//}

	explicit FString(char* const _In);

	//Constructs an FString from a C-String.
	FString(const char* In);

	explicit FString(size_t _Length);

	//Constructs a FString from a non null terminated character array.
	FString(size_t Length, const char* In);

	FString(const FString& Other) = default;

	~FString() = default;

	FString& operator=(const char*);
	FString& operator=(const FString& Other) = default;
	FString operator+(const char* _In) const;
	FString& operator+=(const char* _In);
	FString operator+(const FString& Other) const;

	string_type operator[](const size_t _Index) { return Data[_Index]; }
	string_type operator[](const size_t _Index) const { return Data[_Index]; }

	auto begin() { return Data.begin(); }
	[[nodiscard]] auto begin()const { return Data.begin(); }
	auto end() { return Data.end(); }
	[[nodiscard]] auto end() const { return Data.end(); }

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

	/**
	 * \brief Drops/removes the parts of the string from from forward.
	 * \param from index to cut forward from.
	 */
	void Drop(int64 from);

	void ReplaceAll(char a, char with);

	//Returns the length of the C-String accounting for the null terminator character. C-String MUST BE NULL TERMINATED.
	constexpr static size_t StringLength(const char* In);

	static FString MakeString(const char* _Text, ...);
private:
	friend OutStream& operator<<(OutStream& _Archive, FString& _String);

	friend InStream& operator>>(InStream& _Archive, FString& _String);

	FVector<string_type> Data;

	static char ToLowerCase(char _Char);
	static char ToUpperCase(char _Char);
};
