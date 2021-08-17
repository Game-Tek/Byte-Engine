cmake_minimum_required(VERSION 3.20)
include(FetchContent)

project(ByteEngine LANGUAGES CXX VERSION 1.0.0)

# assimp download
FetchContent_Declare(assimp GIT_REPOSITORY "https://github.com/assimp/assimp" GIT_TAG "fcc7ad52a1838fefd381782b8102c83fe09525ce")

set(ASSIMP_BUILD_TESTS OFF)
set(ASSIMP_BUILD_SHARED_LIBS OFF)
set(ASSIMP_ASSIMP_INSTALL_PDB OFF)

# maybe add assimp subdirectory??

FetchContent_MakeAvailable(assimp)
# assimp download

# GTSL
FetchContent_Declare(GTSL GIT_REPOSITORY "https://github.com/Game-Tek/Game-Tek-Standard-Library" GIT_TAG "970aaa8b64b4f573108cfa4939613008e445d2a4")
FetchContent_MakeAvailable(GTSL)
# GTSL

# requirements
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
# requirements

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
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/Application.cpp"
)

target_include_directories(ByteEngine PUBLIC "${PROJECT_SOURCE_DIR}/src")
target_include_directories(ByteEngine PUBLIC "${PROJECT_SOURCE_DIR}/ext")

# assimp
target_include_directories(ByteEngine PUBLIC "${assimp_SOURCE_DIR}/include")
target_include_directories(ByteEngine PUBLIC "${assimp_BINARY_DIR}/include")
target_link_libraries(ByteEngine INTERFACE ASSIMP)
# assimp

# GTSL
target_include_directories(ByteEngine PUBLIC "${gtsl_SOURCE_DIR}")
target_link_libraries(ByteEngine INTERFACE GTSL)
# GTSL

set_target_properties(ByteEngine PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED YES CXX_EXTENSIONS NO)
set_target_properties(ByteEngine PROPERTIES PUBLIC_HEADER "${PROJECT_SOURCE_DIR}/src/ByteEngine.h")
target_compile_options(ByteEngine PRIVATE $<$<OR:$<CXX_COMPILER_ID:Clang>,$<CXX_COMPILER_ID:AppleClang>,$<CXX_COMPILER_ID:GNU>>:-Wall>$<$<CXX_COMPILER_ID:MSVC>:/W4>)

if (CMAKE_BUILD_TYPE MATCHES Debug)
	target_compile_definitions(ByteEngine PUBLIC BE_DEBUG=1)
else()
	target_compile_definitions(ByteEngine PUBLIC BE_DEBUG=0)
endif()

if (WIN32)
	if (BE_GAPI STREQUAL "vulkan")
		include_directories("$ENV{VULKAN_SDK}/Include")
		target_compile_definitions(ByteEngine PUBLIC BE_VULKAN=1)
		target_compile_definitions(ByteEngine PUBLIC BE_DX12=0)
	elseif(BE_GAPI STREQUAL "dx12")	
		target_compile_definitions(ByteEngine PUBLIC BE_VULKAN=0)
		target_compile_definitions(ByteEngine PUBLIC BE_DX12=1)
	endif()	
	
	target_link_libraries(ByteEngine INTERFACE Hid.lib)
	target_link_libraries(ByteEngine INTERFACE XInput.lib)
	target_link_libraries(ByteEngine INTERFACE shaderc_shared.lib)
	
	target_compile_definitions(ByteEngine PUBLIC BE_PLATFORM_WIN=1)
	target_compile_definitions(ByteEngine PUBLIC NOMINMAX)
	target_compile_definitions(ByteEngine PUBLIC VC_EXTRALEAN)
	target_compile_definitions(ByteEngine PUBLIC WIN32_LEAN_AND_MEAN)
endif()

install(TARGETS ByteEngine DESTINATION lib)
install(FILES "${PROJECT_SOURCE_DIR}/src/ByteEngine.h" DESTINATION include)