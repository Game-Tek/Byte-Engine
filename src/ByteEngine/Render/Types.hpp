#pragma once

#define POINT_LIGHT_DATA { { u8"vec3f", u8"position" }, { u8"vec3f", u8"color" }, {u8"float32", u8"intensity"}, {u8"float32", u8"radius"} }
#define LIGHTING_DATA { { u8"uint32", u8"lightCount" }, { u8"uint32[8]", u8"lights" }, { u8"uint32", u8"pointLightsLength" },  { u8"PointLightData[1024]", u8"pointLights" } }
#define INSTANCE_DATA { { u8"matrix3x4f", u8"transform" }, { u8"uint32", u8"vertexBufferOffset" }, { u8"uint32", u8"indexBufferOffset" }, { u8"uint32", u8"shaderGroupIndex" }, {u8"uint32", u8"padding" } }
#define VIEW_DATA { { u8"matrix4f", u8"view" }, { u8"matrix4f", u8"proj" }, { u8"matrix4f", u8"viewInverse" }, { u8"matrix4f", u8"projInverse" }, { u8"matrix4f", u8"vp" }, { u8"matrix4f", u8"vpInverse" }, { u8"vec4f", u8"position" },{ u8"float32", u8"near" }, { u8"float32", u8"far" }, { u8"u16vec2", u8"extent"} }
#define CAMERA_DATA { { u8"ViewData[3]", u8"viewHistory" } }
#define GLOBAL_DATA { { u8"uint32", u8"frameIndex" }, { u8"float32", u8"elapsedTime" }, { u8"float32", u8"deltaTime" }, { u8"uint32", u8"framePipelineDepth" }, { u8"uint32[4]", u8"random" }, { u8"TextureReference[8]", u8"blueNoise2D" } }

#define UI_RES { { u8"float32", u8"bestDistance" }, { u8"vec2f", u8"a" }, { u8"vec2f", u8"b" } }

#define UI_INSTANCE_DATA { { u8"matrix3x4f", u8"transform" }, { u8"vec4f", u8"color" }, { u8"float32", u8"roundness" }, { u8"uint32[2]", u8"derivedTypeIndex" } }
#define UI_TEXT_DATA { { u8"uint32", u8"fontIndex" }, { u8"uint32[128]", u8"chars" } }
#define UI_LINEAR_SEGMENT { { u8"vec2f[2]", u8"segments" } }
#define UI_QUADRATIC_SEGMENT { { u8"vec2f[3]", u8"segments" } }
#define UI_GLYPH_CONTOUR_DATA { { u8"uint32", u8"linearSegmentCount" }, { u8"uint32", u8"quadraticSegmentCount" }, { u8"LinearSegment[128]", u8"linearSegments" }, { u8"QuadraticSegment[128]", u8"quadraticSegments" } }
#define UI_GLYPH_DATA { { u8"uint32", u8"contourCount" }, { u8"GlyphContourData[4]", u8"contours" } }
#define UI_FONT_DATA { { u8"GlyphData*[128]", u8"glyphs" } }
#define UI_DATA { { u8"matrix4f", u8"projection" }, { u8"FontData*[4]", u8"fontData" }, { u8"TextData[16]", u8"textData" } }

#define INDIRECT_DISPATCH_COMMAND_DATA { { u8"uint32", u8"x" }, { u8"uint32", u8"y" }, { u8"uint32", u8"z" } }

#define TRACE_RAY_PARAMETER_DATA { { u8"uint64", u8"accelerationStructure" }, { u8"uint32", u8"rayFlags" }, { u8"uint32", u8"recordOffset"}, { u8"uint32", u8"recordStride"}, { u8"uint32", u8"missIndex"}, { u8"float32", u8"tMin"}, { u8"float32", u8"tMax"} }

#define FORWARD_RENDERPASS_DATA { { u8"ImageReference", u8"Albedo" }, { u8"ImageReference", u8"Normal" }, { u8"ImageReference", u8"Roughness" }, { u8"ImageReference", u8"Depth" } }
#define RT_RENDERPASS_DATA { { u8"TextureReference", u8"Albedo" }, { u8"TextureReference", u8"Depth" }, { u8"ImageReference", u8"Shadow" } }
#define LIGHTING_RENDERPASS_DATA { { u8"TextureReference", u8"Albedo" }, { u8"TextureReference", u8"Normal" }, { u8"TextureReference", u8"Roughness" }, { u8"TextureReference", u8"Shadow" }, { u8"TextureReference", u8"Depth" }, { u8"ImageReference", u8"Lighting" } }