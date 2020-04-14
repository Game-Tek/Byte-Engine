#pragma once

#include <GTM/Vector2.h>
#include <GTM/Vector3.h>
#include <GTSL/TextureCoordinates.h>
#include <GAL/RenderMesh.h>

using Index = uint16;

struct Vertex2D
{
	Vector2 Position;
	TextureCoordinates TextureCoordinates;
};

struct Vertex
{
	Vector3 Position;
	Vector3 Normal;
	TextureCoordinates TextCoord;
	Vector3 Tangent;
	Vector3 BiTangent;

	inline static const GAL::VertexDescriptor Descriptor{ { GAL::ShaderDataTypes::FLOAT3, GAL::ShaderDataTypes::FLOAT3, GAL::ShaderDataTypes::FLOAT2, GAL::ShaderDataTypes::FLOAT3, GAL::ShaderDataTypes::FLOAT3 } };
};

