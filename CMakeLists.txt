cmake_minimum_required(VERSION 3.20)

project(ByteEngine LANGUAGES CXX VERSION 1.0.0)

# requirements
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include_directories("${PROJECT_SOURCE_DIR}/src")
include_directories("${PROJECT_SOURCE_DIR}/ext")

add_library(ByteEngine STATIC "${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/EntryPoint.h" "${PROJECT_SOURCE_DIR}/src/ByteEngine/Physics/PhysicsWorld.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Render/RenderOrchestrator.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/AllocatorReferences.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/PoolAllocator.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/StackAllocator.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/SystemAllocator.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/Templates/GameApplication.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Debug/FunctionTimer.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Game/ApplicationManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Object.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Render/RendererAllocator.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Render/RenderSystem.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Game/World.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Light.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Render/StaticMeshRenderGroup.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Render/UIManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Resources/AnimationResourceManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Resources/AudioResourceManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Resources/FontResourceManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Resources/PipelineCacheResourceManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Resources/ResourceManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Resources/StaticMeshResourceManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Resources/TextureResourceManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Sound/AudioSystem.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Utility/Shapes/Cone.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Utility/Shapes/ConeWithFalloff.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/InputManager.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Debug/Logger.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/Clock.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/Application.cpp"
"${PROJECT_SOURCE_DIR}/ext/stb image/IMAGE_IMPLEMENTATION.cpp")

# set_target_properties(mylib PROPERTIES VERSION ${PROJECT_VERSION})

set_target_properties(ByteEngine PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED YES CXX_EXTENSIONS NO)
set_target_properties(ByteEngine PROPERTIES PUBLIC_HEADER "${PROJECT_SOURCE_DIR}/ByteEngine.h")
target_compile_options(ByteEngine PRIVATE $<$<OR:$<CXX_COMPILER_ID:Clang>,$<CXX_COMPILER_ID:AppleClang>,$<CXX_COMPILER_ID:GNU>>:-Wall>$<$<CXX_COMPILER_ID:MSVC>:/W4>)

if (CMAKE_BUILD_TYPE MATCHES Debug)
	target_compile_definitions(ByteEngine PUBLIC BE_DEBUG=1)
endif()

if (WIN32)
	target_compile_definitions(ByteEngine PUBLIC BE_PLATFORM_WIN=1)
	target_compile_definitions(ByteEngine PUBLIC NOMINMAX)
	target_compile_definitions(ByteEngine PUBLIC VC_EXTRALEAN)
	target_compile_definitions(ByteEngine PUBLIC WIN32_LEAN_AND_MEAN)
endif()