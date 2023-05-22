#pragma once

#include "Debug/Assert.h"
#include <GTSL/Core.h>
#include <GTSL/Vector.hpp>

template<typename T>
struct Graph
{
private:
	struct Internal
	{
		Internal(T t) : data(t) {}

		~Internal()
		{
			for (auto i = 0; i < downstreamCount; i++)
				downstream[i] = nullptr;
		}

		T data;
		Internal* downstream[64]{};
		Internal* upstream[64]{};
		GTSL::uint32 downstreamCount = 0, upstreamCount = 0;
	};

	Internal* m_internal = nullptr;
	bool m_shared = false;
public:
	explicit Graph(T t) : m_internal(t) {}
	Graph(Internal* internal) : m_internal(internal),m_shared(true) {}

	Graph(Graph&& other) noexcept
		: m_internal(other.m_internal)
	{
		other.m_internal = nullptr;
	}

	Graph(const Graph& other)
		: m_internal(other.GetData())
	{
		m_internal->downstreamCount = other.m_internal->downstreamCount;
		m_internal->upstreamCount = other.m_internal->upstreamCount;

		for (auto i = 0; i < m_internal->downstreamCount; ++i)
			m_internal->downstream[i] = other.m_internal->downstream[i];

		for (auto i = 0; i < m_internal->upstreamCount; ++i)
			m_internal->upstream[i] = other.m_internal->upstream[i];
	}

	~Graph()
	{
		if (m_internal && !m_shared) delete m_internal;
		m_internal = nullptr;
	}

	void Connect(Graph& other)
	{
		for(auto i = 0; i < other.m_internal->upstreamCount; ++i)
		{
			if (other.m_internal->upstream[i] == other.m_internal)
				BE_DEBUG_BREAK;
		}

		m_internal->downstream[m_internal->downstreamCount++] = other.m_internal;
		other.m_internal->upstream[other.m_internal->upstreamCount++] = m_internal;
	}

	auto GetParents() const
	{
		GTSL::StaticVector<Graph, 64> parent;

		for (auto i = 0; i < m_internal->upstreamCount; ++i)
			parent.EmplaceBack(m_internal->upstream[i]);

		return parent;
	}

	auto GetChildren() const
	{
		GTSL::StaticVector<Graph, 64> children;

		for (auto i = 0; i < m_internal->downstreamCount; ++i)
			children.EmplaceBack(m_internal->downstream[i]);

		return children;
	}

	T& GetData() { return m_internal->data; }
	const T& GetData() { return m_internal->data; }
};