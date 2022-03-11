#pragma once

#define POINT_LIGHT_DATA { { u8"vec3f", u8"position" }, { u8"vec3f", u8"color" }, {u8"float32", u8"intensity"} }
#define LIGHTING_DATA { {u8"uint32", u8"pointLightsLength"},  {u8"PointLightData[1024]", u8"pointLights"} }
#define INSTANCE_DATA { { u8"matrix3x4f", u8"transform" }, { u8"uint32", u8"vertexBufferOffset" }, { u8"uint32", u8"indexBufferOffset" }, { u8"uint32", u8"shaderGroupIndex" }, {u8"uint32", u8"padding" } }
#define CAMERA_DATA { { u8"matrix4f", u8"view" }, { u8"matrix4f", u8"proj" }, { u8"matrix4f", u8"viewInverse" }, { u8"matrix4f", u8"projInverse" }, { u8"matrix4f", u8"vp" }, { u8"matrix4f", u8"vpInverse" }, { u8"float32", u8"near" }, { u8"float32", u8"far" }, { u8"u16vec2", u8"extent"} }
#define GLOBAL_DATA { { u8"uint32[4]", u8"blah" } }