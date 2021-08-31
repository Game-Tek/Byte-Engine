cmake_minimum_required(VERSION 3.20)
include(FetchContent)

project(ByteEngine LANGUAGES CXX VERSION 1.0.0)

# assimp download
FetchContent_Declare(assimp GIT_REPOSITORY "https://github.com/assimp/assimp" GIT_TAG "ceced70df0fb33e9eaf483dac8851cdb834b6404")

option(ASSIMP_BUILD_TESTS OFF)
option(ASSIMP_BUILD_ZLIB ON)
option(BUILD_SHARED_LIBS OFF)
option(ASSIMP_BUILD_ASSIMP_TOOLS OFF)
option(ASSIMP_INSTALL_PDB OFF)
option(ASSIMP_BUILD_GLTF_IMPORTER OFF)
option(ASSIMP_BUILD_GLTF_EXPORTER OFF)

FetchContent_MakeAvailable(assimp)
# assimp download

# stb
FetchContent_Declare(stb GIT_REPOSITORY "https://github.com/nothings/stb" GIT_TAG "c0c982601f40183e74d84a61237e968dca08380e")
FetchContent_MakeAvailable(stb)
# stb

# GTSL
FetchContent_Declare(GTSL GIT_REPOSITORY "https://github.com/Game-Tek/Game-Tek-Standard-Library" GIT_TAG "a780a0ca84480870e3e27844144d0bcdb7bc7091")
FetchContent_MakeAvailable(GTSL)
# GTSL


# ByteEngine options
set(BE_GAPI "vulkan")
# ByteEngine options

add_library(ByteEngine STATIC "${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/EntryPoint.h"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Physics/PhysicsWorld.cpp"
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
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/Application.cpp")

set_target_properties(ByteEngine PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED YES CXX_EXTENSIONS NO)
set_target_properties(ByteEngine PROPERTIES PUBLIC_HEADER "${PROJECT_SOURCE_DIR}/src/ByteEngine.h")
target_compile_options(ByteEngine PRIVATE $<$<OR:$<CXX_COMPILER_ID:Clang>,$<CXX_COMPILER_ID:AppleClang>,$<CXX_COMPILER_ID:GNU>>:-Wall>$<$<CXX_COMPILER_ID:MSVC>:/W4>)
target_compile_features(ByteEngine PUBLIC cxx_std_20)

target_include_directories(ByteEngine PUBLIC "${PROJECT_SOURCE_DIR}/src")
target_include_directories(ByteEngine PUBLIC "${PROJECT_SOURCE_DIR}/ext")

# assimp
target_include_directories(ByteEngine PUBLIC "${assimp_SOURCE_DIR}/include")
target_include_directories(ByteEngine PUBLIC "${assimp_BINARY_DIR}/include")
target_link_libraries(ByteEngine INTERFACE "${assimp_BINARY_DIR}/lib/assimp-vc142-mtd.lib")
target_link_libraries(ByteEngine INTERFACE "${assimp_BINARY_DIR}/contrib/zlib/zlibstaticd.lib")
# assimp

# stb
target_include_directories(ByteEngine PUBLIC "${stb_SOURCE_DIR}")
# stb

# GTSL
target_include_directories(ByteEngine PUBLIC "${gtsl_SOURCE_DIR}")
target_link_libraries(ByteEngine INTERFACE GTSL)
# GTSL

if (CMAKE_BUILD_TYPE MATCHES Debug)
	target_compile_definitions(ByteEngine PUBLIC BE_DEBUG=1)
else()
	target_compile_definitions(ByteEngine PUBLIC BE_DEBUG=0)
endif()

if (WIN32)
	if (BE_GAPI STREQUAL "vulkan")
		target_include_directories(ByteEngine PUBLIC "$ENV{VULKAN_SDK}/Include")
		target_link_libraries(ByteEngine INTERFACE "$ENV{VULKAN_SDK}/Lib/shaderc_shared.lib")
		target_compile_definitions(ByteEngine PUBLIC BE_VULKAN=1)
		target_compile_definitions(ByteEngine PUBLIC BE_DX12=0)
	elseif(BE_GAPI STREQUAL "dx12")	
		target_compile_definitions(ByteEngine PUBLIC BE_VULKAN=0)
		target_compile_definitions(ByteEngine PUBLIC BE_DX12=1)
	endif()	
	
	target_link_libraries(ByteEngine INTERFACE Hid.lib)
	target_link_libraries(ByteEngine INTERFACE XInput.lib)
	
	target_compile_definitions(ByteEngine PUBLIC BE_PLATFORM_WIN=1)
	target_compile_definitions(ByteEngine PUBLIC NOMINMAX)
	target_compile_definitions(ByteEngine PUBLIC VC_EXTRALEAN)
	target_compile_definitions(ByteEngine PUBLIC WIN32_LEAN_AND_MEAN)
endif()

install(TARGETS ByteEngine DESTINATION lib)
install(FILES "${PROJECT_SOURCE_DIR}/src/ByteEngine.h" DESTINATION include)