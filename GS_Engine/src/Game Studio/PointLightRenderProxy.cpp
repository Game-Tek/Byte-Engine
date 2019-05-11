#include "PointLightRenderProxy.h"

#include "GL.h"
#include <GLAD/glad.h>

#include "GSM.hpp"

Vector3 * PointLightRenderProxy::MeshLoc = PointLightRenderProxy::GenVertices(16, 16);
uint32 *  PointLightRenderProxy::IndexLoc = PointLightRenderProxy::GenIndices(16, 16);

PointLightRenderProxy::PointLightRenderProxy(WorldObject * Owner) : MeshRenderProxy(Owner, new VBO(MeshLoc, sizeof(*MeshLoc)), new IBO(IndexLoc, 16 * 16 * 3), new VAO(sizeof(Vector3)))
{
	VertexArray->Bind();
	VertexArray->CreateVertexAttribute(3, GL_FLOAT, false, sizeof(float) * 3);

	WorldObjectOwner = Owner;
}

PointLightRenderProxy::~PointLightRenderProxy()
{
}

void PointLightRenderProxy::Draw() const
{
	IndexBuffer->Bind();
	VertexArray->Bind();

	GS_GL_CALL(glDrawElements(GL_TRIANGLES, IndexBuffer->GetCount(), GL_UNSIGNED_INT, nullptr));
}

Vector3 * PointLightRenderProxy::GenVertices(const uint8 HSegments, const uint8 VSegments)
{
	//CODE BY SONG-HO
	//http://www.songho.ca/opengl/gl_sphere.html

	//RADIUS REPLACE WITH 1.0f

	Vector3  * ta_ = new Vector3[HSegments * VSegments];

	float x, y, z, xy;                            // vertex position
	float nx, ny, nz, lengthInv = 1.0f / 1.0f;    // normal
	float s, t;                                   // texCoord

	float sectorStep = 2 * GSM::PI / HSegments;
	float stackStep = GSM::PI / VSegments;
	float sectorAngle, stackAngle;

	for (uint16 i = 0; i <= VSegments; ++i)
	{
		stackAngle = GSM::PI / 2 - i * stackStep;       // starting from pi/2 to -pi/2
		xy = 1.0f * GSM::Cosine(GSM::RadiansToDegrees(stackAngle));            // r * cos(u)
		z = 1.0f * GSM::Sine(GSM::RadiansToDegrees(stackAngle));			    // r * sin(u)

		// add (sectorCount+1) vertices per stack
		// the first and last vertices have same position and normal, but different tex coords
		for (uint16 j = 0; j <= HSegments; ++j)
		{
			sectorAngle = j * sectorStep;           // starting from 0 to 2pi

			// vertex position
			x = xy * GSM::Cosine(GSM::RadiansToDegrees(sectorAngle));             // r * cos(u) * cos(v)
			y = xy * GSM::Sine(GSM::RadiansToDegrees(sectorAngle));             // r * cos(u) * sin(v)
			ta_[i + j] = Vector3(x, y, z);
		}
	}

	return ta_;
}

uint32 * PointLightRenderProxy::GenIndices(const uint8 HSegments, const uint8 VSegments)
{
	uint32 * ta_ = new uint32[HSegments * VSegments * 10];

	// indices
	//  k1--k1+1
	//  |  / |
	//  | /  |
	//  k2--k2+1
	uint32 k1, k2;

	uint16 add = 0;

	for (uint32 i = 0; i < VSegments; ++i)
	{
		k1 = i * (HSegments + 1);     // beginning of current stack
		k2 = k1 + HSegments + 1;      // beginning of next stack

		for (uint32 j = 0; j < HSegments; ++j, ++k1, ++k2)
		{
			// 2 triangles per sector excluding 1st and last stacks
			if (i != 0)
			{
				ta_[add] = k1;
				++add;

				ta_[add] = k2;
				++add;

				ta_[add] = k1 + 1;
				++add;    // k1---k2---k1+1
			}

			if (i != (VSegments - 1))
			{
				ta_[add] = k1 + 1;
				++add;

				ta_[add] = k2;
				++add;

				ta_[add] = k1 + 1;   // k1---k2---k1+1
				++add;
			}

			ta_[add] = k1;
			++add;

			ta_[add] = k2;
			++add;

			if (i != 0)  // horizontal lines except 1st stack
			{
				ta_[add] = k1;
				++add;

				ta_[add] = k1 + 1;
				++add;
			}
		}
	}

	return ta_;
}