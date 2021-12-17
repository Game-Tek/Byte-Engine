#pragma once

namespace BE {
	struct TypeIdentifer {
		uint16 SystemId = 0xFFFF, TypeId = 0xFFFF;
	};

	struct Handle {
		Handle(TypeIdentifer type_identifier, uint32 handle) : Identifier(type_identifier), EntityIndex(handle) {}

		uint32 operator()() const { return EntityIndex; }

		const TypeIdentifer Identifier;
		uint32 EntityIndex = 0xFFFFFFFF;
	};

	static_assert(sizeof(Handle) <= 8);
}