cmake_minimum_required(VERSION 3.24)

if(UNIX)
	add_compile_options(-mavx2)
endif()

# ByteEngine options
option(BE_GAPI "Graphics API to Build with (vulkan or dx12)" "vulkan")
option(BE_CSHARP_SCRIPTING "Build CSharp Wrapper" ON)
option(BE_DEBUG "Enable Debug" ON)
option(BE_BUILD_TESTS "Build Tests" OFF)
option(USE_XREPO_FOR_DEPS "Use XRepo to pull dependencies" OFF)
# ByteEngine options

if(${USE_XREPO_FOR_DEPS})
	if(NOT EXISTS "${CMAKE_BINARY_DIR}/xrepo.cmake")
		message(STATUS "Downloading xrepo.cmake from https://github.com/xmake-io/xrepo-cmake/")
		file(DOWNLOAD "https://raw.githubusercontent.com/xmake-io/xrepo-cmake/main/xrepo.cmake"
                  "${CMAKE_BINARY_DIR}/xrepo.cmake"
                  TLS_VERIFY ON)
	endif()

	include(${CMAKE_BINARY_DIR}/xrepo.cmake)
endif()

project(ByteEngine LANGUAGES CXX VERSION 1.0.0)

include(FetchContent)

# GTSL
FetchContent_Declare(GTSL GIT_REPOSITORY "https://github.com/Game-Tek/Game-Tek-Standard-Library" GIT_TAG "889283d1d17aaa0b7478a4cfa68e8781d2dff28e")
FetchContent_MakeAvailable(GTSL)
# GTSL

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
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Render/StaticMeshSystem.cpp"
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
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/ScriptingSystem.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/Clock.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Application/Application.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Resources/BC7.cpp"
"${PROJECT_SOURCE_DIR}/src/ByteEngine/Render/WorldRenderPipeline.cpp")

set_target_properties(ByteEngine PROPERTIES CXX_STANDARD 20 CXX_STANDARD_REQUIRED YES CXX_EXTENSIONS NO)
set_target_properties(ByteEngine PROPERTIES PUBLIC_HEADER "${PROJECT_SOURCE_DIR}/src/ByteEngine.h")
target_compile_options(ByteEngine PRIVATE $<$<OR:$<CXX_COMPILER_ID:Clang>,$<CXX_COMPILER_ID:AppleClang>,$<CXX_COMPILER_ID:GNU>>:-Wall>$<$<CXX_COMPILER_ID:MSVC>:/W4>)
target_compile_features(ByteEngine PUBLIC cxx_std_20)

target_include_directories(ByteEngine PUBLIC "${PROJECT_SOURCE_DIR}/src")
target_include_directories(ByteEngine PUBLIC "${PROJECT_SOURCE_DIR}/ext")

# assimp
if(${USE_HUNTER})
	xrepo_package("assimp 5.2.5")
endif()

find_package(assimp CONFIG REQUIRED)

set(BUILD_SHARED_LIBS OFF CACHE INTERNAL "" FORCE)
set(ASSIMP_BUILD_TESTS OFF CACHE INTERNAL "" FORCE)
set(ASSIMP_BUILD_ZLIB ON CACHE INTERNAL "" FORCE)
set(ASSIMP_BUILD_ASSIMP_TOOLS OFF CACHE INTERNAL "" FORCE)
set(ASSIMP_INSTALL_PDB OFF CACHE INTERNAL "" FORCE)
set(ASSIMP_BUILD_ALL_EXPORTERS_BY_DEFAULT OFF CACHE INTERNAL "" FORCE)
set(ASSIMP_BUILD_ALL_IMPORTERS_BY_DEFAULT OFF CACHE INTERNAL "" FORCE)
set(ASSIMP_BUILD_OBJ_IMPORTER ON CACHE INTERNAL "" FORCE)
set(ASSIMP_BUILD_FBX_IMPORTER ON CACHE INTERNAL "" FORCE)

target_link_libraries(ByteEngine INTERFACE assimp::assimp)
target_include_directories(ByteEngine PUBLIC assimp::assimp)
target_include_directories(ByteEngine INTERFACE assimp_INCLUDE_DIRS)
# assimp

if(${USE_XREPO_FOR_DEPS})
	xrepo_package("stb 2023.01.30")
	xrepo_target_packages(ByteEngine PUBLIC stb)
else()
	find_package(stb REQUIRED)
	target_include_directories(ByteEngine PUBLIC stb::stb)
endif()

# GTSL
target_include_directories(ByteEngine PUBLIC "${gtsl_SOURCE_DIR}")
target_link_libraries(ByteEngine INTERFACE GTSL)
# GTSL

enable_testing()
if(${BE_BUILD_TESTS})
	add_subdirectory(tests)
endif()

if (CMAKE_BUILD_TYPE MATCHES Debug)
	target_compile_definitions(ByteEngine PUBLIC BE_DEBUG=1)
else()
	target_compile_definitions(ByteEngine PUBLIC BE_DEBUG=0)
endif()

if (WIN32)
	target_compile_definitions(ByteEngine PUBLIC BE_PLATFORM_WINDOWS=1)
	target_compile_definitions(ByteEngine PUBLIC BE_PLATFORM_LINUX=0)

	if(BE_CSHARP_SCRIPTING)
		target_include_directories(ByteEngine PUBLIC "C:/Program Files/Mono/include/mono-2.0")
		target_link_libraries(ByteEngine PRIVATE "C:/Program Files/Mono/lib/mono-2.0-sgen.lib")
	endif()

	if (BE_GAPI STREQUAL "vulkan")
		find_package(Vulkan REQUIRED COMPONENTS shaderc_combined)

		target_include_directories(ByteEngine PUBLIC Vulkan::Vulkan)
		target_link_libraries(ByteEngine PUBLIC Vulkan::Vulkan)
		target_link_libraries(ByteEngine PUBLIC Vulkan::shaderc_combined)

		target_compile_definitions(ByteEngine PUBLIC BE_VULKAN=1)
		target_compile_definitions(ByteEngine PUBLIC BE_DX12=0)
	elseif(BE_GAPI STREQUAL "dx12")	
		target_link_libraries(ByteEngine INTERFACE d3d12.lib)

		target_compile_definitions(ByteEngine PUBLIC BE_VULKAN=0)
		target_compile_definitions(ByteEngine PUBLIC BE_DX12=1)
	endif()	
	
	target_link_libraries(ByteEngine INTERFACE Hid.lib)
	target_link_libraries(ByteEngine INTERFACE XInput.lib)
	
	target_compile_definitions(ByteEngine PUBLIC NOMINMAX)
	target_compile_definitions(ByteEngine PUBLIC VC_EXTRALEAN)
	target_compile_definitions(ByteEngine PUBLIC WIN32_LEAN_AND_MEAN)
endif()

if (UNIX)
	target_compile_definitions(ByteEngine PUBLIC BE_PLATFORM_WINDOWS=0)
	target_compile_definitions(ByteEngine PUBLIC BE_PLATFORM_LINUX=1)

	if(BE_CSHARP_SCRIPTING)
		target_include_directories(ByteEngine PUBLIC "/usr/include/mono-2.0")
		target_link_libraries(ByteEngine PRIVATE "/lib/libmono-2.0.a")
	endif()

	find_package(ALSA REQUIRED)
	target_include_directories(ByteEngine PUBLIC ALSA::ALSA)
	target_link_libraries(ByteEngine PUBLIC ALSA::ALSA)

	if (BE_GAPI STREQUAL "vulkan")
		find_package(Vulkan REQUIRED COMPONENTS shaderc_combined)

		target_include_directories(ByteEngine PUBLIC Vulkan::Vulkan)
		target_link_libraries(ByteEngine PUBLIC Vulkan::Vulkan)
		target_link_libraries(ByteEngine PUBLIC Vulkan::shaderc_combined)

		target_compile_definitions(ByteEngine PUBLIC BE_VULKAN=1)
		target_compile_definitions(ByteEngine PUBLIC BE_DX12=0)
	elseif(BE_GAPI STREQUAL "dx12")
		target_compile_definitions(ByteEngine PUBLIC BE_VULKAN=0)
		target_compile_definitions(ByteEngine PUBLIC BE_DX12=0)
	endif()	
endif()