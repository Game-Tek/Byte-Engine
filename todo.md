- Test render frame with renderer but no elements
- Test asset path
- Support no audio endpoint
updat- Add support for perprimitiveEXT qualifier in shader outputs
	- remove them from mesh shader header
- Add a visibility pass that scans pending material evaluation materials and initiates their load
- Add a visibility pass that scans pending material textures and initiates their load
- Transition material and texture resource states from pending/loading to loaded after GPU work completes
- Retry renderable mesh messages when transfer buffer space is exhausted instead of panicking
- Replace prepare_transfers tuple return with a named result type
