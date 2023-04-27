project('ByteEngine', 'cpp')

src = [
	'src/ByteEngine/Application/EntryPoint.h',
	'src/ByteEngine/Physics/PhysicsWorld.cpp',
	'src/ByteEngine/Render/RenderOrchestrator.cpp',
	'src/ByteEngine/Application/AllocatorReferences.cpp',
	'src/ByteEngine/Application/PoolAllocator.cpp',
	'src/ByteEngine/Application/StackAllocator.cpp',
	'src/ByteEngine/Application/SystemAllocator.cpp',
	'src/ByteEngine/Application/Templates/GameApplication.cpp',
	'src/ByteEngine/Debug/FunctionTimer.cpp',
	'src/ByteEngine/Game/ApplicationManager.cpp',
	'src/ByteEngine/Object.cpp',
	'src/ByteEngine/Render/RendererAllocator.cpp',
	'src/ByteEngine/Render/RenderSystem.cpp',
	'src/ByteEngine/Game/World.cpp',
	'src/ByteEngine/Light.cpp',
	'src/ByteEngine/Render/StaticMeshSystem.cpp',
	'src/ByteEngine/Render/UIManager.cpp',
	'src/ByteEngine/Resources/AnimationResourceManager.cpp',
	'src/ByteEngine/Resources/AudioResourceManager.cpp',
	'src/ByteEngine/Resources/FontResourceManager.cpp',
	'src/ByteEngine/Resources/PipelineCacheResourceManager.cpp',
	'src/ByteEngine/Resources/ResourceManager.cpp',
	'src/ByteEngine/Resources/StaticMeshResourceManager.cpp',
	'src/ByteEngine/Resources/TextureResourceManager.cpp',
	'src/ByteEngine/Sound/AudioSystem.cpp',
	'src/ByteEngine/Utility/Shapes/Cone.cpp',
	'src/ByteEngine/Utility/Shapes/ConeWithFalloff.cpp',
	'src/ByteEngine/Application/InputManager.cpp',
	'src/ByteEngine/Debug/Logger.cpp',
	'src/ByteEngine/Application/ScriptingSystem.cpp',
	'src/ByteEngine/Application/Clock.cpp',
	'src/ByteEngine/Application/Application.cpp',
	'src/ByteEngine/Resources/BC7.cpp',
	'src/ByteEngine/Render/WorldRenderPipeline.cpp',
]

assimp = dependency('assimp', version: '>=5.0.0')
stb = dependency('stb')
gtsl = dependency('GTSL')
mono = dependency('mono-2')
monosgen = dependency('monosgen-2')
vulkan = dependency('vulkan')
shaderc = dependency('shaderc')
alsa = dependency('alsa')

args = [
	'-mavx',
	'-DBE_DEBUG=1', '-DBE_PLATFORM_WINDOWS=0', '-DBE_PLATFORM_LINUX=1', '-DBE_VULKAN=1', '-DBE_DX12=0',
	'-Db_sanitize=address',
]

ByteEngine = library('ByteEngine', src, dependencies: [assimp, stb, gtsl, mono, monosgen, vulkan, shaderc, alsa], include_directories: include_directories('src'), cpp_args: args)

test_src = ['tests/TestApplication.cpp']

test = executable('TestByteEngine', test_src, link_with: [ByteEngine], include_directories: include_directories('src'))

test('TestByteEngine', test)