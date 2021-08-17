#pragma once

#include "Pipelines.h"
#include "GTSL/Buffer.hpp"

namespace GAL
{
	template<class ALLOC>
	void Insert(const Pipeline::VertexElement& vertexElement, GTSL::Buffer<ALLOC>& buffer)
	{
		Insert(vertexElement.Identifier, buffer);
		Insert(vertexElement.Type, buffer);
	}

	template<class ALLOC>
	void Extract(Pipeline::VertexElement& vertexElement, GTSL::Buffer<ALLOC>& buffer)
	{
		Extract(vertexElement.Identifier, buffer);
		Extract(vertexElement.Type, buffer);
	}
}
