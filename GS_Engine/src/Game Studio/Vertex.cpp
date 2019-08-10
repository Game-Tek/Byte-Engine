#include "Vertex.h"

static ShaderDataTypes Elements[] = { ShaderDataTypes::FLOAT2, ShaderDataTypes::FLOAT2 };
VertexDescriptor Vertex2D::Descriptor = VertexDescriptor(FVector<ShaderDataTypes>(Elements, 2));