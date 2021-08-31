#pragma once

#include <GTSL/Tree.hpp>
#include "ByteEngine/Application/AllocatorReferences.h"

class StringLookup {
public:

	void AddKey(const GTSL::Range<const char8_t*> string) {
		Node* start = nullptr;

		for (auto e : string) {
			start = start->in[(uint8)e];
		}
	}

	template<typename C>
	void Lookup(const GTSL::Range<const char8_t*> string, C& container) {
		Node* start = nullptr;

		for (auto e : string) {
			start = start->in[(uint8)e];
		}

		auto moveThroughTree = [&](const Node* node, auto&& self) {

		};

		moveThroughTree()
	}

private:
	struct Node {
		GTSL::StaticVector<void*, 128> in;
	};

	GTSL::Tree<Node, BE::PAR> tree;
};