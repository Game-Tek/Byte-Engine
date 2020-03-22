#pragma once

/**
 * \brief Base class for most non-data only classes in the engine.
 */
class Object
{
public:
	Object() = default;
	virtual ~Object() = default;

	[[nodiscard]] virtual const char* GetName() const = 0;
};
